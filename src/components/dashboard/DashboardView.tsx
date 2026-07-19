import { useEffect } from 'react';
import { dashboardWidgets } from './registry';
import { useWikiStore } from '../../stores/wiki';
import { useAtomsStore } from '../../stores/atoms';
import { WelcomeView } from './WelcomeView';
import { DemoIntroCard } from './DemoIntroCard';

export function DashboardView() {
  const fetchAllArticles = useWikiStore(s => s.fetchAllArticles);
  const atomCount = useAtomsStore(s => s.atoms.length);
  const initialLoadComplete = useAtomsStore(s => s.initialLoadComplete);

  useEffect(() => {
    // Kick off wiki data on dashboard mount. The call is idempotent — safe to
    // fire every time the user lands on the dashboard so widgets stay fresh.
    fetchAllArticles();
  }, [fetchAllArticles]);

  // Brand-new users land on an empty grid of widgets, which reads as broken.
  // Defer to the welcome view until atoms exist. Gated on initialLoadComplete
  // so we don't flash the welcome state during cold start before the first
  // fetch settles.
  if (initialLoadComplete && atomCount === 0) {
    return <WelcomeView />;
  }

  return (
    <div className="h-full overflow-y-auto scrollbar-auto-hide">
      <div className="mx-auto max-w-4xl px-6 pt-10 pb-16 md:px-10 md:pt-14 md:pb-20">
        <DemoIntroCard />
        <div className="grid grid-cols-1 md:grid-cols-2 gap-x-10 gap-y-10 md:gap-y-12">
          {dashboardWidgets.map(({ id, span, Component }) => (
            <div key={id} className={span === 'full' ? 'md:col-span-2' : ''}>
              <Component />
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
