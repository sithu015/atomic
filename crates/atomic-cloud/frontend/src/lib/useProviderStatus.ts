import { useCallback, useEffect, useState } from 'react';
import { ApiError, getProviderStatus } from './api';
import type { ProviderStatus } from './api';

export type ProviderStatusState =
  | { status: 'loading' }
  | { status: 'ready'; provider: ProviderStatus }
  | { status: 'error'; message: string };

/**
 * Load the full provider status (`GET /api/account/provider`). Exposes
 * `reload` so a successful save/activate/model write can refresh the view
 * (the new validation timestamp, the flipped active origin) without a page
 * reload. A `401` is handled by the API client (redirect to login).
 */
export function useProviderStatus(): {
  state: ProviderStatusState;
  reload: () => void;
} {
  const [state, setState] = useState<ProviderStatusState>({ status: 'loading' });
  const [nonce, setNonce] = useState(0);
  const reload = useCallback(() => setNonce((n) => n + 1), []);

  useEffect(() => {
    const controller = new AbortController();
    (async () => {
      try {
        const provider = await getProviderStatus(controller.signal);
        setState({ status: 'ready', provider });
      } catch (err) {
        if (err instanceof DOMException && err.name === 'AbortError') return;
        const message =
          err instanceof ApiError ? err.message : 'Couldn’t load your provider settings.';
        setState({ status: 'error', message });
      }
    })();
    return () => controller.abort();
  }, [nonce]);

  return { state, reload };
}
