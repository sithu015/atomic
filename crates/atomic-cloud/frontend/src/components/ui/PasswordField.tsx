import { forwardRef, useId, useState } from 'react';
import type { InputHTMLAttributes, ReactNode } from 'react';
import { Eye, EyeOff } from 'lucide-react';
import { cn } from '../../lib/cn';

interface PasswordFieldProps
  extends Omit<InputHTMLAttributes<HTMLInputElement>, 'type'> {
  label: string;
  error?: string | null;
  help?: ReactNode;
  wrapperClassName?: string;
}

const inputBase =
  'w-full px-3.5 py-2.5 pr-11 text-base text-text-primary bg-bg-white rounded-xl border ' +
  'placeholder:text-text-muted transition-colors font-mono ' +
  'focus:outline-none focus-visible:outline-none ' +
  'disabled:opacity-60 disabled:cursor-not-allowed';

/**
 * A masked secret input (API keys) with a reveal toggle. Mirrors {@link Field}
 * but defaults to `type=password`, uses the mono face for key legibility, and
 * exposes a show/hide eye that flips `type` without ever logging or persisting
 * the value. `autoComplete` defaults to `off` — a key is not a saved password.
 */
export const PasswordField = forwardRef<HTMLInputElement, PasswordFieldProps>(
  ({ label, error, help, id, wrapperClassName, className, autoComplete = 'off', ...props }, ref) => {
    const autoId = useId();
    const inputId = id ?? autoId;
    const errorId = `${inputId}-error`;
    const helpId = `${inputId}-help`;
    const describedBy = error ? errorId : help ? helpId : undefined;
    const [revealed, setRevealed] = useState(false);

    return (
      <div className={cn('flex flex-col gap-1.5', wrapperClassName)}>
        <label htmlFor={inputId} className="text-sm font-medium text-text-primary">
          {label}
        </label>
        <div className="relative">
          <input
            ref={ref}
            id={inputId}
            type={revealed ? 'text' : 'password'}
            autoComplete={autoComplete}
            aria-invalid={error ? true : undefined}
            aria-describedby={describedBy}
            className={cn(
              inputBase,
              error ? 'border-red-400 focus:border-red-500' : 'border-border focus:border-accent',
              className,
            )}
            {...props}
          />
          <button
            type="button"
            onClick={() => setRevealed((r) => !r)}
            aria-label={revealed ? 'Hide key' : 'Show key'}
            aria-pressed={revealed}
            className="absolute inset-y-0 right-2.5 my-auto flex h-7 w-7 items-center justify-center rounded-md text-text-muted hover:text-text-primary hover:bg-bg-tertiary/60 transition-colors focus-visible:outline-2"
          >
            {revealed ? (
              <EyeOff className="w-4 h-4" aria-hidden="true" />
            ) : (
              <Eye className="w-4 h-4" aria-hidden="true" />
            )}
          </button>
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

PasswordField.displayName = 'PasswordField';
