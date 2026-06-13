//! Period rollover and storage-bytes enforcement — the two control-plane
//! jobs that write the `quota_usage` table (plan: "Observability, quotas,
//! billing" → "Quotas").
//!
//! Both live here rather than in [`crate::reaper`] because both are *quota*
//! mechanics keyed on the plan registry and the `quota_usage` schema, not the
//! reaper's provisioning/migration recovery. `serve` runs them on their own
//! intervals (a period-rollover loop and a storage-recompute loop), each
//! interval glue around a tested pure-ish function that takes an explicit
//! `now`/policy — the slice-2/5 reaper-test idiom.
//!
//! # What `quota_usage` is (and is not) for
//!
//! Resource counts that are cheap to read live — atoms
//! (`AtomicCore::count_atoms`) and KBs (`DatabaseManager::list_databases`) —
//! are *never* stored here; they are read straight from the tenant database
//! at request time by the [`crate::quota`] guard, drift-free. `quota_usage`
//! holds only what can't be counted cheaply per request:
//!
//! - **`storage_bytes`** — the tenant database's on-disk size, recomputed
//!   periodically (it requires a maintenance-connection `pg_database_size`,
//!   not something to do on the hot path). This is the metric the storage
//!   enforcement arm gates on.
//! - the **advisory AI-credits** rollup (a later slice writes it for the "80%
//!   of allowance used" UX) — never gated on; OpenRouter enforces the real
//!   limit per managed key.
//!
//! # Period rollover (plan: "Period rollover")
//!
//! > AI allowances reset natively at OpenRouter (monthly, midnight UTC) — no
//! > rollover code needed for them. A 1-hour-cadence job inserts new
//! > `period_start` rows for the remaining metrics. Old rows kept for
//! > billing/audit.
//!
//! [`roll_over_period`] is that job. It opens the *current* monthly period
//! row (`period_start = first-of-month`, `value = 0`) for every active
//! account × rolled-over metric ([`ROLLED_OVER_METRICS`]), with
//! `ON CONFLICT DO NOTHING` so it is idempotent within a period and safe
//! across pods without any lock (the first pod to run it in a new month
//! inserts the rows; every later pod, this month or another, is a no-op
//! INSERT). Old period rows are left in place — they are the billing/audit
//! trail. AI allowances are deliberately absent: their reset is OpenRouter's,
//! not ours.
//!
//! # Storage-bytes enforcement (plan: enforcement table)
//!
//! > Periodic reaper | Storage bytes recompute | Week 1 warn; week 2 restrict
//! > writes; **no auto-delete**
//!
//! [`recompute_storage`] is that arm. For every active account it measures
//! the tenant database's `pg_database_size` (the honest on-disk figure,
//! including indexes and the vector store — see [`StoragePolicy`] for why
//! this measure and not summed atom lengths), upserts it into the current
//! period's `storage_bytes` row, and resolves the account's storage serving
//! state against its plan's `storage_bytes_limit`:
//!
//! - **under the limit** → `storage_state = 'active'`, `storage_over_since`
//!   cleared. A cleanup that brings a tenant back under lifts the restriction
//!   on the next recompute.
//! - **over the limit, inside the grace window** → `storage_state = 'warn'`,
//!   `storage_over_since` stamped on the *first* over-limit recompute (never
//!   reset while already over — the grace clock must not restart). Full
//!   access; a banner-worthy marker only.
//! - **over the limit, past the restrict window** → `storage_state =
//!   'restricted'`: writes blocked (the data-plane write-guard 402s
//!   mutations exactly as the dunning `read_only` path does), reads/exports
//!   allowed, data **retained — never deleted** (the cardinal rule).
//!
//! The grace windows ([`StoragePolicy::warn_after`] / `restrict_after`)
//! default to the plan's "week 1 warn, week 2 restrict": warn immediately on
//! going over, restrict after 7 days still over. They take an explicit `now`
//! so a test drives the ladder by manufacturing a past `storage_over_since`
//! via SQL — no real waits.
//!
//! Storage restriction is kept in its **own** `accounts.storage_state` column,
//! orthogonal to the dunning `billing_state` (migration 013 documents the
//! deviation from the plan's terse "reuse read_only"): the two have different
//! causes and recovery paths, and a payment-succeeded webhook must not
//! silently un-restrict a tenant that is over its storage ceiling. The
//! data-plane write-guard blocks writes when *either* is active.

use std::time::Duration;

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use sqlx::Connection;

use crate::control_plane::ControlPlane;
use crate::error::CloudError;
use crate::plans::{Plan, PlanRegistry};
use crate::provision::{tenant_db_name, ClusterConfig};

/// The `quota_usage.metric` value the storage arm reads and writes.
pub const STORAGE_BYTES_METRIC: &str = "storage_bytes";

/// The metrics the period-rollover job opens a fresh `period_start` row for
/// (plan: "inserts new `period_start` rows for the remaining metrics"). AI
/// credits are deliberately **absent** — OpenRouter resets the real allowance
/// natively, so there is no rollover code for it (the advisory AI rollup a
/// later slice may add resets with the same OpenRouter period and so doesn't
/// belong to *our* rollover either). Today that is `storage_bytes` alone; the
/// list is the seam for any future per-period non-AI metric.
pub const ROLLED_OVER_METRICS: &[&str] = &[STORAGE_BYTES_METRIC];

/// The serving restriction the storage ceiling imposes, stored as text in
/// `accounts.storage_state`. Orthogonal to [`crate::billing::dunning::BillingState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageState {
    /// Under the storage limit (the default).
    Active,
    /// Over the limit, inside the grace window: full access, banner marker.
    Warn,
    /// Over the limit past the grace window: writes blocked, data retained.
    Restricted,
}

impl StorageState {
    /// The text stored in `accounts.storage_state`.
    pub fn as_str(self) -> &'static str {
        match self {
            StorageState::Active => "active",
            StorageState::Warn => "warn",
            StorageState::Restricted => "restricted",
        }
    }

    /// Whether *mutating* requests are blocked while reads still pass — the
    /// storage analogue of the dunning read-only gate. The data-plane
    /// write-guard 402s writes when this is true.
    pub fn blocks_writes(self) -> bool {
        self == StorageState::Restricted
    }
}

/// Reconstruct the storage state from the raw column, defaulting an unknown
/// or absent value to [`StorageState::Active`] with a loud log — the
/// conservative reading (never block a tenant over one corrupt column),
/// mirroring [`crate::billing::dunning::billing_state_from_column`].
pub fn storage_state_from_column(value: &str) -> StorageState {
    match value {
        "active" => StorageState::Active,
        "warn" => StorageState::Warn,
        "restricted" => StorageState::Restricted,
        other => {
            tracing::warn!(other, "unknown storage_state; treating as active");
            StorageState::Active
        }
    }
}

/// The first day of `now`'s month, in UTC — the monthly `quota_usage`
/// `period_start`. Monthly granularity matches the AI allowance's monthly
/// OpenRouter reset (so the advisory rollup a later slice adds shares the
/// period) and the billing cycle; storage is a snapshot within the month,
/// updated in place by each recompute.
pub fn current_period_start(now: DateTime<Utc>) -> NaiveDate {
    let date = now.date_naive();
    NaiveDate::from_ymd_opt(date.year(), date.month(), 1)
        .expect("first of month is always a valid date")
}

/// Default cadence for the period-rollover loop (plan: "A 1-hour-cadence
/// job"). Exposed so the wiring and any CLI knob share one constant.
pub const DEFAULT_PERIOD_ROLLOVER_INTERVAL: Duration = Duration::from_secs(3600);

/// Open the current monthly period's `quota_usage` rows for every active
/// account × rolled-over metric (module docs). Idempotent and cross-pod safe:
/// `ON CONFLICT DO NOTHING` makes a re-run — same month, same pod or another
/// — insert nothing. Old period rows are retained (the billing/audit trail).
/// Returns how many fresh rows this call inserted (0 once the period is open).
///
/// `now` is explicit so a test can roll a *future* period forward without
/// waiting a month, and assert the second call inserts zero. The insert is a
/// single set-based statement (a cross join of active accounts and the metric
/// list), so the whole rollover is one round-trip regardless of fleet size.
pub async fn roll_over_period(
    control: &ControlPlane,
    now: DateTime<Utc>,
) -> Result<u64, CloudError> {
    let period_start = current_period_start(now);
    let metrics: Vec<String> = ROLLED_OVER_METRICS.iter().map(|m| m.to_string()).collect();
    let inserted = sqlx::query(
        "INSERT INTO quota_usage (account_id, period_start, metric, value, updated_at) \
         SELECT a.id, $1, m.metric, 0, NOW() \
           FROM accounts a \
           CROSS JOIN UNNEST($2::text[]) AS m(metric) \
          WHERE a.status = 'active' \
         ON CONFLICT (account_id, period_start, metric) DO NOTHING",
    )
    .bind(period_start)
    .bind(&metrics)
    .execute(control.pool())
    .await
    .map_err(CloudError::db("rolling over quota_usage period"))?
    .rows_affected();
    if inserted > 0 {
        tracing::info!(
            inserted,
            period_start = %period_start,
            "opened a new quota_usage period"
        );
    }
    Ok(inserted)
}

/// Tunables for the storage recompute arm. [`Default`] is the plan's "week 1
/// warn, week 2 restrict"; tests shrink the windows to drive the ladder on a
/// manufactured clock.
#[derive(Debug, Clone)]
pub struct StoragePolicy {
    /// How long after going over the limit before the `warn` marker is set.
    /// Zero by default: warn the instant a recompute finds an account over
    /// (the plan's "week 1 warn" is the *window* warn occupies, not a delay
    /// before it — the user should see the banner immediately).
    pub warn_after: Duration,
    /// How long an account may stay over the limit before writes are
    /// restricted (plan: "week 2 restrict"). Default 7 days, so the first
    /// week is warn-only and restriction lands at the start of week two.
    pub restrict_after: Duration,
}

impl Default for StoragePolicy {
    fn default() -> Self {
        Self {
            warn_after: Duration::ZERO,
            restrict_after: Duration::from_secs(7 * 24 * 60 * 60),
        }
    }
}

/// Default cadence for the storage-recompute loop. Hourly is ample for
/// day-granularity grace windows and keeps `pg_database_size` traffic low.
pub const DEFAULT_STORAGE_RECOMPUTE_INTERVAL: Duration = Duration::from_secs(3600);

/// What one [`recompute_storage`] pass changed, for logging and tests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StorageRecompute {
    /// Accounts measured and whose `storage_bytes` row was upserted.
    pub measured: u64,
    /// Accounts newly moved into `warn` this pass.
    pub moved_to_warn: u64,
    /// Accounts newly moved into `restricted` this pass (writes now blocked;
    /// data retained, never deleted).
    pub moved_to_restricted: u64,
    /// Accounts cleared back to `active` (a cleanup brought them under).
    pub cleared: u64,
    /// Per-account measure failures (an unreachable tenant database). The
    /// account is left at its prior state for the next pass — never
    /// fail-open (un-restricting on a transient error) or fail-closed
    /// (restricting an account we couldn't measure).
    pub errors: Vec<String>,
}

impl StorageRecompute {
    /// Whether the pass changed any serving state (for quiet logging).
    pub fn is_quiet(&self) -> bool {
        self.moved_to_warn == 0
            && self.moved_to_restricted == 0
            && self.cleared == 0
            && self.errors.is_empty()
    }
}

/// One active account's identity for the storage arm.
struct StorageTenant {
    account_id: String,
    plan_id: Option<String>,
    storage_state: String,
}

/// Recompute per-tenant storage bytes into `quota_usage` and advance each
/// account's `storage_state` against its plan's `storage_bytes_limit` (module
/// docs: warn → restrict, never delete). One pass over every active account:
/// measure `pg_database_size`, upsert the metric, resolve the state.
///
/// `now` is explicit so the warn/restrict horizons are testable by
/// manufacturing a past `storage_over_since` via SQL. A per-account error
/// (unreachable database, missing plan) is recorded and the account left at
/// its prior state — the pass never aborts and never mis-restricts.
pub async fn recompute_storage(
    control: &ControlPlane,
    cluster: &ClusterConfig,
    registry: &PlanRegistry,
    policy: &StoragePolicy,
    now: DateTime<Utc>,
) -> StorageRecompute {
    let mut summary = StorageRecompute::default();

    let tenants: Vec<StorageTenant> = match sqlx::query_as::<_, (String, Option<String>, String)>(
        "SELECT id, plan_id, storage_state FROM accounts WHERE status = 'active'",
    )
    .fetch_all(control.pool())
    .await
    {
        Ok(rows) => rows
            .into_iter()
            .map(|(account_id, plan_id, storage_state)| StorageTenant {
                account_id,
                plan_id,
                storage_state,
            })
            .collect(),
        Err(e) => {
            summary.errors.push(format!(
                "listing active accounts: {}",
                CloudError::db("")(e)
            ));
            return summary;
        }
    };

    // One maintenance connection for the whole pass — pg_database_size runs
    // against the cluster's `postgres` database, not each tenant database.
    let mut conn = match cluster.connect_maintenance().await {
        Ok(conn) => conn,
        Err(e) => {
            summary
                .errors
                .push(format!("connecting to cluster maintenance database: {e}"));
            return summary;
        }
    };

    for tenant in tenants {
        match recompute_one(control, &mut conn, registry, policy, now, &tenant).await {
            Ok(outcome) => {
                summary.measured += 1;
                match outcome {
                    StorageOutcome::MovedToWarn => summary.moved_to_warn += 1,
                    StorageOutcome::MovedToRestricted => summary.moved_to_restricted += 1,
                    StorageOutcome::Cleared => summary.cleared += 1,
                    StorageOutcome::Unchanged => {}
                }
            }
            Err(e) => summary.errors.push(format!(
                "recomputing storage for {}: {e}",
                tenant.account_id
            )),
        }
    }

    let _ = conn.close().await;
    if !summary.is_quiet() {
        tracing::info!(?summary, "storage recompute");
    }
    summary
}

/// What recomputing one account's storage decided.
enum StorageOutcome {
    MovedToWarn,
    MovedToRestricted,
    Cleared,
    Unchanged,
}

/// Measure one tenant's database size, upsert the `storage_bytes` metric, and
/// resolve its `storage_state` against the plan limit.
async fn recompute_one(
    control: &ControlPlane,
    conn: &mut sqlx::PgConnection,
    registry: &PlanRegistry,
    policy: &StoragePolicy,
    now: DateTime<Utc>,
    tenant: &StorageTenant,
) -> Result<StorageOutcome, CloudError> {
    let plan = resolve_plan(registry, tenant)?;

    // The account UUID is the tenant database name's source of truth — derive
    // it rather than join account_databases, so a measure works even mid an
    // interrupted-deletion window (the reaper owns that state, not us).
    let account_uuid = uuid::Uuid::parse_str(&tenant.account_id).map_err(|_| {
        CloudError::Invariant(format!("account id {} is not a UUID", tenant.account_id))
    })?;
    let db_name = tenant_db_name(account_uuid);

    // Honest on-disk size: pg_database_size includes the heap, indexes, and
    // the sqlite-vec-equivalent vector store — the real footprint the tenant
    // costs, not a summed atom-content approximation that ignores embeddings
    // and indexes (which dominate). The name is parameter-bound (it is a
    // value here, not interpolated DDL), so no identifier-quoting concern.
    let bytes: i64 = sqlx::query_scalar("SELECT pg_database_size($1)")
        .bind(&db_name)
        .fetch_one(&mut *conn)
        .await
        .map_err(CloudError::db("measuring tenant database size"))?;

    upsert_storage_metric(control, &tenant.account_id, now, bytes).await?;

    let over = plan.storage_bytes_limit.is_some_and(|limit| bytes > limit);
    let current = storage_state_from_column(&tenant.storage_state);
    apply_storage_state(control, &tenant.account_id, current, over, policy, now).await
}

/// Resolve the plan for a storage tenant, mirroring
/// [`PlanRegistry::for_account`]'s fallback (NULL plan_id → free) without the
/// extra round-trip (the storage list already read `plan_id`).
fn resolve_plan(registry: &PlanRegistry, tenant: &StorageTenant) -> Result<Plan, CloudError> {
    let plan_id = tenant
        .plan_id
        .clone()
        .unwrap_or_else(|| crate::plans::DEFAULT_PLAN_ID.to_string());
    registry.get(&plan_id).ok_or_else(|| {
        CloudError::Invariant(format!(
            "account {} references unknown plan {plan_id:?}",
            tenant.account_id
        ))
    })
}

/// Upsert the current period's `storage_bytes` value for `account_id`. The
/// rollover job opens the row at 0; this overwrites it with the measured
/// size. Idempotent and last-writer-wins — concurrent pods measuring within a
/// pass converge on whichever ran last (the size is a snapshot, not an
/// accumulator). Self-opening (`ON CONFLICT ... DO UPDATE`) so a recompute
/// that races ahead of the rollover still records its measurement.
async fn upsert_storage_metric(
    control: &ControlPlane,
    account_id: &str,
    now: DateTime<Utc>,
    bytes: i64,
) -> Result<(), CloudError> {
    let period_start = current_period_start(now);
    sqlx::query(
        "INSERT INTO quota_usage (account_id, period_start, metric, value, updated_at) \
         VALUES ($1, $2, $3, $4, NOW()) \
         ON CONFLICT (account_id, period_start, metric) \
         DO UPDATE SET value = EXCLUDED.value, updated_at = NOW()",
    )
    .bind(account_id)
    .bind(period_start)
    .bind(STORAGE_BYTES_METRIC)
    .bind(bytes)
    .execute(control.pool())
    .await
    .map_err(CloudError::db("upserting storage_bytes metric"))?;
    Ok(())
}

/// Resolve the new `storage_state` from whether the account is over the limit
/// and how long it has been over, and write it (with the grace anchor). Pure
/// decision, single guarded UPDATE — cross-pod safe (the conditional UPDATE
/// only ever advances or clears; concurrent pods converge).
async fn apply_storage_state(
    control: &ControlPlane,
    account_id: &str,
    current: StorageState,
    over: bool,
    policy: &StoragePolicy,
    now: DateTime<Utc>,
) -> Result<StorageOutcome, CloudError> {
    if !over {
        // Back under the limit (or unlimited): clear the marker and the grace
        // anchor. A cleanup lifts the restriction on the next recompute.
        if current == StorageState::Active {
            return Ok(StorageOutcome::Unchanged);
        }
        sqlx::query(
            "UPDATE accounts SET storage_state = 'active', storage_over_since = NULL WHERE id = $1",
        )
        .bind(account_id)
        .execute(control.pool())
        .await
        .map_err(CloudError::db("clearing storage_state"))?;
        return Ok(StorageOutcome::Cleared);
    }

    // Over the limit. Stamp the grace anchor on the FIRST over-limit recompute
    // (never reset while already over — the grace clock must not restart), and
    // compute how long the account has been over to decide warn vs restrict.
    let over_since: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT storage_over_since FROM accounts WHERE id = $1")
            .bind(account_id)
            .fetch_optional(control.pool())
            .await
            .map_err(CloudError::db("reading storage_over_since"))?
            .flatten();
    let over_since = match over_since {
        Some(since) => since,
        None => now, // first over-limit recompute: the clock starts now
    };
    let elapsed = (now - over_since).to_std().unwrap_or(Duration::ZERO);
    let target = if elapsed >= policy.restrict_after {
        StorageState::Restricted
    } else if elapsed >= policy.warn_after {
        StorageState::Warn
    } else {
        // Inside the (zero-by-default) pre-warn window: keep whatever marker
        // is set, but ensure at least the anchor is stamped.
        current
    };

    // Stamp the anchor (COALESCE keeps an existing one) and set the target
    // state in one statement.
    sqlx::query(
        "UPDATE accounts \
            SET storage_state = $2, \
                storage_over_since = COALESCE(storage_over_since, $3) \
          WHERE id = $1",
    )
    .bind(account_id)
    .bind(target.as_str())
    .bind(now)
    .execute(control.pool())
    .await
    .map_err(CloudError::db("advancing storage_state"))?;

    Ok(match (current, target) {
        (StorageState::Restricted, _) => StorageOutcome::Unchanged,
        (_, StorageState::Restricted) => StorageOutcome::MovedToRestricted,
        (StorageState::Warn, StorageState::Warn) => StorageOutcome::Unchanged,
        (_, StorageState::Warn) => StorageOutcome::MovedToWarn,
        _ => StorageOutcome::Unchanged,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_period_is_first_of_month_utc() {
        let mid = DateTime::parse_from_rfc3339("2026-06-13T14:41:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            current_period_start(mid),
            NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()
        );
        // Last instant of a month still lands on that month's first.
        let eom = DateTime::parse_from_rfc3339("2026-12-31T23:59:59Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            current_period_start(eom),
            NaiveDate::from_ymd_opt(2026, 12, 1).unwrap()
        );
    }

    #[test]
    fn storage_state_round_trips_and_gates() {
        for state in [
            StorageState::Active,
            StorageState::Warn,
            StorageState::Restricted,
        ] {
            assert_eq!(storage_state_from_column(state.as_str()), state);
        }
        assert!(StorageState::Restricted.blocks_writes());
        assert!(!StorageState::Warn.blocks_writes());
        assert!(!StorageState::Active.blocks_writes());
        // Unknown column degrades to active (never block over corruption).
        assert_eq!(storage_state_from_column("garbage"), StorageState::Active);
    }

    #[test]
    fn rolled_over_metrics_excludes_ai_credits() {
        // AI allowances reset natively at OpenRouter — never in our rollover.
        assert!(ROLLED_OVER_METRICS.contains(&STORAGE_BYTES_METRIC));
        assert!(!ROLLED_OVER_METRICS.iter().any(|m| m.contains("ai")));
    }

    #[test]
    fn default_storage_policy_is_week1_warn_week2_restrict() {
        let policy = StoragePolicy::default();
        assert_eq!(policy.warn_after, Duration::ZERO);
        assert_eq!(policy.restrict_after, Duration::from_secs(7 * 24 * 60 * 60));
    }

    #[test]
    fn recompute_summary_quiet_iff_no_state_change() {
        assert!(StorageRecompute::default().is_quiet());
        // A measure with no state change is still quiet.
        assert!(StorageRecompute {
            measured: 5,
            ..StorageRecompute::default()
        }
        .is_quiet());
        assert!(!StorageRecompute {
            moved_to_restricted: 1,
            ..StorageRecompute::default()
        }
        .is_quiet());
        assert!(!StorageRecompute {
            errors: vec!["x".into()],
            ..StorageRecompute::default()
        }
        .is_quiet());
    }
}
