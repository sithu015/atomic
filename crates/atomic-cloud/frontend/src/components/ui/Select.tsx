import { forwardRef, useId } from 'react';
import type { ReactNode, SelectHTMLAttributes } from 'react';
import { ChevronDown } from 'lucide-react';
import { cn } from '../../lib/cn';

export interface SelectOption {
  value: string;
  label: string;
}

interface SelectProps extends Omit<SelectHTMLAttributes<HTMLSelectElement>, 'children'> {
  label: string;
  options: SelectOption[];
  /** Inline error; sets invalid styling + `aria-invalid`. */
  error?: string | null;
  /** Quiet helper beneath the control when there's no error. */
  help?: ReactNode;
  /** A placeholder option rendered first, disabled and unselectable once a
   * real value is chosen. */
  placeholder?: string;
  wrapperClassName?: string;
}

const selectBase =
  'w-full appearance-none px-3.5 py-2.5 pr-10 text-base text-text-primary bg-bg-white ' +
  'rounded-xl border transition-colors focus:outline-none focus-visible:outline-none ' +
  'disabled:opacity-60 disabled:cursor-not-allowed';

/**
 * A labelled native `<select>`, styled to match the website's {@link Field}:
 * warm-paper surface, accent focus border, a custom chevron. Native on purpose
 * — keyboard, mobile pickers, and accessibility come for free, and the option
 * set here is small and curated.
 */
export const Select = forwardRef<HTMLSelectElement, SelectProps>(
  (
    { label, options, error, help, placeholder, id, wrapperClassName, className, value, ...props },
    ref,
  ) => {
    const autoId = useId();
    const selectId = id ?? autoId;
    const errorId = `${selectId}-error`;
    const helpId = `${selectId}-help`;
    const describedBy = error ? errorId : help ? helpId : undefined;

    return (
      <div className={cn('flex flex-col gap-1.5', wrapperClassName)}>
        <label htmlFor={selectId} className="text-sm font-medium text-text-primary">
          {label}
        </label>
        <div className="relative">
          <select
            ref={ref}
            id={selectId}
            value={value}
            aria-invalid={error ? true : undefined}
            aria-describedby={describedBy}
            className={cn(
              selectBase,
              error
                ? 'border-red-400 focus:border-red-500'
                : 'border-border focus:border-accent',
              className,
            )}
            {...props}
          >
            {placeholder !== undefined && (
              <option value="" disabled>
                {placeholder}
              </option>
            )}
            {options.map((opt) => (
              <option key={opt.value} value={opt.value}>
                {opt.label}
              </option>
            ))}
          </select>
          <ChevronDown
            className="pointer-events-none absolute inset-y-0 right-3 my-auto w-4 h-4 text-text-muted"
            aria-hidden="true"
          />
        </div>
        {error ? (
          <p id={errorId} role="alert" className="text-sm text-red-600">
            {error}
          </p>
        ) : help ? (
          <p id={helpId} className="text-sm text-text-muted">
            {help}
          </p>
        ) : null}
      </div>
    );
  },
);

Select.displayName = 'Select';
