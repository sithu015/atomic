import { useCallback, useEffect, useState } from 'react';
import { ApiError, getOverview } from './api';
import type { AccountOverview } from './api';

/**
 * The dashboard's load state for the account overview. The non-`ready`
 * variants map the structured CloudAuth gates the overview fetch can hit:
 *
 * - `provisioning` / `upgrading` — the 503 `account_provisioning` /
 *   `account_upgrading` holds (the account's tenant DB is being set up or
 *   migrated). The shell shows a friendly "setting up / upgrading" screen and
 *   the hook auto-retries.
 * - `suspended` — the 402 `account_suspended` gate (CloudAuth blocks a
 *   suspended account before the handler runs). The shell shows a blocking
 *   notice + the billing upgrade link.
 * - A `401` never reaches here: the API client redirects to the app-host login.
 */
export type OverviewState =
  | { status: 'loading' }
  | { status: 'ready'; overview: AccountOverview }
  | { status: 'provisioning' }
  | { status: 'upgrading' }
  | { status: 'suspended'; upgradeUrl: string | null }
  | { status: 'error'; message: string };

/** How long the hold states wait before auto-retrying the overview. */
const HOLD_RETRY_MS = 6000;

/**
 * Load the account overview, mapping the structured cloud gates into
 * {@link OverviewState}. Auto-retries the provisioning/upgrading holds; exposes
 * `reload` for manual refresh after a settings change.
 */
export function useOverview(): { state: OverviewState; reload: () => void } {
  const [state, setState] = useState<OverviewState>({ status: 'loading' });
  // Bumping this re-runs the effect (manual reload, or a scheduled hold retry).
  const [nonce, setNonce] = useState(0);

  const reload = useCallback(() => setNonce((n) => n + 1), []);

  useEffect(() => {
    const controller = new AbortController();
    let holdTimer: ReturnType<typeof setTimeout> | undefined;

    (async () => {
      try {
        const overview = await getOverview(controller.signal);
        setState({ status: 'ready', overview });
      } catch (err) {
        if (err instanceof DOMException && err.name === 'AbortError') return;
        if (err instanceof ApiError) {
          if (err.code === 'account_provisioning') {
            setState({ status: 'provisioning' });
            holdTimer = setTimeout(() => setNonce((n) => n + 1), HOLD_RETRY_MS);
            return;
          }
          if (err.code === 'account_upgrading') {
            setState({ status: 'upgrading' });
            holdTimer = setTimeout(() => setNonce((n) => n + 1), HOLD_RETRY_MS);
            return;
          }
          if (err.code === 'account_suspended' || err.status === 402) {
            setState({ status: 'suspended', upgradeUrl: err.field('upgrade_url') });
            return;
          }
          // A 401 has already navigated away; anything else is a real error.
          setState({ status: 'error', message: err.message });
          return;
        }
        setState({ status: 'error', message: 'Something went wrong loading your account.' });
      }
    })();

    return () => {
      controller.abort();
      if (holdTimer) clearTimeout(holdTimer);
    };
  }, [nonce]);

  return { state, reload };
}
