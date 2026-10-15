import type { ReactNode } from 'react';
import { AlertCircle, Monitor, Search } from 'lucide-react';

export function EmptyState({ icon, title, children, action }: { icon?: ReactNode; title: string; children?: ReactNode; action?: ReactNode }) {
  return <div className="empty-state">{icon ?? <Search size={34} strokeWidth={1.5} />}<h3>{title}</h3>{children && <p>{children}</p>}{action}</div>;
}

export function ErrorState({ message, onRetry }: { message: string; onRetry?: () => void }) {
  return <div className="empty-state error-state" role="alert"><AlertCircle size={34} strokeWidth={1.5} /><h3>Non siamo riusciti a caricare i risultati.</h3><p>{message}</p>{onRetry && <button className="btn btn-secondary" onClick={onRetry}>Riprova</button>}</div>;
}

export function DesktopOnly() {
  return <EmptyState icon={<Monitor size={34} strokeWidth={1.5} />} title="Il catalogo vive nell'app desktop.">Apri Nexa Store su Ubuntu per leggere le sorgenti del tuo computer.</EmptyState>;
}
