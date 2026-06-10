//! End-to-end coverage for the composed application route table.
//!
//! `support::test_app` builds its `App` from
//! `atomic_server::app::configure_app` — the same function `main.rs` uses —
//! so this suite pins the full composition in one place: a public route
//! (`/health`) answers without auth, the bearer-gated `/api` scope accepts
//! a valid token and rejects a missing one, and the `/mcp` scope sits
//! behind `McpAuth` (401 with `WWW-Authenticate` when unauthenticated).
//! Every other e2e suite exercises the same wiring implicitly; this one
//! asserts the cross-scope layout directly so a regression in the
//! composition itself (e.g. a route slipping inside the wrong auth scope)
//! fails loudly rather than as a confusing downstream test error.

mod support;

use std::sync::Arc;

use actix_web::dev::ServiceRequest;
use actix_web::middleware::{from_fn, Next};
use actix_web::test as actix_test;
use actix_web::{App, HttpMessage};
use atomic_core::DatabaseManager;
use atomic_server::app::configure_app;
use atomic_server::db_extractor::RequestDatabaseManager;
use serde_json::{json, Value};
use support::{mcp_transport_for, test_app, Backend, TestCtx};

#[actix_web::test]
async fn full_composition_sqlite() {
    run_full_composition(Backend::Sqlite).await;
}

#[actix_web::test]
async fn full_composition_postgres() {
    if std::env::var("ATOMIC_TEST_DATABASE_URL").is_err() {
        eprintln!("full_composition_postgres: skipping (ATOMIC_TEST_DATABASE_URL not set)");
        return;
    }
    run_full_composition(Backend::Postgres).await;
}

async fn run_full_composition(backend: Backend) {
    let Some(ctx) = TestCtx::new(backend).await else {
        return;
    };
    let app = actix_test::init_service(test_app(&ctx)).await;

    // Public route: /health answers without any credentials.
    let req = actix_test::TestRequest::get().uri("/health").to_request();
    let resp = actix_test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200, "/health must be public");
    let body: Value = actix_test::read_body_json(resp).await;
    assert_eq!(body["status"], "ok");
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));

    // Authenticated route: the /api scope admits a valid bearer token...
    let req = actix_test::TestRequest::get()
        .uri("/api/atoms")
        .insert_header(ctx.auth_header())
        .to_request();
    let resp = actix_test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200, "/api must work with a valid token");

    // ...and rejects the same request without one. BearerAuth surfaces the
    // rejection as an actix error (rendered as a 401 over HTTP), so probe
    // the error's status rather than a ServiceResponse.
    let req = actix_test::TestRequest::get()
        .uri("/api/atoms")
        .to_request();
    let err = match actix_test::try_call_service(&app, req).await {
        Ok(resp) => panic!("/api must reject missing tokens, got {}", resp.status()),
        Err(err) => err,
    };
    assert_eq!(
        err.as_response_error().error_response().status(),
        401,
        "/api rejection must render as 401"
    );

    // MCP scope: McpAuth gates /mcp and advertises OAuth discovery on 401.
    let req = actix_test::TestRequest::post().uri("/mcp").to_request();
    let resp = actix_test::call_service(&app, req).await;
    assert_eq!(resp.status(), 401, "/mcp must reject missing tokens");
    let www_authenticate = resp
        .headers()
        .get("WWW-Authenticate")
        .and_then(|v| v.to_str().ok())
        .expect("MCP 401 must carry WWW-Authenticate");
    assert!(
        www_authenticate.contains("resource_metadata="),
        "MCP 401 must point at OAuth discovery, got: {www_authenticate}"
    );
}

// ==================== Injected database-manager resolution ====================

#[actix_web::test]
async fn injected_manager_resolution_sqlite() {
    run_injected_manager_resolution(Backend::Sqlite).await;
}

#[actix_web::test]
async fn injected_manager_resolution_postgres() {
    if std::env::var("ATOMIC_TEST_DATABASE_URL").is_err() {
        eprintln!(
            "injected_manager_resolution_postgres: skipping (ATOMIC_TEST_DATABASE_URL not set)"
        );
        return;
    }
    run_injected_manager_resolution(Backend::Postgres).await;
}

/// `Db` resolution contract for embedders: a layer composing
/// `configure_app` under its own middleware can install a
/// [`RequestDatabaseManager`] per request, and the `Db` extractor resolves
/// against that manager instead of `AppState`'s; with no such middleware
/// (the standalone server), it falls back to `AppState`.
///
/// Two managers over the *same* storage with different active databases
/// make the override observable through the standard routes. (Two fully
/// disjoint `TestCtx`s would be starker, but the Postgres backend shares
/// one physical database per process — constructing a second context
/// truncates the first — so the active-database split is the portable way
/// to tell the managers apart.)
async fn run_injected_manager_resolution(backend: Backend) {
    let is_postgres = matches!(backend, Backend::Postgres);
    let Some(ctx) = TestCtx::new(backend).await else {
        return;
    };
    let app = actix_test::init_service(test_app(&ctx)).await;

    // Seed one atom in the default database and one in a second database.
    let alpha_id = post_atom(&app, &ctx, None, "alpha — lives in the default database").await;
    let probe_db = create_database(&app, &ctx, "resolution-probe").await;
    let beta_id = post_atom(
        &app,
        &ctx,
        Some(&probe_db),
        "beta — lives in the probe database",
    )
    .await;

    // No middleware installed → the extractor falls back to AppState, whose
    // active database is the default. The rest of the suite relies on this
    // implicitly; assert it once, explicitly, next to the override test.
    let ids = list_atom_ids(&app, &ctx, None).await;
    assert!(
        ids.contains(&alpha_id) && !ids.contains(&beta_id),
        "without middleware, Db must resolve via AppState's active database"
    );

    // A second manager over the same storage, with the probe database active.
    let manager = if is_postgres {
        let url = std::env::var("ATOMIC_TEST_DATABASE_URL").expect("postgres url");
        Arc::new(
            DatabaseManager::new_postgres(ctx.data_dir(), &url)
                .await
                .expect("open second postgres manager"),
        )
    } else {
        Arc::new(DatabaseManager::new(ctx.data_dir()).expect("open second sqlite manager"))
    };
    manager
        .set_active(&probe_db)
        .await
        .expect("activate probe db");

    // The same route table, wrapped in a middleware that installs the second
    // manager — the composition shape an embedder would use.
    let injected = RequestDatabaseManager(Arc::clone(&manager));
    let app = actix_test::init_service(
        App::new()
            .wrap(from_fn(move |req: ServiceRequest, next: Next<_>| {
                let injected = injected.clone();
                async move {
                    req.extensions_mut().insert(injected);
                    next.call(req).await
                }
            }))
            .configure(configure_app(ctx.state.clone(), mcp_transport_for(&ctx))),
    )
    .await;

    // The extractor now resolves against the injected manager, whose active
    // database is the probe — not AppState's default.
    let ids = list_atom_ids(&app, &ctx, None).await;
    assert!(
        ids.contains(&beta_id) && !ids.contains(&alpha_id),
        "with middleware, Db must resolve via the injected manager's active database"
    );

    // The per-request selection rules (X-Atomic-Database header) apply to
    // the injected manager too — the override swaps the manager, not the
    // resolution logic.
    let default_db = ctx.state.manager.active_id().expect("default db id");
    let ids = list_atom_ids(&app, &ctx, Some(&default_db)).await;
    assert!(
        ids.contains(&alpha_id) && !ids.contains(&beta_id),
        "X-Atomic-Database must select within the injected manager"
    );
}

/// POST /api/atoms, optionally into a specific database, returning the id.
async fn post_atom<S, B>(app: &S, ctx: &TestCtx, db_id: Option<&str>, content: &str) -> String
where
    S: actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse<B>,
        Error = actix_web::Error,
    >,
    B: actix_web::body::MessageBody,
{
    let mut req = actix_test::TestRequest::post()
        .uri("/api/atoms")
        .insert_header(ctx.auth_header())
        .set_json(json!({ "content": content }));
    if let Some(db_id) = db_id {
        req = req.insert_header(ctx.db_header(db_id));
    }
    let resp = actix_test::call_service(app, req.to_request()).await;
    assert_eq!(resp.status(), 201, "POST /api/atoms should return 201");
    let body: Value = actix_test::read_body_json(resp).await;
    body["id"]
        .as_str()
        .expect("created atom has id")
        .to_string()
}

/// POST /api/databases, returning the new database's id.
async fn create_database<S, B>(app: &S, ctx: &TestCtx, name: &str) -> String
where
    S: actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse<B>,
        Error = actix_web::Error,
    >,
    B: actix_web::body::MessageBody,
{
    let req = actix_test::TestRequest::post()
        .uri("/api/databases")
        .insert_header(ctx.auth_header())
        .set_json(json!({ "name": name }))
        .to_request();
    let resp = actix_test::call_service(app, req).await;
    assert_eq!(resp.status(), 201, "POST /api/databases should return 201");
    let body: Value = actix_test::read_body_json(resp).await;
    body["id"]
        .as_str()
        .expect("created database has id")
        .to_string()
}

/// GET /api/atoms (optionally scoped via the X-Atomic-Database header),
/// returning the set of atom ids in the listing.
async fn list_atom_ids<S, B>(
    app: &S,
    ctx: &TestCtx,
    db_id: Option<&str>,
) -> std::collections::HashSet<String>
where
    S: actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse<B>,
        Error = actix_web::Error,
    >,
    B: actix_web::body::MessageBody,
{
    let mut req = actix_test::TestRequest::get()
        .uri("/api/atoms")
        .insert_header(ctx.auth_header());
    if let Some(db_id) = db_id {
        req = req.insert_header(ctx.db_header(db_id));
    }
    let resp = actix_test::call_service(app, req.to_request()).await;
    assert_eq!(resp.status(), 200, "GET /api/atoms should succeed");
    let body: Value = actix_test::read_body_json(resp).await;
    body["atoms"]
        .as_array()
        .expect("listing has atoms array")
        .iter()
        .filter_map(|a| a["id"].as_str().map(str::to_string))
        .collect()
}
