import { ArrowUpRight, LogOut } from 'lucide-react';
import { Logo } from '../ui/Logo';
import { appHostLoginUrl, tenantRootUrl } from '../../lib/host';

interface AccountTopbarProps {
  /** The account's subdomain, shown beside the logo (null while loading). */
  subdomain: string | null;
}

/**
 * The fixed top bar of the authenticated dashboard: the brand, the account's
 * subdomain, a link out to the product knowledge base (the tenant root, an
 * nginx-served app separate from this dashboard), and sign-out.
 *
 * Sign-out navigates to the app-host login. The session cookie is `HttpOnly`
 * (the browser can't clear it) and this slice ships no logout endpoint, so
 * this is a soft sign-out — it returns the user to the login surface; a true
 * server-side "sign out everywhere" is a later slice.
 */
export function AccountTopbar({ subdomain }: AccountTopbarProps) {
  return (
    <header className="fixed top-0 left-0 right-0 z-50 h-16 backdrop-blur-md bg-bg-primary/80 border-b border-border-light">
      <div className="max-w-6xl mx-auto h-full px-6 flex items-center justify-between gap-4">
        <div className="flex items-center min-w-0">
          <Logo className="h-6 shrink-0" />
          {subdomain && (
            <span className="ml-3 hidden truncate text-sm text-text-muted sm:inline">
              <span className="text-text-secondary font-medium">{subdomain}</span>
              <span className="text-text-muted">’s workspace</span>
            </span>
          )}
        </div>

        <nav className="flex items-center gap-1 sm:gap-2" aria-label="Account actions">
          <a
            href={tenantRootUrl()}
            className="inline-flex items-center gap-1.5 rounded-lg px-3 py-2 text-sm font-medium text-text-secondary transition-colors hover:bg-accent-subtle/50 hover:text-accent-dark focus-visible:outline-2"
          >
            <span className="hidden sm:inline">Open knowledge base</span>
            <span className="sm:hidden">Knowledge base</span>
            <ArrowUpRight className="h-4 w-4" aria-hidden="true" />
          </a>
          <a
            href={appHostLoginUrl()}
            className="inline-flex items-center gap-1.5 rounded-lg px-3 py-2 text-sm font-medium text-text-secondary transition-colors hover:bg-bg-tertiary/60 hover:text-text-primary focus-visible:outline-2"
          >
            <LogOut className="h-4 w-4" aria-hidden="true" />
            <span className="hidden sm:inline">Sign out</span>
          </a>
        </nav>
      </div>
    </header>
  );
}
