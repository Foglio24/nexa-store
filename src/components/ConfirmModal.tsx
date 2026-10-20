import { ArrowRight, Check, ChevronRight, Info, ShieldCheck, Terminal } from 'lucide-react';
import type { Action, Plan, SourceFilter } from '../types';
import { useT } from '../i18n';
import Modal from './Modal';
import SourceBadge from './SourceBadge';

/** Summary of exactly what will happen; nothing runs until the user confirms here. */
export default function ConfirmModal({ plan, action, source, running, onCancel, onConfirm }: {
  plan: Plan; action: Action; source?: SourceFilter; running: boolean; onCancel: () => void; onConfirm: () => void;
}) {
  const t = useT();
  const technical = plan.details.filter(detail => detail.includes('\n') || detail.length > 500);
  const summary = plan.details.filter(detail => !detail.includes('\n') && detail.length <= 500);
  const destructive = action === 'remove';
  const confirmLabel = action === 'refresh' && source === 'all' ? t('Check for updates')
    : action === 'update' && source === 'all' ? t('Update all')
    : action === 'refresh' && source === 'snap' ? t('Check')
    : action === 'install' ? t('Install and confirm')
    : action === 'remove' ? t('Remove and confirm')
    : action === 'update' ? t('Update and confirm')
    : t('Refresh indexes and confirm');
  return <Modal title={plan.title} onClose={onCancel} locked={running}>
    <div className="confirm-head">
      <span className={`confirm-icon ${destructive ? 'danger' : ''}`}><ShieldCheck size={26} /></span>
      <div>
        <h2 className="confirm-title">{plan.title}</h2>
        {source && source !== 'all' && <SourceBadge source={source} />}
      </div>
    </div>
    <p className="confirm-summary">{plan.summary}</p>
    {summary.length > 0 && <ul className="plan-details">{summary.map((detail, index) => <li key={index}><Check size={15} /><span>{detail}</span></li>)}</ul>}
    <details className="plan-technical">
      <summary><Terminal size={15} /><span>{t('Technical details and command')}</span><ChevronRight size={15} /></summary>
      <div className="plan-technical-body" tabIndex={0} role="region" aria-label={t('Dependencies, metadata, permissions and command')}>
        {technical.map((detail, index) => <pre key={index}>{detail}</pre>)}
        <div className="command-preview"><span>{t('Command')}</span><pre>{plan.command}</pre></div>
      </div>
    </details>
    <div className="confirm-note"><Info size={17} /><p>{plan.requiresAdmin
      ? t('The system may ask for your password in a separate window. Nexa Store does not see it and does not keep it.')
      : t('The operation starts only after you confirm. You can cancel now without changing anything.')}{' '}
      {t('Once started you can close the window: the operation goes on and you get a notification when it ends.')}</p></div>
    <div className="modal-actions">
      <button className="btn btn-secondary" onClick={onCancel} disabled={running}>{t('Cancel')}</button>
      <button className={`btn ${destructive ? 'btn-danger' : 'btn-primary'}`} data-testid="confirm-operation" onClick={onConfirm} disabled={running}>{confirmLabel}<ArrowRight size={16} /></button>
    </div>
  </Modal>;
}
