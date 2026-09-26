import type { ReactNode } from 'react';
import { clsx } from 'clsx';

// Presentational building blocks shared by the Settings tabs.

export const ghostButton =
  'inline-flex items-center gap-2 rounded-lg border border-border bg-accent/40 px-3 py-1.5 text-xs font-medium text-foreground transition-colors hover:bg-accent disabled:opacity-50';

export function CubbyMark({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 64 64" className={className} aria-hidden="true">
      <path
        fill="#147ee8"
        d="M14 2h34c7.7 0 14 6.3 14 14v5H32c-6.6 0-12 5.4-12 12s5.4 12 12 12h30v3c0 7.7-6.3 14-14 14H14C6.3 62 0 55.7 0 48V16C0 8.3 6.3 2 14 2Z"
      />
      <rect x="42" y="25" width="20" height="16" rx="8" fill="#32aeb1" />
    </svg>
  );
}

export function PaneHeader({ title, subtitle }: { title: string; subtitle: string }) {
  return (
    <div>
      <h1 className="text-xl font-semibold tracking-tight">{title}</h1>
      <p className="mt-1 text-[13px] text-muted-foreground">{subtitle}</p>
    </div>
  );
}

export function SectionLabel({ children }: { children: ReactNode }) {
  return (
    <p className="mb-2 ml-1 text-[11px] font-semibold uppercase tracking-[0.09em] text-muted-foreground">
      {children}
    </p>
  );
}

export function SettingCard({ children }: { children: ReactNode }) {
  return (
    <div className="divide-y divide-border overflow-hidden rounded-xl border border-border bg-card">
      {children}
    </div>
  );
}

export function Row({
  title,
  desc,
  control,
  children,
}: {
  title?: ReactNode;
  desc?: ReactNode;
  control?: ReactNode;
  children?: ReactNode;
}) {
  if (children) {
    return (
      <div className="px-4 py-3.5">
        {(title || desc) && (
          <div className="mb-3">
            {title && <div className="text-sm font-medium">{title}</div>}
            {desc && <p className="mt-0.5 text-xs leading-snug text-muted-foreground">{desc}</p>}
          </div>
        )}
        {children}
      </div>
    );
  }
  return (
    <div className="flex items-center gap-4 px-4 py-3.5">
      <div className="min-w-0 flex-1">
        {title && <div className="text-sm font-medium">{title}</div>}
        {desc && <p className="mt-0.5 text-xs leading-snug text-muted-foreground">{desc}</p>}
      </div>
      {control && <div className="flex-shrink-0">{control}</div>}
    </div>
  );
}

export function Toggle({
  checked,
  onChange,
  disabled,
  label,
}: {
  checked: boolean;
  onChange: () => void;
  disabled?: boolean;
  label?: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={onChange}
      className={clsx(
        'relative h-6 w-11 flex-shrink-0 rounded-full transition-colors',
        checked ? 'bg-primary' : 'bg-accent',
        disabled && 'cursor-not-allowed opacity-40'
      )}
    >
      <span
        className={clsx(
          'absolute left-0.5 top-0.5 h-5 w-5 rounded-full bg-white shadow-sm transition-transform',
          checked ? 'translate-x-5' : 'translate-x-0'
        )}
      />
    </button>
  );
}

export function Segmented({
  value,
  onChange,
  options,
}: {
  value: string;
  onChange: (value: string) => void;
  options: { value: string; label: string }[];
}) {
  return (
    <div className="inline-flex gap-0.5 rounded-lg border border-border bg-accent/40 p-0.5">
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          onClick={() => onChange(option.value)}
          className={clsx(
            'rounded-md px-3 py-1.5 text-xs font-medium transition-colors',
            value === option.value
              ? 'bg-primary text-primary-foreground'
              : 'text-muted-foreground hover:text-foreground'
          )}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}
