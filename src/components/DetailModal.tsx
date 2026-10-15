import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { AlertCircle, Download, ExternalLink, LoaderCircle, RefreshCw, ShieldCheck, Trash2 } from 'lucide-react';
import type { AppGroup, PackageDetails, StorePackage } from '../types';
import { categoryInfo } from '../lib/categories';
import { isRestricted, preferredVariant, sourceLabels } from '../lib/groups';
import { useRequest } from '../lib/useRequest';
import { formatDescription, kindOf, trustedLogoUrl } from '../lib/util';
import { useStore } from '../StoreContext';
import AppIcon from './AppIcon';
import Modal from './Modal';
import SourceBadge from './SourceBadge';

export default function DetailModal({ group, onClose }: { group: AppGroup; onClose: () => void }) {
  const { prepare, open, busy, desktop } = useStore();
  const [selected, setSelected] = useState<StorePackage>(() => preferredVariant(group));
  const request = useRequest(() => invoke<PackageDetails>('package_details', { package: selected }), [selected]);
  const details = request.data;
  const pkg = details?.package ?? selected;
  const logo = trustedLogoUrl(pkg) ?? trustedLogoUrl(selected);
  const restricted = isRestricted(pkg);
  const ready = !request.loading && !request.error && !!details;
  const tags = (pkg.categories ?? []).map(categoryInfo).filter(Boolean);
  const facts: [string, string | undefined][] = [
    ['Sorgente', `${pkg.remote && pkg.source === 'flatpak' ? pkg.remote : sourceLabels[pkg.source]}${pkg.scope && pkg.source === 'flatpak' ? ` · ${pkg.scope === 'user' ? 'utente' : 'sistema'}` : ''}`],
    ['Versione', pkg.version || undefined],
    ['Dimensione', details?.size],
    ['Licenza', details?.license],
    ['Editore', pkg.publisher],
    ['Canale', pkg.channel],
    ['Isolamento', pkg.confinement],
  ];

  return <Modal title={`Dettagli di ${group.name}`} onClose={onClose} wide>
    <header className="detail-header">
      <AppIcon name={group.name} url={logo} size="xl" />
      <div className="detail-title">
        <h2>{group.name}</h2>
        <p>{pkg.summary || group.summary}</p>
        <div className="detail-chips">
          <SourceBadge source={pkg.source} />
          <span className="tag">{kindOf(pkg) === 'application' ? 'App' : 'Pacchetto tecnico'}</span>
          {tags.map(tag => tag && <span className="tag" key={tag.id}>{tag.label}</span>)}
          {pkg.installed && <span className="tag tag-success">Installata</span>}
        </div>
      </div>
    </header>

    {group.variants.length > 1 && <div className="variant-picker">
      <span className="variant-label">Sorgente</span>
      <div className="segmented" role="group" aria-label="Scegli da quale sorgente">
        {group.variants.map(variant => <button key={`${variant.source}:${variant.id}`} className={variant.source === selected.source && variant.id === selected.id ? 'selected' : ''}
          aria-pressed={variant.source === selected.source && variant.id === selected.id} onClick={() => setSelected(variant)}>
          {sourceLabels[variant.source]}{variant.installed ? ' ✓' : ''}
        </button>)}
      </div>
    </div>}

    <div className="detail-actions">
      {pkg.installed && kindOf(pkg) === 'application' && <button className="btn btn-open" disabled={!desktop} onClick={() => open(pkg)}><ExternalLink size={16} />Apri</button>}
      {pkg.updateAvailable && !restricted && <button className="btn btn-primary" disabled={busy || !desktop || !ready} onClick={() => prepare('update', pkg)}><RefreshCw size={16} />Aggiorna</button>}
      {!pkg.installed && !restricted && <button className="btn btn-primary" disabled={busy || !desktop || !ready} onClick={() => prepare('install', pkg)}><Download size={16} />Installa</button>}
      {pkg.installed && !restricted && <button className="btn btn-danger-quiet" disabled={busy || !desktop || !ready} onClick={() => prepare('remove', pkg)}><Trash2 size={16} />Rimuovi</button>}
      <span className="detail-safety"><ShieldCheck size={15} />Prima di ogni modifica vedrai un riepilogo da confermare.</span>
    </div>

    {restricted && <div className="alert alert-warning snap-restriction" role="status"><ShieldCheck size={18} /><div>
      <strong>Gestione Snap limitata all'isolamento strict</strong>
      <p>Questo pacchetto usa «{pkg.confinement}»: puoi consultarne i dettagli, ma installazione e rimozione sono bloccate. Nexa Store gestisce solo applicazioni Snap strict dal canale stable.</p>
    </div></div>}

    {request.loading && !details ? <div className="loading-inline" role="status"><LoaderCircle size={20} className="spin" />Lettura dei dettagli…</div>
      : request.error ? <div className="alert alert-error" role="alert"><AlertCircle size={18} /><p>{request.error}</p></div>
      : details && <div className="detail-body">
        <section>
          <h3>Descrizione</h3>
          <div className="detail-description">{formatDescription(details.description) || 'Nessuna descrizione aggiuntiva disponibile.'}</div>
          {details.homepage && <p className="detail-homepage"><ExternalLink size={14} />{details.homepage}</p>}
        </section>
        <section>
          <h3>Informazioni</h3>
          <dl className="facts">{facts.filter(([, value]) => value).map(([label, value]) => <div key={label}><dt>{label}</dt><dd>{value}</dd></div>)}</dl>
        </section>
      </div>}
  </Modal>;
}
