//! Provider backpressure: the per-tenant circuit breaker and the
//! interactive `out_of_ai_credits` surface (plan: "Worker fairness & job
//! queue" → "Provider rate-limit handling"; "Provider management" →
//! "Managed key lifecycle" / "Blocked states").
//!
//! # Two layers
//!
//! **Layer 1 — local backoff** lives with the ledgers, not here: a
//! rate-limited `task_runs` execution gets its `next_attempt_at` pushed out
//! by the ledger's own exponential backoff (`scheduler::ledger::fail`), and
//! a rate-limited pipeline job is re-enqueued by the dispatcher's executor
//! with `not_before` honoring the provider's `Retry-After` hint (see
//! [`crate::dispatcher::CoreExecutor`]). This module is **layer 2**: when
//! one tenant's provider keeps rate-limiting, per-row backoff alone would
//! keep burning dispatch capacity (and the provider's goodwill) on a tenant
//! that needs a real pause.
//!
//! # The breaker state machine
//!
//! - **Detection is in-memory, per pod**: a sliding window of
//!   rate-limit-classified failures per tenant. Three within 60 seconds
//!   trips the breaker. Per-pod detection is deliberate — sharing failure
//!   counts across pods would buy a slightly earlier trip at the cost of a
//!   control-plane write per failure; a noisy tenant trips every pod's
//!   window within seconds anyway.
//! - **The pause itself is control plane**, so every pod honors it:
//!   `accounts.provider_paused_until` (+ `provider_pause_kind`,
//!   `provider_pause_streak`; migration 007). The dispatcher skips paused
//!   tenants wholesale — their ledger work *sits*, it never fails.
//! - **Cooldown doubles per consecutive trip** (60s, 120s, …, capped at
//!   1h), tracked by `provider_pause_streak`. A healthy run resets the
//!   streak ([`ProviderBreaker::record_healthy`]); so does any provider
//!   mutation (BYOK save / activate / models write — rotation step 6, in
//!   [`crate::provider_credentials`]).
//! - **Credit exhaustion (402)** pauses immediately with
//!   `kind = 'credits'`, until the allowance's reset when known, else a
//!   recheck horizon (1h). It does not touch the streak — it isn't a
//!   rate-limit escalation, it's a billing wall that lifts on reset,
//!   upgrade, or key rotation.
//!
//! # The rate-limit / credits asymmetry
//!
//! A **rate-limit** pause stops *background dispatch only*. Interactive
//! traffic (chat, an explicit wiki generate) still goes to the provider —
//! it has its own provider-side limiting, and a user staring at a chat box
//! deserves the provider's real answer, not a synthetic one. The breaker
//! protects the *background* budget.
//!
//! A **credits** pause additionally turns the AI-interactive routes into a
//! structured 402 ([`out_of_credits_guard`]): the provider would refuse
//! anyway, and "out of AI credits, resets at X" is a better answer than a
//! provider error string. Atom CRUD — and everything else non-AI — stays
//! fully functional; the ledger holds the background work for when the
//! allowance returns.
//!
//! One last-writer-wins residue: the two pause kinds share one column pair,
//! so a rate-limit trip while a credits pause is in force overwrites the
//! kind, briefly re-opening the interactive routes. The next interactive or
//! dispatched call hits the provider's 402 and re-pauses as credits —
//! self-healing, and not worth a second column pair.

use std::collections::{HashMap, VecDeque};
use std::str::FromStr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use actix_web::body::MessageBody;
use actix_web::dev::{ServiceRequest, ServiceResponse};
use actix_web::http::{header, Method};
use actix_web::middleware::Next;
use actix_web::{HttpMessage, HttpResponse};
use chrono::{DateTime, Utc};

use crate::auth::ResolvedTenant;
use crate::control_plane::ControlPlane;
use crate::error::CloudError;

/// Why a tenant's provider dispatch is paused. Serialized to text in
/// `accounts.provider_pause_kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseKind {
    /// Repeated provider rate limiting tripped the circuit breaker.
    /// Background dispatch pauses; interactive routes are unaffected.
    RateLimit,
    /// The provider refused for billing reasons (402 — exhausted managed
    /// allowance, or a BYOK key out of its own credits). Background
    /// dispatch pauses AND the AI-interactive routes return the structured
    /// `out_of_ai_credits` error.
    Credits,
}

impl PauseKind {
    /// The text stored in `accounts.provider_pause_kind`.
    pub fn as_str(self) -> &'static str {
        match self {
            PauseKind::RateLimit => "rate_limit",
            PauseKind::Credits => "credits",
        }
    }
}

impl std::fmt::Display for PauseKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for PauseKind {
    type Err = CloudError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "rate_limit" => Ok(PauseKind::RateLimit),
            "credits" => Ok(PauseKind::Credits),
            other => Err(CloudError::InvalidPauseKind(other.to_string())),
        }
    }
}

/// A tenant's provider pause, as read off the accounts row. Carried on
/// [`ResolvedTenant`] by CloudAuth (the lookup it already makes per
/// request); consumers compare `until` against their own `now` — a pause is
/// only in force while `until` is in the future.
#[derive(Debug, Clone, Copy)]
pub struct ProviderPause {
    pub until: DateTime<Utc>,
    pub kind: PauseKind,
}

impl ProviderPause {
    /// Reconstruct from the raw column pair. `None` when not paused; a
    /// present timestamp with a missing/unknown kind degrades to a
    /// rate-limit-shaped pause (background-only — the conservative reading:
    /// it never blocks a user) with a loud log, rather than failing the
    /// request over one corrupt column.
    pub fn from_columns(until: Option<DateTime<Utc>>, kind: Option<&str>) -> Option<Self> {
        let until = until?;
        let kind = match kind.map(PauseKind::from_str) {
            Some(Ok(kind)) => kind,
            other => {
                tracing::warn!(
                    ?other,
                    "provider_pause_kind missing or unknown; treating pause as rate_limit"
                );
                PauseKind::RateLimit
            }
        };
        Some(ProviderPause { until, kind })
    }

    /// Whether the pause is in force at `now`.
    pub fn active_at(&self, now: DateTime<Utc>) -> bool {
        self.until > now
    }
}

/// Tuning knobs for the circuit breaker. Defaults are the plan's numbers.
#[derive(Debug, Clone)]
pub struct BreakerConfig {
    /// Sliding detection window for rate-limit failures.
    pub window: Duration,
    /// Failures within [`Self::window`] that trip the breaker.
    pub threshold: usize,
    /// First trip's cooldown; doubles per consecutive trip.
    pub base_cooldown: Duration,
    /// Cooldown ceiling.
    pub max_cooldown: Duration,
    /// Credits-pause horizon when the provider exposes no reset time: how
    /// long until dispatch re-probes the allowance.
    pub credits_recheck: Duration,
}

impl Default for BreakerConfig {
    fn default() -> Self {
        Self {
            window: Duration::from_secs(60),
            threshold: 3,
            base_cooldown: Duration::from_secs(60),
            max_cooldown: Duration::from_secs(60 * 60),
            credits_recheck: Duration::from_secs(60 * 60),
        }
    }
}

/// The per-pod circuit breaker (module docs: the breaker state machine).
/// Detection state is in-memory; pauses are control-plane writes every pod
/// honors. Cheap to share via `Arc`.
pub struct ProviderBreaker {
    control: ControlPlane,
    config: BreakerConfig,
    /// Per-tenant sliding windows of recent rate-limit failures. Bounded:
    /// at most `threshold` entries per tenant (a trip clears the window),
    /// and tenants prune to nothing as their entries age out.
    windows: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl ProviderBreaker {
    pub fn new(control: ControlPlane, config: BreakerConfig) -> Self {
        Self {
            control,
            config,
            windows: Mutex::new(HashMap::new()),
        }
    }

    /// Record one rate-limit-classified failure for `account_id`. When this
    /// failure is the `threshold`-th within the window, trip the breaker:
    /// pause the tenant in the control plane (cooldown doubled by the
    /// stored streak) and return the pause horizon. `Ok(None)` means "noted,
    /// not tripped" — or the account vanished under a concurrent deletion.
    pub async fn record_rate_limited(
        &self,
        account_id: &str,
    ) -> Result<Option<DateTime<Utc>>, CloudError> {
        if !self.note_failure(account_id, Instant::now()) {
            return Ok(None);
        }

        // Trip. One statement so the streak read and the doubled cooldown
        // can't interleave with a concurrent trip from another pod: the
        // RHS sees the pre-update streak, so cooldown = base * 2^streak,
        // capped. The exponent is clamped well below f64 overflow.
        let paused_until: Option<DateTime<Utc>> = sqlx::query_scalar(
            "UPDATE accounts SET \
                 provider_pause_streak = provider_pause_streak + 1, \
                 provider_pause_kind = 'rate_limit', \
                 provider_paused_until = NOW() + make_interval(secs => LEAST( \
                     $2, \
                     $3 * power(2::double precision, \
                                LEAST(provider_pause_streak, 30)::double precision))) \
             WHERE id = $1 \
             RETURNING provider_paused_until",
        )
        .bind(account_id)
        .bind(self.config.max_cooldown.as_secs_f64())
        .bind(self.config.base_cooldown.as_secs_f64())
        .fetch_optional(self.control.pool())
        .await
        .map_err(CloudError::db("tripping provider circuit breaker"))?;

        match paused_until {
            Some(until) => {
                tracing::warn!(
                    account_id,
                    paused_until = %until,
                    "provider circuit breaker tripped; tenant dispatch paused"
                );
                Ok(Some(until))
            }
            // The account was deleted between the failure and the trip.
            None => Ok(None),
        }
    }

    /// Record a credit-exhaustion (402) failure: pause immediately with
    /// `kind = 'credits'` until `resets_at` (when the provider exposed one)
    /// or the configured recheck horizon. Returns the pause horizon;
    /// `Ok(None)` when the account vanished concurrently. The rate-limit
    /// streak is untouched (module docs).
    pub async fn record_payment_required(
        &self,
        account_id: &str,
        resets_at: Option<DateTime<Utc>>,
    ) -> Result<Option<DateTime<Utc>>, CloudError> {
        let until = resets_at.unwrap_or_else(|| {
            Utc::now() + chrono::Duration::from_std(self.config.credits_recheck).unwrap()
        });
        let updated = sqlx::query(
            "UPDATE accounts SET \
                 provider_paused_until = $2, \
                 provider_pause_kind = 'credits' \
             WHERE id = $1",
        )
        .bind(account_id)
        .bind(until)
        .execute(self.control.pool())
        .await
        .map_err(CloudError::db("pausing tenant on credit exhaustion"))?;
        if updated.rows_affected() == 0 {
            return Ok(None);
        }
        tracing::warn!(
            account_id,
            paused_until = %until,
            "tenant out of AI credits; dispatch paused and interactive AI routes blocked"
        );
        Ok(Some(until))
    }

    /// Record a healthy (provider-failure-free) execution: drop the
    /// in-memory window and reset the consecutive-trip streak, so the next
    /// trip starts back at the base cooldown. The streak write is
    /// conditioned on `<> 0`, so the steady state is a no-op row match.
    pub async fn record_healthy(&self, account_id: &str) -> Result<(), CloudError> {
        {
            let mut windows = self.windows.lock().expect("breaker windows poisoned");
            windows.remove(account_id);
        }
        sqlx::query(
            "UPDATE accounts SET provider_pause_streak = 0 \
             WHERE id = $1 AND provider_pause_streak <> 0",
        )
        .bind(account_id)
        .execute(self.control.pool())
        .await
        .map_err(CloudError::db("resetting provider pause streak"))?;
        Ok(())
    }

    /// Slide the window and decide whether this failure trips the breaker.
    /// Pure in-memory bookkeeping, split out (and instant-parameterized) so
    /// the threshold/window semantics are unit-testable without a clock.
    fn note_failure(&self, account_id: &str, now: Instant) -> bool {
        let mut windows = self.windows.lock().expect("breaker windows poisoned");
        let window = windows.entry(account_id.to_string()).or_default();
        window.push_back(now);
        while let Some(front) = window.front() {
            if now.duration_since(*front) > self.config.window {
                window.pop_front();
            } else {
                break;
            }
        }
        if window.len() >= self.config.threshold {
            // Trip consumes the window: the next failure starts a fresh
            // count rather than instantly re-tripping on stale entries.
            windows.remove(account_id);
            true
        } else {
            false
        }
    }
}

// --- The interactive out-of-credits surface ---------------------------------

/// Whether `(method, path)` is one of the AI-interactive routes a credits
/// pause must answer with the structured `out_of_ai_credits` error: the
/// synchronous, user-facing operations whose handlers would otherwise call
/// the provider inline (plan: "chat/wiki/reports return a structured 'out
/// of AI credits' error"). Everything else — atom CRUD above all — stays
/// fully functional; background work is held by the dispatcher's pause
/// gate, not by this guard.
///
/// Deliberately NOT guarded: semantic search and tag compaction also spend
/// credits, but they degrade with the provider's own error rather than
/// pre-empting — they're accents, not the product's blocked surface; and
/// the embedding-management retry routes only *enqueue* ledger work, which
/// the pause already holds.
pub fn ai_interactive_route(method: &Method, path: &str) -> bool {
    if *method != Method::POST {
        return false;
    }
    // Chat: POST /api/conversations/{id}/messages.
    if let Some(rest) = path.strip_prefix("/api/conversations/") {
        if let Some((id, tail)) = rest.split_once('/') {
            return !id.is_empty() && tail == "messages";
        }
        return false;
    }
    // Wiki synthesis: POST /api/wiki/{tag}/(generate|update|propose).
    if let Some(rest) = path.strip_prefix("/api/wiki/") {
        if let Some((tag, tail)) = rest.split_once('/') {
            return !tag.is_empty() && matches!(tail, "generate" | "update" | "propose");
        }
        return false;
    }
    // Reports: POST /api/reports/{id}/run.
    if let Some(rest) = path.strip_prefix("/api/reports/") {
        if let Some((id, tail)) = rest.split_once('/') {
            return !id.is_empty() && tail == "run";
        }
        return false;
    }
    false
}

/// Placeholder upgrade link for the `out_of_ai_credits` body, derived from
/// the request's host (`<sub>.<base>` → `https://app.<base>/billing`). The
/// billing slice owns the real destination; the *shape* of the response is
/// this slice's contract.
fn upgrade_url(host: &str) -> String {
    let host = host.split(':').next().unwrap_or(host);
    let base = host.split_once('.').map(|(_, base)| base).unwrap_or(host);
    format!("https://app.{base}/billing")
}

/// Data-plane middleware: while the tenant is paused with
/// `kind = 'credits'`, the AI-interactive routes return the structured 402
/// (module docs: the rate-limit / credits asymmetry). Reads the pause off
/// [`ResolvedTenant`] — CloudAuth loaded it from the accounts row this
/// request already paid for — so the guard itself does no I/O. Wired inside
/// CloudAuth and the plane guard; a missing extension is skipped
/// defensively (the plane guard already fails such requests closed).
pub async fn out_of_credits_guard(
    req: ServiceRequest,
    next: Next<impl MessageBody + 'static>,
) -> Result<ServiceResponse<actix_web::body::BoxBody>, actix_web::Error> {
    let pause = req
        .extensions()
        .get::<ResolvedTenant>()
        .and_then(|tenant| tenant.provider_pause);
    if let Some(pause) = pause {
        if pause.kind == PauseKind::Credits
            && pause.active_at(Utc::now())
            && ai_interactive_route(req.method(), req.path())
        {
            let host = req
                .headers()
                .get(header::HOST)
                .and_then(|v| v.to_str().ok())
                .or_else(|| req.uri().host())
                .unwrap_or_default();
            let denial = HttpResponse::PaymentRequired().json(serde_json::json!({
                "error": "out_of_ai_credits",
                "message": "This account is out of AI credits. AI features resume \
                            when the allowance resets or after an upgrade; your \
                            notes remain fully accessible and editable.",
                "resets_at": pause.until.to_rfc3339(),
                "upgrade_url": upgrade_url(host),
            }));
            return Ok(req.into_response(denial));
        }
    }
    next.call(req).await.map(|res| res.map_into_boxed_body())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_kind_round_trips_through_text() {
        for kind in [PauseKind::RateLimit, PauseKind::Credits] {
            assert_eq!(kind.as_str().parse::<PauseKind>().unwrap(), kind);
        }
        assert!(matches!(
            "blocked".parse::<PauseKind>(),
            Err(CloudError::InvalidPauseKind(_))
        ));
    }

    #[test]
    fn pause_from_columns() {
        assert!(ProviderPause::from_columns(None, None).is_none());
        assert!(ProviderPause::from_columns(None, Some("credits")).is_none());

        let until = Utc::now();
        let pause = ProviderPause::from_columns(Some(until), Some("credits")).unwrap();
        assert_eq!(pause.kind, PauseKind::Credits);
        assert_eq!(pause.until, until);

        // Unknown/missing kind degrades to the conservative reading.
        for kind in [None, Some("mystery")] {
            let pause = ProviderPause::from_columns(Some(until), kind).unwrap();
            assert_eq!(pause.kind, PauseKind::RateLimit);
        }
    }

    /// The detection window: exactly `threshold` failures inside the window
    /// trip; fewer don't; stale failures age out. (`tokio::test` because
    /// even a lazy, never-used sqlx pool wants a runtime to exist in.)
    #[tokio::test]
    async fn window_trips_on_threshold_not_before() {
        // A control-plane-free breaker: note_failure never touches the DB.
        let breaker = breaker_without_db();
        let t0 = Instant::now();

        assert!(!breaker.note_failure("acct", t0));
        assert!(!breaker.note_failure("acct", t0 + Duration::from_secs(10)));
        assert!(
            breaker.note_failure("acct", t0 + Duration::from_secs(20)),
            "third failure within 60s must trip"
        );

        // The trip consumed the window: the count restarts.
        assert!(!breaker.note_failure("acct", t0 + Duration::from_secs(21)));

        // Aged-out failures don't count: two old + one new ≠ trip.
        let breaker = breaker_without_db();
        assert!(!breaker.note_failure("acct", t0));
        assert!(!breaker.note_failure("acct", t0 + Duration::from_secs(1)));
        assert!(
            !breaker.note_failure("acct", t0 + Duration::from_secs(120)),
            "failures older than the window must not count toward a trip"
        );
    }

    /// Tenants are isolated: one tenant's failures never advance another's
    /// window.
    #[tokio::test]
    async fn windows_are_per_tenant() {
        let breaker = breaker_without_db();
        let t0 = Instant::now();
        assert!(!breaker.note_failure("a", t0));
        assert!(!breaker.note_failure("a", t0));
        assert!(
            !breaker.note_failure("b", t0),
            "b has one failure, not three"
        );
        assert!(breaker.note_failure("a", t0));
    }

    /// `note_failure` is pure in-memory bookkeeping, so a lazy pool that is
    /// never used lets the window logic run without a database.
    fn breaker_without_db() -> ProviderBreaker {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
            .expect("lazy pool");
        ProviderBreaker::new(
            ControlPlane::from_pool_for_tests(pool),
            BreakerConfig::default(),
        )
    }

    #[test]
    fn interactive_route_matcher() {
        let post = Method::POST;
        for guarded in [
            "/api/conversations/abc-123/messages",
            "/api/wiki/tag-1/generate",
            "/api/wiki/tag-1/update",
            "/api/wiki/tag-1/propose",
            "/api/reports/r-1/run",
        ] {
            assert!(
                ai_interactive_route(&post, guarded),
                "{guarded} must be guarded"
            );
        }
        for open in [
            "/api/atoms",
            "/api/conversations",
            "/api/conversations/abc-123/scope",
            "/api/conversations//messages",
            "/api/wiki/tag-1/proposal/accept",
            "/api/wiki/tag-1",
            "/api/reports",
            "/api/reports/r-1/findings",
            "/api/search",
        ] {
            assert!(!ai_interactive_route(&post, open), "{open} must stay open");
        }
        // Only POST is interactive; reads on the same paths stay open.
        assert!(!ai_interactive_route(
            &Method::GET,
            "/api/conversations/abc-123/messages"
        ));
    }

    #[test]
    fn upgrade_url_derives_app_host() {
        assert_eq!(
            upgrade_url("kenny.atomic.cloud"),
            "https://app.atomic.cloud/billing"
        );
        assert_eq!(
            upgrade_url("kenny.cloudtest.local:8080"),
            "https://app.cloudtest.local/billing"
        );
    }
}
