//! Data-plane write-guard for the `read_only` billing state (plan: "Billing"
//! → "3 days past_due → Read-only mode"; "Subscription deleted … read-only
//! until under").
//!
//! `suspended` is gated up front in [`CloudAuth`](crate::auth) — the request
//! never reaches a handler. `read_only` is subtler: reads and exports must
//! keep working (the user can still see and retrieve their data; nothing is
//! deleted), only *writes* are blocked. That distinction needs the request
//! method, so it lives here as a data-plane middleware that 402s mutating
//! methods while passing reads through. Wired inside CloudAuth and the plane
//! guard (so [`ResolvedTenant`] is installed) and *outside* the dispatch-hint
//! writer, so a blocked write never marks a hint.
//!
//! The same pattern the quota and out-of-credits guards use: read the state
//! off [`ResolvedTenant`] (CloudAuth already loaded it from the accounts row
//! this request paid for), so the guard itself does no I/O.

use actix_web::body::{BoxBody, MessageBody};
use actix_web::dev::{ServiceRequest, ServiceResponse};
use actix_web::http::{header, Method};
use actix_web::middleware::Next;
use actix_web::{HttpMessage, HttpResponse};

use crate::auth::ResolvedTenant;

/// Whether `method` mutates. The write-guard blocks exactly these while the
/// tenant is `read_only`; GET/HEAD/OPTIONS reads pass.
fn is_mutating(method: &Method) -> bool {
    matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    )
}

/// Data-plane middleware: while the tenant is `billing_state = 'read_only'`,
/// return a structured 402 for mutating requests (module docs). A missing
/// [`ResolvedTenant`] is skipped defensively (the plane guard fails such
/// requests closed already).
pub async fn billing_write_guard(
    req: ServiceRequest,
    next: Next<impl MessageBody + 'static>,
) -> Result<ServiceResponse<BoxBody>, actix_web::Error> {
    let read_only = req
        .extensions()
        .get::<ResolvedTenant>()
        .is_some_and(|t| t.billing_state.blocks_writes());

    if read_only && is_mutating(req.method()) {
        let host = req
            .headers()
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .or_else(|| req.uri().host())
            .unwrap_or_default();
        let denial = HttpResponse::PaymentRequired().json(serde_json::json!({
            "error": "account_read_only",
            "message": "This account is read-only for non-payment. Your data is \
                        retained and readable; update your billing to resume editing.",
            "upgrade_url": upgrade_url(host),
        }));
        return Ok(req.into_response(denial));
    }
    next.call(req).await.map(|res| res.map_into_boxed_body())
}

/// `<sub>.<base>` → `https://app.<base>/billing` (plan: `upgrade_url`).
fn upgrade_url(host: &str) -> String {
    let host = host.split(':').next().unwrap_or(host);
    let base = host.split_once('.').map(|(_, base)| base).unwrap_or(host);
    format!("https://app.{base}/billing")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_mutating_methods_blocked() {
        assert!(is_mutating(&Method::POST));
        assert!(is_mutating(&Method::PUT));
        assert!(is_mutating(&Method::PATCH));
        assert!(is_mutating(&Method::DELETE));
        assert!(!is_mutating(&Method::GET));
        assert!(!is_mutating(&Method::HEAD));
        assert!(!is_mutating(&Method::OPTIONS));
    }

    #[test]
    fn upgrade_url_derives_app_host() {
        assert_eq!(
            upgrade_url("kenny.atomic.cloud"),
            "https://app.atomic.cloud/billing"
        );
    }
}
