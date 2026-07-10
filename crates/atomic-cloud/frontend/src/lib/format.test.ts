import { describe, it, expect, vi, afterEach } from 'vitest';
import {
  daysUntil,
  formatCents,
  formatUsage,
  usageFraction,
} from './format';

describe('format helpers', () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it('formatUsage reads a null limit as unlimited', () => {
    expect(formatUsage(12, 100)).toBe('12 / 100');
    expect(formatUsage(12, null)).toBe('12 / unlimited');
    expect(formatUsage(null, 100)).toBe('— / 100');
  });

  it('usageFraction clamps to [0,1] and is null when unlimited/unknown', () => {
    expect(usageFraction(50, 100)).toBe(0.5);
    expect(usageFraction(150, 100)).toBe(1); // clamped
    expect(usageFraction(10, null)).toBeNull();
    expect(usageFraction(null, 100)).toBeNull();
    expect(usageFraction(10, 0)).toBeNull(); // no divide-by-zero
  });

  it('formatCents renders a dollar string', () => {
    expect(formatCents(50)).toBe('$0.50');
    expect(formatCents(1234)).toBe('$12.34');
    expect(formatCents(null)).toBe('—');
  });

  it('daysUntil counts whole days from a fixed now', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-06-14T00:00:00Z'));
    // 3 days out.
    expect(daysUntil('2026-06-17T00:00:00Z')).toBe(3);
    // Already past → negative.
    expect(daysUntil('2026-06-13T00:00:00Z')).toBe(-1);
    expect(daysUntil(null)).toBeNull();
    expect(daysUntil('not-a-date')).toBeNull();
  });
});
