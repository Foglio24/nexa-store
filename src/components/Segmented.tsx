import type { ReactNode } from 'react';

export interface SegmentOption<T extends string> { value: T; label: ReactNode; disabled?: boolean }

export default function Segmented<T extends string>({ value, options, onChange, label }: {
  value: T; options: SegmentOption<T>[]; onChange: (value: T) => void; label: string;
}) {
  return <div className="segmented" role="group" aria-label={label}>
    {options.map(option => <button key={option.value} className={option.value === value ? 'selected' : ''} aria-pressed={option.value === value}
      disabled={option.disabled} onClick={() => onChange(option.value)}>{option.label}</button>)}
  </div>;
}
