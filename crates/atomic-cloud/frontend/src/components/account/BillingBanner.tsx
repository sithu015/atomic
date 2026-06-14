import { Banner } from '../ui/Banner';
import type { BannerTone } from '../ui/Banner';
import type { BillingState } from '../../lib/api';
import { daysUntil } from '../../lib/format';

interface BillingBannerProps {
  billingState: BillingState;
  trialEndsAt: string | null;
}

/**
 * A global banner driven by the account's billing serving state. `active`
 * renders nothing; the trial and dunning states each get a tone-appropriate
 * notice, with a "Manage billing" action that navigates to the portal route
 * (a server 302 into Stripe's Customer Portal) for the states where paying
 * resolves the issue.
 *
 * The `suspended` state never reaches here — CloudAuth blocks a suspended
 * account before the overview loads, and the shell renders a dedicated
 * blocking screen — but it's handled defensively for completeness.
 */
export function BillingBanner({ billingState, trialEndsAt }: BillingBannerProps) {
  const notice = describe(billingState, trialEndsAt);
  if (!notice) return null;

  return (
    <Banner
      tone={notice.tone}
      title={notice.title}
      action={
        notice.action ? (
          <a
            href="/api/billing/portal"
            className="inline-flex items-center rounded-lg bg-bg-white/70 px-3 py-1.5 text-sm font-medium text-text-primary ring-1 ring-inset ring-current/20 transition-colors hover:bg-bg-white focus-visible:outline-2"
          >
            Manage billing
          </a>
        ) : undefined
      }
    >
      {notice.body}
    </Banner>
  );
}

interface Notice {
  tone: BannerTone;
  title: string;
  body: string;
  action: boolean;
}

function describe(state: BillingState, trialEndsAt: string | null): Notice | null {
  switch (state) {
    case 'active':
      return null;
    case 'trialing': {
      const days = daysUntil(trialEndsAt);
      const left =
        days === null
          ? 'Your free trial is active.'
          : days <= 0
            ? 'Your free trial ends today.'
            : `${days} day${days === 1 ? '' : 's'} left in your free trial.`;
      return {
        tone: 'info',
        title: left,
        body: 'You have full access to the paid tier. Add billing before it ends to keep your provider, higher limits, and AI allowance.',
        action: true,
      };
    }
    case 'past_due':
      return {
        tone: 'warning',
        title: 'Your last payment didn’t go through.',
        body: 'You still have full access for now. Update your payment method to avoid an interruption.',
        action: true,
      };
    case 'read_only':
      return {
        tone: 'warning',
        title: 'Your account is read-only.',
        body: 'A payment is overdue, so writes are paused — your data is safe and fully readable. Update billing to restore full access.',
        action: true,
      };
    case 'suspended':
      return {
        tone: 'error',
        title: 'Your account is suspended.',
        body: 'Serving is paused for non-payment. Your data is retained — update billing to restore access.',
        action: true,
      };
  }
}
