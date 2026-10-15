import type { ReactNode } from 'react';
import { AlertCircle, Monitor, Search } from 'lucide-react';
import { useT } from '../i18n';

export function EmptyState({ icon, title, children, action }: { icon?: ReactNode; title: string; children?: ReactNode; action?: ReactNode }) {
  return <div className="empty-state">{icon ?? <Search size={34} strokeWidth={1.5} />}<h3>{title}</h3>{children && <p>{children}</p>}{action}</div>;
}

export function ErrorState({ message, onRetry }: { message: string; onRetry?: () => void }) {
  const t = useT();
  return <div className="empty-state error-state" role="alert"><AlertCircle size={34} strokeWidth={1.5} /><h3>{t('We could not load the results.')}</h3><p>{message}</p>{onRetry && <button className="btn btn-secondary" onClick={onRetry}>{t('Try again')}</button>}</div>;
}

export function DesktopOnly() {
  const t = useT();
  return <EmptyState icon={<Monitor size={34} strokeWidth={1.5} />} title={t('The catalog lives in the desktop app.')}>{t("Open Nexa Store on Ubuntu to read your computer's sources.")}</EmptyState>;
}
