//! Static serving for the cloud account-plane SPA (the signup/login pages and
//! the authenticated `/account/*` dashboard).
//!
//! The single-page app is built once (`crates/atomic-cloud/frontend`, `npm run
//! build` → `dist/`) and this module serves that `dist/` as the cloud server's
//! **fallback** route: every JSON/API/OAuth/MCP/WS route is an explicit
//! service registered ahead of it (see [`configure_cloud_app`](crate::server)),
//! so the SPA fallback only ever handles a path no real route matched — it can
//! never shadow `/api/*`, `/oauth/*`, `/mcp`, `/ws`, `/health`, or `/ready`.
//!
//! Two responsibilities beyond "send a file":
//!
//! 1. **Base-domain injection.** The SPA tells an app host from a tenant
//!    subdomain by reading a `<meta name="atomic-cloud-base-domain">` tag the
//!    build leaves as a placeholder. This module rewrites that placeholder to
//!    the deployment's real base domain **once at startup**, holding the
//!    rewritten `index.html` in memory — so the client never falls back to
//!    host-guessing in production.
//!
//! 2. **SPA fallback.** A client-side-routed app must serve `index.html` for
//!    deep links (`/account/provider`, `/login`) that have no file on disk.
//!    A request for an **existing** file under `dist/` (a hashed asset, the
//!    favicon, a logo) is served as that file with a long-lived cache header;
//!    anything else returns the cached `index.html` (200, no-cache) so the
//!    router can take over. Requests are path-traversal-guarded — a resolved
//!    path that escapes the dist root is refused — so the fallback can never
//!    read outside the build output.
//!
//! The whole thing is **optional**: a deployment (or a test) that wires no
//! dist directory simply doesn't register the fallback, and unmatched paths
//! 404 as before. `serve` points it at `frontend/dist`; the README documents
//! producing that build.

use std::io;
use std::path::{Component, Path, PathBuf};

use actix_web::http::header::{
    CacheControl, CacheDirective, ContentType, HeaderValue, CONTENT_TYPE,
};
use actix_web::{web, HttpRequest, HttpResponse};

use crate::error::CloudError;

/// The meta-tag placeholder the build leaves in `index.html` for the server
/// to rewrite with the deployment's base domain. Must match the value in
/// `frontend/index.html` (and `frontend/src/lib/host.ts`).
const BASE_DOMAIN_PLACEHOLDER: &str = "__ATOMIC_CLOUD_BASE_DOMAIN__";

/// Cache lifetime for hashed build assets (`dist/assets/*`). The filenames
/// carry a content hash, so they are safe to cache effectively forever; a new
/// build emits new names. One year, the conventional "immutable" maximum.
const ASSET_MAX_AGE_SECS: u32 = 31_536_000;

/// The built SPA, ready to serve as the cloud server's fallback route.
///
/// Holds the dist root (for on-disk assets) and the base-domain-injected
/// `index.html` in memory (for the SPA fallback). Cheap to clone into each
/// worker — `index_html` is an `Arc<str>`, `root` an `Arc<Path>`.
#[derive(Clone)]
pub struct SpaServer {
    root: std::sync::Arc<Path>,
    index_html: std::sync::Arc<str>,
}

impl SpaServer {
    /// Load the SPA from `dist_dir`, injecting `base_domain` into the
    /// `index.html` base-domain meta placeholder.
    ///
    /// Fails if the directory has no `index.html` — a misconfigured serve
    /// (pointed at a directory that was never built) should surface at boot,
    /// not as a 404 on the first browser hit.
    pub async fn load(dist_dir: impl AsRef<Path>, base_domain: &str) -> Result<Self, CloudError> {
        let root = dist_dir.as_ref().to_path_buf();
        let index_path = root.join("index.html");
        let raw = tokio::fs::read_to_string(&index_path)
            .await
            .map_err(|source| CloudError::Io {
                context: format!(
                    "reading SPA index.html at {} (was `npm run build` run in \
                     crates/atomic-cloud/frontend?)",
                    index_path.display()
                ),
                source,
            })?;
        // Normalize the base domain the same way the client does (lowercase,
        // no leading dot) so the meta value is canonical regardless of how the
        // operator spelled `--base-domain`.
        let normalized = base_domain.trim_start_matches('.').to_lowercase();
        let index_html = raw.replace(BASE_DOMAIN_PLACEHOLDER, &normalized);
        Ok(Self {
            root: root.into(),
            index_html: index_html.into(),
        })
    }

    /// Build the [`SpaServer`] only if `dist_dir` exists and carries an
    /// `index.html`; otherwise `Ok(None)`.
    ///
    /// `serve` uses this so a deployment without a built frontend (a
    /// pure-API pod, a dev run that hasn't run `npm run build`) boots cleanly
    /// with the SPA fallback simply absent, rather than failing. A directory
    /// that exists but is missing `index.html` is treated as "not built" — the
    /// same `None`, with a single loud log at the call site.
    pub async fn load_optional(
        dist_dir: impl AsRef<Path>,
        base_domain: &str,
    ) -> Result<Option<Self>, CloudError> {
        let index_path = dist_dir.as_ref().join("index.html");
        match tokio::fs::try_exists(&index_path).await {
            Ok(true) => Self::load(dist_dir, base_domain).await.map(Some),
            Ok(false) => Ok(None),
            Err(source) => Err(CloudError::Io {
                context: format!("probing for SPA index.html at {}", index_path.display()),
                source,
            }),
        }
    }

    /// The base-domain-injected `index.html`, for the fallback and tests.
    pub fn index_html(&self) -> &str {
        &self.index_html
    }

    /// Register this server as the app's fallback (`default_service`). Must be
    /// called **last** in `configure`, after every explicit route — actix
    /// matches explicit services first and only falls through to the default
    /// service when none matched.
    pub fn configure_fallback(self, cfg: &mut web::ServiceConfig) {
        cfg.app_data(web::Data::new(self))
            .default_service(web::route().to(serve_spa));
    }
}

/// The fallback handler: serve an existing build file, or the SPA shell.
///
/// Only `GET`/`HEAD` are served as the SPA; any other method on an unmatched
/// path is a genuine 404 (a `POST` to a nonexistent API route is a client
/// error, not a page navigation). This keeps the fallback from masking a
/// mistyped API call as an HTML 200.
async fn serve_spa(req: HttpRequest, spa: web::Data<SpaServer>) -> HttpResponse {
    if !matches!(*req.method(), actix_web::http::Method::GET | actix_web::http::Method::HEAD) {
        return HttpResponse::NotFound().json(serde_json::json!({ "error": "not_found" }));
    }

    // Try to serve an existing file under dist for the request path; fall back
    // to the SPA shell for anything that isn't a real file (deep links).
    if let Some(resolved) = resolve_asset_path(&spa.root, req.path()) {
        match tokio::fs::read(&resolved).await {
            Ok(bytes) => return asset_response(&resolved, bytes),
            // A path that resolved inside dist but isn't a file (a directory,
            // or a deep link that coincidentally matches a real subdir name)
            // falls through to the shell.
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => {
                tracing::error!(path = req.path(), error = %e, "serving SPA asset failed");
                return HttpResponse::InternalServerError()
                    .json(serde_json::json!({ "error": "asset_read_failed" }));
            }
        }
    }

    spa_shell_response(spa.index_html())
}

/// Resolve a request path to a candidate file under `root`, or `None` when the
/// path is the SPA root (`/`) or escapes the dist directory.
///
/// Traversal-guarded: the path is split into components, `..` and absolute
/// roots are rejected outright (rather than popped), so no request can address
/// a file outside the build output even through an encoded or layered `..`.
fn resolve_asset_path(root: &Path, request_path: &str) -> Option<PathBuf> {
    let trimmed = request_path.trim_start_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    let rel = Path::new(trimmed);
    let mut safe = PathBuf::new();
    for component in rel.components() {
        match component {
            Component::Normal(part) => safe.push(part),
            // Anything that isn't a plain path segment — `..`, a root, a
            // Windows prefix, a bare `.` — disqualifies the whole path: a
            // request that tries to climb out of dist gets the shell, never a
            // file outside it.
            _ => return None,
        }
    }
    Some(root.join(safe))
}

/// A 200 for an existing build file, with a content type from its extension
/// and a cache header keyed on whether it's a hashed asset.
fn asset_response(path: &Path, bytes: Vec<u8>) -> HttpResponse {
    let mut builder = HttpResponse::Ok();
    if let Some(content_type) = content_type_for(path) {
        builder.insert_header((CONTENT_TYPE, content_type));
    }
    // Files under `assets/` carry a content hash in their name → immutable,
    // cache hard. Everything else (favicon, logos) gets a short cache.
    let is_hashed_asset = path
        .components()
        .any(|c| matches!(c, Component::Normal(part) if part == "assets"));
    if is_hashed_asset {
        builder.insert_header(CacheControl(vec![
            CacheDirective::Public,
            CacheDirective::MaxAge(ASSET_MAX_AGE_SECS),
            CacheDirective::Extension("immutable".to_string(), None),
        ]));
    } else {
        builder.insert_header(CacheControl(vec![
            CacheDirective::Public,
            CacheDirective::MaxAge(3600),
        ]));
    }
    builder.body(bytes)
}

/// The SPA shell (`index.html`) response: 200, HTML, explicitly **not**
/// cached, so a deploy's new asset hashes are always picked up (the shell is
/// the one file whose name doesn't change between builds).
fn spa_shell_response(index_html: &str) -> HttpResponse {
    HttpResponse::Ok()
        .insert_header(ContentType::html())
        .insert_header(CacheControl(vec![CacheDirective::NoCache]))
        .body(index_html.to_string())
}

/// A minimal extension → `Content-Type` map for the file kinds a Vite build
/// emits. Unknown extensions get no header (actix/browsers sniff), which is
/// fine for the SPA's narrow output set.
fn content_type_for(path: &Path) -> Option<HeaderValue> {
    let ext = path.extension().and_then(|e| e.to_str())?.to_ascii_lowercase();
    let value = match ext.as_str() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "txt" => "text/plain; charset=utf-8",
        "map" => "application/json",
        _ => return None,
    };
    HeaderValue::from_static(value).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_rejects_traversal_and_root() {
        let root = Path::new("/srv/dist");
        // The SPA root has no file — it's the shell.
        assert_eq!(resolve_asset_path(root, "/"), None);
        assert_eq!(resolve_asset_path(root, ""), None);
        // Traversal in any form is refused (→ shell, never a file outside).
        assert_eq!(resolve_asset_path(root, "/../secret"), None);
        assert_eq!(resolve_asset_path(root, "/assets/../../etc/passwd"), None);
        assert_eq!(resolve_asset_path(root, "/a/../../b"), None);
        // A normal asset path resolves under the root.
        assert_eq!(
            resolve_asset_path(root, "/assets/index-abc123.js"),
            Some(PathBuf::from("/srv/dist/assets/index-abc123.js"))
        );
        assert_eq!(
            resolve_asset_path(root, "/favicon.svg"),
            Some(PathBuf::from("/srv/dist/favicon.svg"))
        );
    }

    #[test]
    fn content_types_cover_vite_output() {
        for (file, expected) in [
            ("index.html", "text/html; charset=utf-8"),
            ("assets/index-abc.js", "text/javascript; charset=utf-8"),
            ("assets/index-abc.css", "text/css; charset=utf-8"),
            ("logo.svg", "image/svg+xml"),
            ("font.woff2", "font/woff2"),
        ] {
            let ct = content_type_for(Path::new(file)).expect("known type");
            assert_eq!(ct.to_str().unwrap(), expected, "{file}");
        }
        assert!(content_type_for(Path::new("weird.xyz")).is_none());
    }

    #[tokio::test]
    async fn load_injects_base_domain_into_meta() {
        let dir = tempfile::tempdir().expect("tempdir");
        tokio::fs::write(
            dir.path().join("index.html"),
            format!(
                r#"<meta name="atomic-cloud-base-domain" content="{BASE_DOMAIN_PLACEHOLDER}" />"#
            ),
        )
        .await
        .expect("write index");

        let spa = SpaServer::load(dir.path(), ".Atomic.Cloud")
            .await
            .expect("load spa");
        assert!(
            spa.index_html().contains(r#"content="atomic.cloud""#),
            "base domain normalized + injected: {}",
            spa.index_html()
        );
        assert!(
            !spa.index_html().contains(BASE_DOMAIN_PLACEHOLDER),
            "placeholder fully replaced"
        );
    }

    #[tokio::test]
    async fn load_optional_absent_when_unbuilt() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Directory exists but has no index.html → treated as "not built".
        let spa = SpaServer::load_optional(dir.path(), "atomic.cloud")
            .await
            .expect("probe");
        assert!(spa.is_none(), "no index.html → no SPA server");
    }
}
