import type { ReactNode } from 'react';
import { cn } from '../../lib/cn';

export type PillTone = 'neutral' | 'accent' | 'success' | 'warning' | 'error';

interface StatusPillProps {
  tone?: PillTone;
  children: ReactNode;
  className?: string;
  /** A small leading dot — useful for live/active status. */
  dot?: boolean;
}

const tones: Record<PillTone, { wrap: string; dot: string }> = {
  neutral: { wrap: 'bg-bg-tertiary text-text-secondary', dot: 'bg-text-muted' },
  accent: { wrap: 'bg-accent-subtle text-accent-dark', dot: 'bg-accent' },
  success: { wrap: 'bg-emerald-50 text-emerald-700', dot: 'bg-emerald-500' },
  warning: { wrap: 'bg-amber-50 text-amber-800', dot: 'bg-amber-500' },
  error: { wrap: 'bg-red-50 text-red-700', dot: 'bg-red-500' },
};

/** A small rounded-full status badge in the website's palette. */
export function StatusPill({ tone = 'neutral', children, className, dot = false }: StatusPillProps) {
  const t = tones[tone];
  return (
    <span
      className={cn(
        'inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-xs font-medium',
        t.wrap,
        className,
      )}
    >
      {dot && <span className={cn('h-1.5 w-1.5 rounded-full', t.dot)} aria-hidden="true" />}
      {children}
    </span>
  );
}
