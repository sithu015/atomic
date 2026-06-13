//! The real [`StripeClient`] request shape, pinned with wiremock — NO REAL
//! STRIPE (plan/testing convention: the HTTP client goes behind a trait with
//! a scriptable double, and the real impl gets a wiremock test for request
//! shape). The webhook signature scheme is unit-tested in `src/billing.rs`
//! over a known-secret HMAC fixture; this proves the outbound calls send the
//! right form bodies, the Basic-auth secret, and parse the `url` out of the
//! response — without a Stripe account or network.
//!
//! Not Postgres-gated: it never touches a database.

use atomic_cloud::{BillingProvider, StripeClient};
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn checkout_session_request_shape_and_parsing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/checkout/sessions"))
        // The secret key rides in Basic auth (username, empty password):
        // base64("sk_test_x:") = "c2tfdGVzdF94Og==".
        .and(header("authorization", "Basic c2tfdGVzdF94Og=="))
        .and(body_string_contains("mode=subscription"))
        .and(body_string_contains("customer_email=k%40example.com"))
        .and(body_string_contains("metadata%5Bsubdomain%5D=alpha"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "url": "https://checkout.stripe.test/cs_1" })),
        )
        .mount(&server)
        .await;

    let client = StripeClient::with_base_url("sk_test_x", server.uri()).expect("client");
    let session = client
        .create_checkout_session(
            "price_pro",
            "k@example.com",
            "alpha",
            "https://app.test/billing?status=success",
            "https://app.test/billing?status=cancel",
        )
        .await
        .expect("checkout session");
    assert_eq!(session.url, "https://checkout.stripe.test/cs_1");
}

#[tokio::test]
async fn portal_session_request_shape_and_parsing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/billing_portal/sessions"))
        .and(body_string_contains("customer=cus_1"))
        .and(body_string_contains("return_url="))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "url": "https://portal.stripe.test/ps_1" })),
        )
        .mount(&server)
        .await;

    let client = StripeClient::with_base_url("sk_test_x", server.uri()).expect("client");
    let session = client
        .create_portal_session("cus_1", "https://app.test/billing")
        .await
        .expect("portal session");
    assert_eq!(session.url, "https://portal.stripe.test/ps_1");
}

#[tokio::test]
async fn non_success_status_maps_to_typed_stripe_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/checkout/sessions"))
        .respond_with(ResponseTemplate::new(402).set_body_json(serde_json::json!({
            "error": { "message": "Your card was declined." }
        })))
        .mount(&server)
        .await;

    let client = StripeClient::with_base_url("sk_test_x", server.uri()).expect("client");
    let err = client
        .create_checkout_session("price_pro", "k@example.com", "alpha", "s", "c")
        .await
        .expect_err("declined");
    // The error carries the status + a bounded body slice, never the key.
    let msg = err.to_string();
    assert!(msg.contains("402"), "status surfaced: {msg}");
    assert!(!msg.contains("sk_test_x"), "secret key never leaks: {msg}");
}
