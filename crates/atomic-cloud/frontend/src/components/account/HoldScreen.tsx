import type { ReactNode } from 'react';
import { NodeGraphBackdrop } from '../NodeGraphBackdrop';
import { Logo } from '../ui/Logo';
import { Spinner } from '../ui/Spinner';
import { Button } from '../ui/Button';

interface HoldScreenProps {
  /** Show the spinner (a transient hold) vs. a static icon state. */
  busy?: boolean;
  title: ReactNode;
  children: ReactNode;
  /** Optional primary action (e.g. retry, manage billing). */
  action?: ReactNode;
}

/**
 * A branded full-viewport frame for the dashboard's non-content states —
 * provisioning, upgrading, suspended, and hard errors. Shares the landing/auth
 * node-graph backdrop so even a hold feels on-brand, never a blank page.
 */
export function HoldScreen({ busy = false, title, children, action }: HoldScreenProps) {
  return (
    <div className="relative flex min-h-dvh flex-col items-center justify-center overflow-hidden bg-bg-primary px-6 text-center text-text-primary">
      <NodeGraphBackdrop />
      <div className="relative w-full max-w-md">
        <Logo className="mx-auto mb-8 h-7" />
        <h1 className="font-display text-3xl leading-tight tracking-tight text-balance">
          {title}
        </h1>
        <div className="mt-4 text-text-secondary leading-relaxed">{children}</div>
        {busy && (
          <div className="mt-6 flex items-center justify-center gap-3 text-text-muted">
            <Spinner className="h-5 w-5 text-accent" />
            <span className="text-sm">This usually takes a few seconds.</span>
          </div>
        )}
        {action && <div className="mt-8 flex justify-center">{action}</div>}
      </div>
    </div>
  );
}

/** The shell's first-paint loading frame (before the overview resolves). */
export function DashboardLoading() {
  return (
    <HoldScreen busy title="Loading your account…">
      <p>Fetching your workspace details.</p>
    </HoldScreen>
  );
}

/** A retryable hard-error frame. */
export function DashboardError({ message, onRetry }: { message: string; onRetry: () => void }) {
  return (
    <HoldScreen
      title="We couldn’t load your account."
      action={<Button onClick={onRetry}>Try again</Button>}
    >
      <p>{message}</p>
    </HoldScreen>
  );
}
