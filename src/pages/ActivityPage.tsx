import { Activity, AlertCircle, ArrowRight, CheckCircle2, Clock3, LoaderCircle, Terminal } from 'lucide-react';
import type { ActivityEntry } from '../types';
import { useI18n } from '../i18n';
import PageHeader from '../components/PageHeader';
import { EmptyState } from '../components/States';

export default function ActivityPage({ items, onExplore }: { items: ActivityEntry[]; onExplore: () => void }) {
  const { t, locale } = useI18n();
  return <div>
    <PageHeader title={t('Operations')} subtitle={t('Installs, removals and updates from this session.')} />
    {items.length === 0
      ? <EmptyState icon={<Activity size={34} strokeWidth={1.5} />} title={t('No operations yet.')}
          action={<button className="btn btn-secondary" onClick={onExplore}>{t('Explore apps')}<ArrowRight size={15} /></button>}>
          {t('When you install, remove or update something, the result and full output appear here.')}</EmptyState>
      : <div className="activity-list">{items.map(item => <article className={`activity-card ${item.status}`} key={item.id}>
        <div className="activity-title">
          <span className={`activity-status ${item.status}`}>{item.status === 'running' ? <LoaderCircle size={20} className="spin" /> : item.status === 'success' ? <CheckCircle2 size={20} /> : <AlertCircle size={20} />}</span>
          <div><h3>{item.title}</h3><span>{item.status === 'running' ? t('In progress: you can close the window, you will be notified when it ends.') : item.status === 'success' ? t('Completed') : t('Not completed')}</span></div>
          <time dateTime={item.startedAt.toISOString()}><Clock3 size={13} />{item.startedAt.toLocaleTimeString(locale, { hour: '2-digit', minute: '2-digit' })}</time>
        </div>
        {item.status === 'running' && <div className="progress-track" role="progressbar" aria-label={t('Operation in progress')}><span /></div>}
        <details open={item.status === 'error'}><summary><Terminal size={14} />{t('Operation output')}</summary><pre>{item.output}</pre></details>
      </article>)}</div>}
  </div>;
}
