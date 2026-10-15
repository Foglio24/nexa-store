import { ExternalLink, RefreshCw, Trash2 } from 'lucide-react';
import type { AppGroup, StorePackage } from '../types';
import { isRestricted, sourcesOf } from '../lib/groups';
import { groupLogo, kindOf } from '../lib/util';
import { useStore } from '../StoreContext';
import ActionButton from './ActionButton';
import AppIcon from './AppIcon';
import SourceBadge from './SourceBadge';

/** List row used for search results, installed software and updates. */
export default function AppRow({ group, mode }: { group: AppGroup; mode: 'search' | 'installed' | 'updates' }) {
  const { openGroup, prepare, open, busy, desktop } = useStore();
  const pkg: StorePackage = group.variants.find(variant => variant.installed) ?? group.variants[0];
  const technical = kindOf(pkg) === 'package';
  return <article className="app-row">
    <button className="app-row-main" onClick={() => openGroup(group)} aria-label={`Dettagli di ${group.name}`}>
      <AppIcon name={group.name} url={groupLogo(group)} size="md" />
      <span className="app-row-text">
        <h3 className="app-name">{group.name}{technical && <span className="tag">Pacchetto tecnico</span>}</h3>
        <p className="app-summary">{group.summary || 'Descrizione non disponibile.'}</p>
      </span>
    </button>
    <div className="app-row-meta">
      <div className="source-list">{sourcesOf(group).map(source => <SourceBadge key={source} source={source} />)}</div>
      {pkg.version && <span className="version" title={pkg.version}>{pkg.version}</span>}
    </div>
    <div className="app-row-action">
      {mode === 'search' && <ActionButton group={group} />}
      {mode === 'installed' && <>
        {!technical && <button className="btn btn-open btn-sm" disabled={!desktop} onClick={() => open(pkg)} aria-label={`Apri ${group.name}`}><ExternalLink size={14} />Apri</button>}
        {pkg.updateAvailable && <button className="btn btn-primary btn-sm" disabled={busy || !desktop} onClick={() => prepare('update', pkg)} aria-label={`Aggiorna ${group.name}`}><RefreshCw size={14} />Aggiorna</button>}
        {isRestricted(pkg)
          ? <button className="btn btn-secondary btn-sm" onClick={() => openGroup(group)}>Dettagli</button>
          : <button className="btn btn-danger-quiet btn-sm" disabled={busy || !desktop} onClick={() => prepare('remove', pkg)} aria-label={`Rimuovi ${group.name}`}><Trash2 size={14} />Rimuovi</button>}
      </>}
      {mode === 'updates' && (isRestricted(pkg)
        ? <button className="btn btn-secondary btn-sm" onClick={() => openGroup(group)}>Dettagli</button>
        : <button className="btn btn-primary btn-sm" disabled={busy || !desktop} onClick={() => prepare('update', pkg)} aria-label={`Aggiorna ${group.name}`}><RefreshCw size={14} />Aggiorna</button>)}
    </div>
  </article>;
}
