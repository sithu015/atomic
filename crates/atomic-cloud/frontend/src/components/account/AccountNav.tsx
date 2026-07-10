import { NavLink } from 'react-router-dom';
import {
  LayoutDashboard,
  Cpu,
  CreditCard,
  Plug,
  TriangleAlert,
} from 'lucide-react';
import type { LucideIcon } from 'lucide-react';
import { cn } from '../../lib/cn';

interface NavItem {
  to: string;
  label: string;
  Icon: LucideIcon;
  /** Match only the exact path (the index route). */
  end?: boolean;
}

const ITEMS: NavItem[] = [
  { to: '/account', label: 'Overview', Icon: LayoutDashboard, end: true },
  { to: '/account/provider', label: 'AI provider', Icon: Cpu },
  { to: '/account/billing', label: 'Billing', Icon: CreditCard },
  { to: '/account/mcp', label: 'MCP', Icon: Plug },
  { to: '/account/danger', label: 'Account', Icon: TriangleAlert },
];

/**
 * The dashboard's primary navigation. Renders as a vertical rail on desktop
 * (≥ lg) and a horizontal, scrollable tab strip on smaller screens, so the
 * five sections are always reachable without a hamburger.
 */
export function AccountNav() {
  return (
    <nav aria-label="Account sections" className="lg:sticky lg:top-24">
      <ul className="flex gap-1 overflow-x-auto lg:flex-col lg:overflow-visible">
        {ITEMS.map(({ to, label, Icon, end }) => (
          <li key={to} className="shrink-0">
            <NavLink
              to={to}
              end={end}
              className={({ isActive }) =>
                cn(
                  'group flex items-center gap-2.5 rounded-lg px-3 py-2 text-sm font-medium transition-colors whitespace-nowrap focus-visible:outline-2',
                  isActive
                    ? 'bg-accent-subtle text-accent-dark'
                    : 'text-text-secondary hover:bg-bg-tertiary/60 hover:text-text-primary',
                )
              }
            >
              {({ isActive }) => (
                <>
                  <Icon
                    className={cn(
                      'h-4 w-4 shrink-0',
                      isActive ? 'text-accent' : 'text-text-muted group-hover:text-text-secondary',
                    )}
                    strokeWidth={1.75}
                    aria-hidden="true"
                  />
                  {label}
                </>
              )}
            </NavLink>
          </li>
        ))}
      </ul>
    </nav>
  );
}
