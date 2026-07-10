/** Small presentation helpers for the dashboard. Pure, dependency-free. */

/** Compact integer formatting (`1,234`), tolerant of null → em dash. */
export function formatCount(value: number | null | undefined): string {
  if (value === null || value === undefined) return '—';
  return value.toLocaleString('en-US');
}

/** A `used / limit` string, where a null limit reads as unlimited. */
export function formatUsage(
  used: number | null | undefined,
  limit: number | null | undefined,
): string {
  const usedStr = formatCount(used);
  if (limit === null || limit === undefined) return `${usedStr} / unlimited`;
  return `${usedStr} / ${formatCount(limit)}`;
}

/** A 0–1 fraction of usage against a limit, or null when unlimited/unknown. */
export function usageFraction(
  used: number | null | undefined,
  limit: number | null | undefined,
): number | null {
  if (limit === null || limit === undefined || limit <= 0) return null;
  if (used === null || used === undefined) return null;
  return Math.min(1, Math.max(0, used / limit));
}

/** Cents → a `$0.50` style dollar string. */
export function formatCents(cents: number | null | undefined): string {
  if (cents === null || cents === undefined) return '—';
  return new Intl.NumberFormat('en-US', {
    style: 'currency',
    currency: 'USD',
  }).format(cents / 100);
}

/** Plain USD dollars → `$1.23`. */
export function formatUsd(amount: number | null | undefined): string {
  if (amount === null || amount === undefined) return '—';
  return new Intl.NumberFormat('en-US', {
    style: 'currency',
    currency: 'USD',
  }).format(amount);
}

/**
 * Whole days from now until `iso` (negative if already past), or null when the
 * timestamp is missing/unparseable. Used for the trial countdown.
 */
export function daysUntil(iso: string | null | undefined): number | null {
  if (!iso) return null;
  const then = Date.parse(iso);
  if (Number.isNaN(then)) return null;
  const ms = then - Date.now();
  return Math.ceil(ms / (1000 * 60 * 60 * 24));
}

/** A short absolute date (`Jun 14, 2026`), or em dash for missing/invalid. */
export function formatDate(iso: string | null | undefined): string {
  if (!iso) return '—';
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return '—';
  return new Date(t).toLocaleDateString('en-US', {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  });
}

/**
 * A coarse "x ago" relative label for the validation/last-used timestamps.
 * Intentionally simple — minutes/hours/days/the date — since these update
 * rarely and exactness isn't useful.
 */
export function formatRelative(iso: string | null | undefined): string {
  if (!iso) return 'never';
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return 'never';
  const seconds = Math.round((Date.now() - t) / 1000);
  if (seconds < 60) return 'just now';
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} hr${hours === 1 ? '' : 's'} ago`;
  const days = Math.round(hours / 24);
  if (days < 30) return `${days} day${days === 1 ? '' : 's'} ago`;
  return formatDate(iso);
}
