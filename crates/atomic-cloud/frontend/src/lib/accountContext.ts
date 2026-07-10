import { useOutletContext } from 'react-router-dom';
import type { AccountOverview } from './api';

/**
 * Context the dashboard shell ({@link AccountShell}) hands to every
 * `/account/*` child route: the loaded overview and a `reload` to refresh it
 * after a mutation (a provider save changes the summary; a checkout changes
 * billing).
 *
 * Lives in its own module (not alongside the shell component) so the shell file
 * exports only components — keeping Vite's fast-refresh boundary clean.
 */
export interface AccountContext {
  overview: AccountOverview;
  reload: () => void;
}

/** Typed accessor for the shell's outlet context, for child route pages. */
export function useAccount(): AccountContext {
  return useOutletContext<AccountContext>();
}
