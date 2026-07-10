-- Migration 013 — storage-bytes enforcement state (plan: "Observability,
-- quotas, billing" → "Quotas" → enforcement table: "Periodic reaper |
-- Storage bytes recompute | Week 1 warn; week 2 restrict writes; **no
-- auto-delete**"; Decisions log 2026-05-25 "Never auto-delete data").
--
-- Additive-only (tests/migration_lint.rs): two ADD COLUMN. The storage
-- restriction is kept in DEDICATED columns rather than reusing the dunning
-- `billing_state = 'read_only'` (a deliberate deviation from the plan's
-- terse "reuse read_only", recorded in the slice notes): the two are
-- orthogonal causes with different recovery paths. A payment-succeeded
-- webhook clears `billing_state` back to 'active'; it must NOT silently
-- un-restrict a tenant that is over its storage ceiling. Conversely a
-- storage cleanup must not rescue a payment-delinquent account. Keeping
-- `storage_state` separate lets the data-plane write-guard block writes when
-- EITHER cause is active and clear each independently.
--
--   active     — under the storage limit (the default; every existing row).
--   warn       — over the limit, inside the grace window (week 1): full
--                access, a banner-worthy marker only.
--   restricted — over the limit past the grace window (week 2+): writes
--                blocked, reads/exports allowed, data RETAINED (never
--                deleted). The data-plane write-guard 402s mutations exactly
--                as the dunning read_only path does.
--
-- `storage_over_since` anchors the grace window: stamped when an account
-- first goes over (cleared the moment a recompute finds it back under), the
-- storage reaper arm compares it against `now` minus the warn/restrict
-- horizons to advance warn → restricted. NULL when the account is under the
-- limit (the default, and every pre-migration row).
ALTER TABLE accounts
    ADD COLUMN IF NOT EXISTS storage_state      TEXT NOT NULL DEFAULT 'active';
ALTER TABLE accounts
    ADD COLUMN IF NOT EXISTS storage_over_since TIMESTAMPTZ;

-- Record this migration in the version table (the runner reads MAX(version)).
INSERT INTO schema_version (version) VALUES (13);
