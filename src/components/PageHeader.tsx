import type { ReactNode } from 'react';

export default function PageHeader({ title, subtitle, actions, back }: { title: string; subtitle?: string; actions?: ReactNode; back?: ReactNode }) {
  return <header className="page-header">
    {back}
    <div className="page-header-row">
      <div><h1 className="page-title">{title}</h1>{subtitle && <p className="page-subtitle">{subtitle}</p>}</div>
      {actions && <div className="page-actions">{actions}</div>}
    </div>
  </header>;
}
