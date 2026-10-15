import { ArrowRight, Check, ChevronRight, Info, ShieldCheck, Terminal } from 'lucide-react';
import type { Action, PackageSource, Plan } from '../types';
import Modal from './Modal';
import SourceBadge from './SourceBadge';

const confirmLabels: Record<Action, string> = { install: 'Installa', remove: 'Rimuovi', update: 'Aggiorna', refresh: 'Aggiorna indici' };

/** Summary of exactly what will happen; nothing runs until the user confirms here. */
export default function ConfirmModal({ plan, action, source, running, onCancel, onConfirm }: {
  plan: Plan; action: Action; source?: PackageSource; running: boolean; onCancel: () => void; onConfirm: () => void;
}) {
  const technical = plan.details.filter(detail => detail.includes('\n') || detail.length > 500);
  const summary = plan.details.filter(detail => !detail.includes('\n') && detail.length <= 500);
  const destructive = action === 'remove';
  return <Modal title={plan.title} onClose={onCancel} locked={running}>
    <div className="confirm-head">
      <span className={`confirm-icon ${destructive ? 'danger' : ''}`}><ShieldCheck size={26} /></span>
      <div>
        <h2 className="confirm-title">{plan.title}</h2>
        {source && <SourceBadge source={source} />}
      </div>
    </div>
    <p className="confirm-summary">{plan.summary}</p>
    {summary.length > 0 && <ul className="plan-details">{summary.map((detail, index) => <li key={index}><Check size={15} /><span>{detail}</span></li>)}</ul>}
    <details className="plan-technical">
      <summary><Terminal size={15} /><span>Dettagli tecnici e comando</span><ChevronRight size={15} /></summary>
      <div className="plan-technical-body" tabIndex={0} role="region" aria-label="Dipendenze, metadati, permessi e comando previsti">
        {technical.map((detail, index) => <pre key={index}>{detail}</pre>)}
        <div className="command-preview"><span>Comando</span><pre>{plan.command}</pre></div>
      </div>
    </details>
    <div className="confirm-note"><Info size={17} /><p>{plan.requiresAdmin
      ? 'Ubuntu potrebbe chiederti la password in una finestra separata. Nexa Store non la vede e non la conserva.'
      : 'L\'operazione parte solo dopo la tua conferma. Puoi annullare adesso senza modificare nulla.'}{' '}
      Una volta avviata puoi chiudere la finestra: l'operazione continua e ti avviso con una notifica quando finisce.</p></div>
    <div className="modal-actions">
      <button className="btn btn-secondary" onClick={onCancel} disabled={running}>Annulla</button>
      <button className={`btn ${destructive ? 'btn-danger' : 'btn-primary'}`} data-testid="confirm-operation" onClick={onConfirm} disabled={running}>{action === 'refresh' && source === 'snap' ? 'Controlla' : `${confirmLabels[action]} e conferma`}<ArrowRight size={16} /></button>
    </div>
  </Modal>;
}
