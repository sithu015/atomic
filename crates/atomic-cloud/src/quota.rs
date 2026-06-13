//! Plan-tier resource enforcement on the data plane (plan: "Observability,
//! quotas, billing" → "Quotas" → "Enforcement points").
//!
//! Cloud cannot touch atomic-server's route handlers (the one-way dependency
//! rule), so resource limits are enforced the way slice 2 marked dispatch
//! hints and slice 4 capped chat streams: a cloud middleware on `api_scope`
//! that path-matches the relevant mutating routes and checks the plan's
//! limit **before** delegating, returning the structured error itself. The
//! handler is never reached on a quota hit, so no atom is created and no
//! ledger work is enqueued.
//!
//! # What's enforced, and how the count is read
//!
//! | Route                                   | Limit       | Count source                        |
//! |-----------------------------------------|-------------|-------------------------------------|
//! | `POST /api/atoms`                       | `atom_limit`| `AtomicCore::count_atoms()` (live)  |
//! | `POST /api/atoms/bulk`                  | `atom_limit`| live count + the request's batch    |
//! | `POST /api/databases`                   | `kb_limit`  | `DatabaseManager::list_databases()` |
//!
//! Both counts are read **live** from the tenant database at enforcement
//! time — cheap, single-statement, strongly consistent. There is no stored
//! atom/KB counter to drift (the `quota_usage` table is for metrics that
//! can't be counted cheaply live; see [`crate::plans`]). A `NULL` limit
//! means unlimited and the guard passes the request straight through —
//! the count is never even read.
//!
//! # The bulk batch delta
//!
//! A bulk create can push the tenant over the limit with a single request,
//! so the guard accounts for the batch: it admits only if
//! `current + batch_size <= limit`. Reading `batch_size` means reading the
//! request body in the middleware, which would consume the payload the
//! handler needs — so the guard buffers the body, counts the array's
//! elements, and **re-injects** the exact bytes before delegating
//! ([`peek_and_replay_json_array_len`]). The single-atom route needs no body
//! read (its delta is always 1).
//!
//! # Quota-exceeded response shape
//!
//! Exactly the plan's contract:
//!
//! ```json
//! { "error": "quota_exceeded",
//!   "metric": "atoms",
//!   "current": 100,
//!   "limit": 100,
//!   "resets_at": null,
//!   "upgrade_url": "https://app.<base>/billing" }
//! ```
//!
//! `resets_at` is `null` for resource limits — they don't reset on a clock,
//! they clear when the user deletes data or upgrades (plan: "Downgrade …
//! over-limit usage retained but writes blocked until under"). `upgrade_url`
//! is derived from the request host (`<sub>.<base>` → `https://app.<base>/billing`),
//! the same derivation the out-of-credits guard uses.

use actix_web::body::{BoxBody, MessageBody};
use actix_web::dev::{Payload, ServiceRequest, ServiceResponse};
use actix_web::http::{header, Method};
use actix_web::middleware::Next;
use actix_web::web::{self, Bytes};
use actix_web::{FromRequest, HttpMessage, HttpResponse};
use atomic_core::DatabaseManager;
use atomic_server::db_extractor::RequestDatabaseManager;

use crate::auth::ResolvedTenant;
use crate::plans::{Plan, PlanRegistry};

/// Which resource a mutating data-plane route consumes — the unit a quota
/// check is denominated in. `None` for routes the guard ignores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuotaTarget {
    /// `POST /api/atoms` — one atom.
    Atom,
    /// `POST /api/atoms/bulk` — N atoms; the batch size is read from the body.
    AtomBulk,
    /// `POST /api/databases` — one knowledge base.
    Kb,
}

/// Classify a `(method, path)` into the resource it consumes, or `None` if
/// the guard doesn't enforce it. Exact-path matches: only the collection
/// `POST`s create resources (`/api/atoms/{id}` is an update, `/api/databases/{id}`
/// a rename — neither grows the count).
fn quota_target(method: &Method, path: &str) -> Option<QuotaTarget> {
    if *method != Method::POST {
        return None;
    }
    match path {
        "/api/atoms" => Some(QuotaTarget::Atom),
        "/api/atoms/bulk" => Some(QuotaTarget::AtomBulk),
        "/api/databases" => Some(QuotaTarget::Kb),
        _ => None,
    }
}

/// Data-plane middleware enforcing plan-tier resource limits (module docs).
/// Wired inside CloudAuth and the plane guard, so [`ResolvedTenant`] and the
/// tenant manager are always installed; a missing extension is skipped
/// defensively (the plane guard already fails such requests closed). Runs
/// *outside* the dispatch-hint writer so a quota denial never marks a hint.
pub async fn quota_guard(
    registry: web::Data<PlanRegistry>,
    req: ServiceRequest,
    next: Next<impl MessageBody + 'static>,
) -> Result<ServiceResponse<BoxBody>, actix_web::Error> {
    let Some(target) = quota_target(req.method(), req.path()) else {
        return next.call(req).await.map(|res| res.map_into_boxed_body());
    };

    // Resolve the account + tenant manager off the extensions CloudAuth
    // installed. A request lacking them is a composition bug the plane guard
    // already fails closed — skip defensively rather than block. The
    // extensions `Ref` is fully scoped to this read so it is never held
    // across an await.
    let resolved = {
        let ext = req.extensions();
        let account_id = ext
            .get::<ResolvedTenant>()
            .map(|t| t.principal.account_id.clone());
        let manager = ext.get::<RequestDatabaseManager>().map(|m| m.0.clone());
        account_id.zip(manager)
    };
    let Some((account_id, manager)) = resolved else {
        return next.call(req).await.map(|res| res.map_into_boxed_body());
    };

    let plan = match registry.for_account(&account_id).await {
        Ok(plan) => plan,
        Err(e) => {
            tracing::error!(account_id, error = %e, "resolving plan for quota check failed");
            return Ok(req.into_response(internal_error()));
        }
    };

    // The bulk route's batch size must be read before the limit branch so
    // the body is buffered+replayed regardless of whether the plan is
    // unlimited (a one-shot read; replaying keeps the handler whole).
    let mut req = req;
    let delta: i64 = match target {
        QuotaTarget::Atom | QuotaTarget::Kb => 1,
        QuotaTarget::AtomBulk => match peek_and_replay_json_array_len(&mut req).await {
            Ok(n) => n as i64,
            // An unreadable/non-array body isn't ours to reject — let the
            // handler return its own deserialization 400. A zero-length
            // batch creates nothing, so it can never exceed a limit.
            Err(()) => 0,
        },
    };

    match check_resource(target, &plan, &manager, &req, delta).await {
        Ok(None) => next.call(req).await.map(|res| res.map_into_boxed_body()),
        Ok(Some(denial)) => Ok(req.into_response(denial)),
        Err(e) => {
            tracing::error!(account_id, error = %e, "reading resource count for quota check failed");
            Ok(req.into_response(internal_error()))
        }
    }
}

/// Run the resource check for `target`. `Ok(None)` admits; `Ok(Some(resp))`
/// is the 402 to return; `Err` is an operational fault reading the count.
async fn check_resource(
    target: QuotaTarget,
    plan: &Plan,
    manager: &DatabaseManager,
    req: &ServiceRequest,
    delta: i64,
) -> Result<Option<HttpResponse>, atomic_core::AtomicCoreError> {
    let (metric, limit) = match target {
        QuotaTarget::Atom | QuotaTarget::AtomBulk => ("atoms", plan.atom_limit),
        QuotaTarget::Kb => ("knowledge_bases", plan.kb_limit),
    };
    // NULL limit = unlimited: never read the count, never block.
    let Some(limit) = limit else {
        return Ok(None);
    };
    let limit = i64::from(limit);

    let current: i64 = match target {
        QuotaTarget::Atom | QuotaTarget::AtomBulk => {
            // Count atoms in the SAME knowledge base the create will target,
            // resolved exactly as atomic-server's handler will resolve it.
            let core = resolve_core(manager, req).await?;
            i64::from(core.count_atoms().await?)
        }
        QuotaTarget::Kb => manager.list_databases().await?.0.len() as i64,
    };

    // Admit only if the request keeps the tenant at-or-under the limit.
    // `current + delta` is the count the create would land on; `delta` is 1
    // for the single routes and the batch size for bulk.
    if current + delta > limit {
        return Ok(Some(quota_exceeded(metric, current, limit, req)));
    }
    Ok(None)
}

/// Resolve which knowledge-base core the request addresses within `manager`,
/// mirroring atomic-server's `resolve_core` selection (X-Atomic-Database
/// header → `?db=` → active) so the count is read against the exact KB the
/// handler will create into. CloudAuth injects the header for database-scoped
/// credentials, so this honors that pinning too.
async fn resolve_core(
    manager: &DatabaseManager,
    req: &ServiceRequest,
) -> Result<atomic_core::AtomicCore, atomic_core::AtomicCoreError> {
    if let Some(db_id) = req
        .headers()
        .get("x-atomic-database")
        .and_then(|v| v.to_str().ok())
    {
        return manager.get_core(db_id).await;
    }
    if let Some(db_id) = req.query_string().split('&').find_map(|pair| {
        let mut parts = pair.splitn(2, '=');
        if parts.next()? == "db" {
            parts.next()
        } else {
            None
        }
    }) {
        return manager.get_core(db_id).await;
    }
    manager.active_core().await
}

/// Buffer the request body, count the top-level JSON array's elements, and
/// **re-inject the exact bytes** so the downstream handler reads an
/// untouched payload. `Err(())` for a body that isn't a JSON array (or can't
/// be read) — the caller treats that as a zero delta and lets the handler
/// surface its own deserialization error.
async fn peek_and_replay_json_array_len(req: &mut ServiceRequest) -> Result<usize, ()> {
    // `web::Bytes::from_request` drains the payload into memory. Clone the
    // (cheap, Arc-backed) HttpRequest so the immutable `request()` borrow
    // doesn't overlap the mutable `take_payload()` borrow.
    let http_req = req.request().clone();
    let bytes = Bytes::from_request(&http_req, &mut req.take_payload())
        .await
        .map_err(|_| ())?;
    // Count without fully materializing every element into owned values.
    let len = match serde_json::from_slice::<Vec<serde::de::IgnoredAny>>(&bytes) {
        Ok(items) => items.len(),
        Err(_) => {
            // Not a JSON array — still replay the bytes so the handler can
            // produce its own 400, then report "not ours".
            req.set_payload(Payload::from(bytes));
            return Err(());
        }
    };
    req.set_payload(Payload::from(bytes));
    Ok(len)
}

/// Placeholder upgrade link, derived from the request host
/// (`<sub>.<base>` → `https://app.<base>/billing`) — the same derivation the
/// out-of-credits guard uses (plan: `upgrade_url` =
/// `https://app.<base-domain>/billing`).
fn upgrade_url(req: &ServiceRequest) -> String {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .or_else(|| req.uri().host())
        .unwrap_or_default();
    let host = host.split(':').next().unwrap_or(host);
    let base = host.split_once('.').map(|(_, base)| base).unwrap_or(host);
    format!("https://app.{base}/billing")
}

/// The plan's quota-exceeded response shape, verbatim. `resets_at` is always
/// `null` for resource limits (module docs).
fn quota_exceeded(metric: &str, current: i64, limit: i64, req: &ServiceRequest) -> HttpResponse {
    HttpResponse::PaymentRequired().json(serde_json::json!({
        "error": "quota_exceeded",
        "metric": metric,
        "current": current,
        "limit": limit,
        "resets_at": serde_json::Value::Null,
        "upgrade_url": upgrade_url(req),
    }))
}

fn internal_error() -> HttpResponse {
    HttpResponse::InternalServerError().json(serde_json::json!({ "error": "internal_error" }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quota_target_matches_only_creating_posts() {
        let post = Method::POST;
        assert_eq!(quota_target(&post, "/api/atoms"), Some(QuotaTarget::Atom));
        assert_eq!(
            quota_target(&post, "/api/atoms/bulk"),
            Some(QuotaTarget::AtomBulk)
        );
        assert_eq!(quota_target(&post, "/api/databases"), Some(QuotaTarget::Kb));
        // Updates, reads, and nested paths are not resource creates.
        for ignored in [
            "/api/atoms/abc",
            "/api/atoms/abc/process",
            "/api/databases/default",
            "/api/databases/default/activate",
            "/api/tags",
        ] {
            assert_eq!(quota_target(&post, ignored), None, "{ignored} ignored");
        }
        // Reads on the create paths are not creates.
        assert_eq!(quota_target(&Method::GET, "/api/atoms"), None);
        assert_eq!(quota_target(&Method::PUT, "/api/atoms"), None);
    }
}
