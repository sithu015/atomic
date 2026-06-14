import { CreditCard, ExternalLink } from 'lucide-react';
import { useAccount } from '../../lib/accountContext';
import { Card } from '../../components/ui/Card';
import { StatusPill } from '../../components/ui/StatusPill';
import type { PillTone } from '../../components/ui/StatusPill';
import type { BillingState } from '../../lib/api';
import { daysUntil, formatCents, formatDate } from '../../lib/format';

/**
 * Billing = Stripe portal + status. We render the plan, the serving state, and
 * the trial/dunning context, then hand off to Stripe for everything money:
 * "Manage billing" navigates to the portal route (a server 302 into the
 * Customer Portal), and the upgrade CTA navigates to the checkout route. No
 * card entry, invoice tables, or Stripe Elements live here — Stripe owns that.
 *
 * The portal/checkout routes are `GET` redirects, so these are plain
 * navigations (`window.location`), not fetches; a full-page redirect to Stripe
 * is exactly the intended flow.
 */
export function Billing() {
  const { overview } = useAccount();
  const { plan, usage } = overview;
  const status = billingDescriptor(overview.billing_state);
  const trialDays = daysUntil(overview.trial_ends_at);
  // The free tier (and the trial of it) can upgrade; a paid plan manages
  // through the portal. `pro` is the seeded purchasable tier.
  const canUpgrade = plan.id === 'free' || overview.billing_state === 'trialing';

  return (
    <div className="space-y-8">
      <header>
        <p className="text-xs font-medium uppercase tracking-wide text-text-muted">Billing</p>
        <h1 className="mt-1 font-display text-3xl tracking-tight md:text-4xl">
          Plan &amp; <span className="italic">billing.</span>
        </h1>
        <p className="mt-2 text-text-secondary">
          Manage your subscription and payment method. Payments are handled
          securely by Stripe.
        </p>
      </header>

      <Card>
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div>
            <p className="text-sm text-text-muted">Current plan</p>
            <p className="mt-1 font-display text-2xl tracking-tight">{plan.name}</p>
            <p className="mt-1 text-sm text-text-muted">
              Monthly AI allowance: {formatCents(usage.ai_credits_monthly_cents)}
            </p>
          </div>
          <StatusPill tone={status.tone} dot>
            {status.label}
          </StatusPill>
        </div>

        {overview.billing_state === 'trialing' && (
          <p className="mt-4 rounded-lg bg-accent-subtle px-3 py-2 text-sm text-accent-dark">
            {trialDays !== null && trialDays > 0
              ? `Your free trial ends in ${trialDays} day${trialDays === 1 ? '' : 's'}`
              : 'Your free trial ends today'}
            {overview.trial_ends_at ? ` — on ${formatDate(overview.trial_ends_at)}.` : '.'} Add
            billing now to keep the paid tier without interruption.
          </p>
        )}

        {overview.billing_state === 'read_only' && (
          <p className="mt-4 rounded-lg bg-amber-50 px-3 py-2 text-sm text-amber-800">
            Your account is read-only because a payment is overdue. Your data is
            safe — update billing to restore writes.
          </p>
        )}

        <div className="mt-6 flex flex-wrap gap-3">
          {canUpgrade && (
            <a
              href="/api/billing/checkout?plan=pro"
              className="inline-flex items-center gap-2.5 rounded-xl bg-accent px-7 py-3.5 text-base font-medium text-white transition-all hover:bg-accent-dark hover:shadow-lg hover:shadow-accent/20 focus-visible:outline-2"
            >
              Upgrade plan
            </a>
          )}
          <a
            href="/api/billing/portal"
            className="inline-flex items-center gap-2.5 rounded-xl border border-border bg-bg-white px-7 py-3.5 text-base font-medium text-text-primary transition-all hover:border-accent/30 hover:bg-accent-subtle/50 focus-visible:outline-2"
          >
            <CreditCard className="h-5 w-5" strokeWidth={1.5} aria-hidden="true" />
            Manage billing
            <ExternalLink className="h-4 w-4 text-text-muted" aria-hidden="true" />
          </a>
        </div>
        <p className="mt-3 text-xs text-text-muted">
          “Manage billing” opens the Stripe Customer Portal, where you can update
          your card, view invoices, or cancel. Your knowledge base is never
          deleted for non-payment — it’s retained until you say otherwise.
        </p>
      </Card>
    </div>
  );
}

function billingDescriptor(state: BillingState): { tone: PillTone; label: string } {
  switch (state) {
    case 'active':
      return { tone: 'success', label: 'Active' };
    case 'trialing':
      return { tone: 'accent', label: 'Trial' };
    case 'past_due':
      return { tone: 'warning', label: 'Past due' };
    case 'read_only':
      return { tone: 'warning', label: 'Read-only' };
    case 'suspended':
      return { tone: 'error', label: 'Suspended' };
  }
}
