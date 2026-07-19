import { demoSignupUrl } from '../../lib/transport';
import { useUIStore } from '../../stores/ui';
import { DEMO_INTRO_REOPEN_EVENT } from '../dashboard/DemoIntroCard';

/**
 * The slim strip shown to anonymous visitors on the cloud's public demo
 * instance (rendered only when `isDemoInstance()`): names what they're
 * looking at and carries the one CTA. The server-side whitelist is the
 * real enforcement; this is orientation. "What is this?" re-opens the
 * dashboard's dismissed intro card — the rope for visitors who land deep
 * (a shared wiki URL) or dismissed the card before reading it.
 */
export function DemoBanner() {
  const setViewMode = useUIStore(s => s.setViewMode);

  const explain = () => {
    setViewMode('dashboard');
    window.dispatchEvent(new Event(DEMO_INTRO_REOPEN_EVENT));
  };

  return (
    <div className="flex-shrink-0 flex items-center justify-center gap-3 px-4 py-1.5 bg-[var(--color-accent)] text-white text-sm">
      <span>
        Live demo — read-only, seeded with ~130 AI/ML papers. The wikis, tags,
        and digest are Atomic's own work.
      </span>
      <button
        onClick={explain}
        className="flex-shrink-0 underline decoration-white/50 underline-offset-2 hover:decoration-white whitespace-nowrap"
      >
        What is this?
      </button>
      <a
        href={demoSignupUrl()}
        className="flex-shrink-0 px-2.5 py-0.5 rounded-md bg-white/15 hover:bg-white/25 font-medium whitespace-nowrap"
      >
        Get your own →
      </a>
    </div>
  );
}
