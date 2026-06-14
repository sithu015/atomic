import { cn } from '../../lib/cn';
import { usageFraction } from '../../lib/format';

interface UsageMeterProps {
  used: number | null;
  limit: number | null;
  /** Accessible label for the meter (e.g. "Atoms used"). */
  label: string;
  className?: string;
}

/**
 * A thin progress meter for a usage-vs-limit metric. Renders an indeterminate
 * "unlimited" track when the limit is null, and warms from accent → amber → red
 * as the fraction approaches the ceiling. A real `<meter>`-style `progressbar`
 * for assistive tech.
 */
export function UsageMeter({ used, limit, label, className }: UsageMeterProps) {
  const fraction = usageFraction(used, limit);

  // Unlimited (or unknown limit): a calm, full accent-subtle track — there's
  // no ceiling to approach.
  if (fraction === null) {
    return (
      <div
        className={cn('h-2 w-full overflow-hidden rounded-full bg-bg-tertiary', className)}
        role="progressbar"
        aria-label={`${label}: unlimited`}
      >
        <div className="h-full w-full bg-accent-subtle" />
      </div>
    );
  }

  const pct = Math.round(fraction * 100);
  const tone =
    fraction >= 0.95 ? 'bg-red-500' : fraction >= 0.8 ? 'bg-amber-500' : 'bg-accent';

  return (
    <div
      className={cn('h-2 w-full overflow-hidden rounded-full bg-bg-tertiary', className)}
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={pct}
    >
      <div
        className={cn('h-full rounded-full transition-all duration-500', tone)}
        style={{ width: `${Math.max(pct, 2)}%` }}
      />
    </div>
  );
}
