import { Activity, AlertCircle, ArrowRight, CheckCircle2, Clock3, LoaderCircle, Terminal } from 'lucide-react';
import type { ActivityEntry } from '../types';
import PageHeader from '../components/PageHeader';
import { EmptyState } from '../components/States';

export default function ActivityPage({ items, onExplore }: { items: ActivityEntry[]; onExplore: () => void }) {
  return <div>
    <PageHeader title="Operazioni" subtitle="Installazioni, rimozioni e aggiornamenti di questa sessione." />
    {items.length === 0
      ? <EmptyState icon={<Activity size={34} strokeWidth={1.5} />} title="Nessuna operazione per ora."
          action={<button className="btn btn-secondary" onClick={onExplore}>Esplora le app<ArrowRight size={15} /></button>}>
          Quando installi, rimuovi o aggiorni qualcosa, trovi qui l'esito e l'output completo.</EmptyState>
      : <div className="activity-list">{items.map(item => <article className={`activity-card ${item.status}`} key={item.id}>
        <div className="activity-title">
          <span className={`activity-status ${item.status}`}>{item.status === 'running' ? <LoaderCircle size={20} className="spin" /> : item.status === 'success' ? <CheckCircle2 size={20} /> : <AlertCircle size={20} />}</span>
          <div><h3>{item.title}</h3><span>{item.status === 'running' ? 'In corso: puoi chiudere la finestra, ti avviso al termine.' : item.status === 'success' ? 'Completata' : 'Non completata'}</span></div>
          <time dateTime={item.startedAt.toISOString()}><Clock3 size={13} />{item.startedAt.toLocaleTimeString('it-IT', { hour: '2-digit', minute: '2-digit' })}</time>
        </div>
        {item.status === 'running' && <div className="progress-track" role="progressbar" aria-label="Operazione in corso"><span /></div>}
        <details open={item.status === 'error'}><summary><Terminal size={14} />Output dell'operazione</summary><pre>{item.output}</pre></details>
      </article>)}</div>}
  </div>;
}
