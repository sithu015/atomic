-- Migration 010 — plans, quotas, and billing (plan: "Observability,
-- quotas, billing" → "Quotas" and "Billing"; Decisions log
-- 2026-06-09 "Billing v1 is subscription with included AI credits",
-- 2026-05-25 "Never auto-delete data for payment failure", "Trials: 14
-- days of paid tier on signup, no card required").
--
-- Additive-only (the lint in tests/migration_lint.rs enforces it): every
-- statement is CREATE TABLE / CREATE INDEX / ADD COLUMN / INSERT. The bare
-- legacy `accounts.plan` column from migration 001 is LEFT IN PLACE — this
-- migration adds `accounts.plan_id` alongside it and backfills, so an old
-- binary that still reads `accounts.plan` keeps working through the rolling
-- deploy. A later N+1 migration drops `accounts.plan` once no code reads it.

-- ── Plans ────────────────────────────────────────────────────────────────
-- The plan-tier catalogue. Seeded in-migration (below) and read by the
-- in-memory plan registry (crate::plans). Numbers for the free tier come
-- from the plan's "Free tier (defaults, product-tunable)" — 100 atoms,
-- $0.50/mo AI credits, 1 KB, 100 MB. The paid 'pro' tier's numbers are
-- PLACEHOLDERS (the plan leaves paid pricing to product); they are
-- deliberately generous-but-finite so quota enforcement has a non-free tier
-- to exercise. `atom_limit` / `kb_limit` are NULL = unlimited; the free
-- tier is finite on both.
CREATE TABLE IF NOT EXISTS plans (
    id                       TEXT PRIMARY KEY,
    name                     TEXT NOT NULL,
    monthly_price_cents      INT NOT NULL DEFAULT 0,
    atom_limit               INT,            -- NULL = unlimited
    ai_credits_monthly_cents INT NOT NULL DEFAULT 0,  -- managed-key allowance; OpenRouter enforces
    kb_limit                 INT,            -- NULL = unlimited
    storage_bytes_limit      BIGINT,         -- NULL = unlimited (advisory; reaper recompute)
    feature_flags            JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at               TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Seed the two v1 plans. ON CONFLICT DO NOTHING so a re-run (or a fresh
-- boot against a partially-migrated cluster) never duplicates or clobbers
-- operator-tuned numbers.
INSERT INTO plans
    (id, name, monthly_price_cents, atom_limit, ai_credits_monthly_cents, kb_limit, storage_bytes_limit, feature_flags)
VALUES
    -- Free tier: the plan's documented defaults.
    ('free', 'Free', 0, 100, 50, 1, 104857600, '{}'::jsonb),
    -- Pro tier: PLACEHOLDER numbers (see header). Unlimited atoms/KBs,
    -- a larger AI-credit allowance, 10 GB storage, frontier-model access
    -- as a feature flag (plan: "frontier models are a paid-tier feature
    -- flag").
    ('pro', 'Pro', 1200, NULL, 2000, NULL, 10737418240, '{"frontier_models": true}'::jsonb)
ON CONFLICT (id) DO NOTHING;

-- ── accounts.plan_id ───────────────────────────────────────────────────────
-- The FK column the quota enforcement reads. Additive: added alongside the
-- legacy bare `accounts.plan` column (NOT renamed, NOT dropped). The FK is
-- declared inline on ADD COLUMN — permitted by the lint (a column-level
-- REFERENCES on a brand-new column constrains no statement an old binary
-- makes, since no old statement names the new column). ON DELETE RESTRICT
-- matches the slice-1 CASCADE-safety convention: a plan referenced by any
-- account can never be silently deleted out from under it.
ALTER TABLE accounts
    ADD COLUMN IF NOT EXISTS plan_id TEXT REFERENCES plans(id) ON DELETE RESTRICT;

-- Backfill plan_id from the legacy `plan` column where it names a known
-- plan; everything else (NULL, or a legacy value with no matching row)
-- lands on 'free'. Runs only against rows the migration finds — new rows
-- get plan_id stamped at provision time (crate::provision).
UPDATE accounts
   SET plan_id = COALESCE((SELECT p.id FROM plans p WHERE p.id = accounts.plan), 'free')
 WHERE plan_id IS NULL;

CREATE INDEX IF NOT EXISTS idx_accounts_plan_id ON accounts (plan_id);

-- ── Billing serving state (dunning) ─────────────────────────────────────────
-- The dunning state machine (plan: "Plan transitions" — past_due → read_only
-- → suspended, data always retained). Kept as its own column so it is
-- orthogonal to `accounts.status` (which gates provisioning/active in
-- CloudAuth): a billing-delinquent account is still `status='active'`, but
-- its `billing_state` restricts or blocks serving.
--
--   active     — normal.
--   past_due   — a payment failed; full access continues (grace).
--   read_only  — 3 days past_due: writes blocked, reads/exports allowed.
--   suspended  — 14 days past_due: login/serving blocked, data RETAINED.
--
-- `past_due_since` anchors the time-driven transitions; the dunning reaper
-- (crate::billing::dunning) reads it and advances the state. NULL when not
-- past due. Default 'active' so every existing and new row is well-defined.
ALTER TABLE accounts
    ADD COLUMN IF NOT EXISTS billing_state  TEXT NOT NULL DEFAULT 'active';
ALTER TABLE accounts
    ADD COLUMN IF NOT EXISTS past_due_since TIMESTAMPTZ;

-- ── quota_usage ─────────────────────────────────────────────────────────────
-- Per-(account, period, metric) counters for metrics that aren't cheaply
-- countable live (plan: storage bytes + daily rollups; the AI-credits
-- counter is ADVISORY only — OpenRouter enforces the real limit). Atom and
-- KB counts are read LIVE from the tenant database at enforcement time
-- (cheap, strongly consistent) and never stored here. Old rows are kept for
-- billing/audit; a 1-hour-cadence job (later slice) inserts fresh
-- period_start rows.
CREATE TABLE IF NOT EXISTS quota_usage (
    account_id   TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    period_start DATE NOT NULL,
    metric       TEXT NOT NULL,
    value        BIGINT NOT NULL DEFAULT 0,
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (account_id, period_start, metric)
);

-- ── Stripe linkage ──────────────────────────────────────────────────────────
-- One Stripe customer / subscription per account (plan: "Billing" schema).
-- account_id is the PK (one customer per account); the Stripe ids are UNIQUE
-- so a webhook can look an account up by either direction. ON DELETE CASCADE
-- so an account hard-delete sweeps the linkage.
CREATE TABLE IF NOT EXISTS stripe_customers (
    account_id                TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    stripe_customer_id        TEXT UNIQUE NOT NULL,
    default_payment_method_id TEXT,
    created_at                TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS stripe_subscriptions (
    account_id              TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    stripe_subscription_id  TEXT UNIQUE NOT NULL,
    plan_id                 TEXT NOT NULL,
    status                  TEXT NOT NULL,
    current_period_start    TIMESTAMPTZ NOT NULL,
    current_period_end      TIMESTAMPTZ NOT NULL,
    cancel_at_period_end    BOOLEAN NOT NULL DEFAULT false,
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ── Plan transitions (audit) ────────────────────────────────────────────────
-- An append-only log of every plan/billing-state change with its trigger
-- (plan: "Plan transitions" table). Distinct from the user-facing
-- account_events log (that rides with the frontend slice); this is the
-- billing audit trail — what changed, from what, to what, why, and when.
CREATE TABLE IF NOT EXISTS plan_transitions (
    id            BIGSERIAL PRIMARY KEY,
    account_id    TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    from_plan_id  TEXT,
    to_plan_id    TEXT,
    trigger       TEXT NOT NULL,   -- 'checkout' | 'upgrade' | 'downgrade' | 'subscription_deleted' | 'dunning' | 'payment_failed' | 'payment_succeeded'
    detail        TEXT,
    occurred_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_plan_transitions_account ON plan_transitions (account_id, occurred_at);

-- Record this migration in the version table (the runner reads MAX(version)).
INSERT INTO schema_version (version) VALUES (10);
