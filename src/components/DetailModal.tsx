import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { AlertCircle, Download, ExternalLink, LoaderCircle, RefreshCw, ShieldCheck, Trash2 } from 'lucide-react';
import type { AppGroup, PackageDetails, StorePackage } from '../types';
import { useT } from '../i18n';
import { categoryInfo } from '../lib/categories';
import { isRestricted, preferredVariant, sourceLabels } from '../lib/groups';
import { useRequest } from '../lib/useRequest';
import { formatDescription, kindOf, trustedLogoUrl } from '../lib/util';
import { useStore } from '../StoreContext';
import AppIcon from './AppIcon';
import Modal from './Modal';
import SourceBadge from './SourceBadge';

export default function DetailModal({ group, onClose }: { group: AppGroup; onClose: () => void }) {
  const t = useT();
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
    [t('Source'), `${pkg.remote && pkg.source === 'flatpak' ? pkg.remote : sourceLabels[pkg.source]}${pkg.scope && pkg.source === 'flatpak' ? ` · ${pkg.scope === 'user' ? t('user') : t('system')}` : ''}`],
    [t('Version'), pkg.version || undefined],
    [t('Size'), details?.size],
    [t('License'), details?.license],
    [t('Publisher'), pkg.publisher],
    [t('Channel'), pkg.channel],
    [t('Confinement'), pkg.confinement],
  ];

  return <Modal title={t('Details of {}', group.name)} onClose={onClose} wide>
    <header className="detail-header">
      <AppIcon name={group.name} url={logo} size="xl" />
      <div className="detail-title">
        <h2>{group.name}</h2>
        <p>{pkg.summary || group.summary}</p>
        <div className="detail-chips">
          <SourceBadge source={pkg.source} />
          <span className="tag">{kindOf(pkg) === 'application' ? t('App') : t('Technical package')}</span>
          {tags.map(tag => tag && <span className="tag" key={tag.id}>{t(tag.label)}</span>)}
          {pkg.installed && <span className="tag tag-success">{t('Installed')}</span>}
        </div>
      </div>
    </header>

    {group.variants.length > 1 && <div className="variant-picker">
      <span className="variant-label">{t('Source')}</span>
      <div className="segmented" role="group" aria-label={t('Choose which source')}>
        {group.variants.map(variant => <button key={`${variant.source}:${variant.id}`} className={variant.source === selected.source && variant.id === selected.id ? 'selected' : ''}
          aria-pressed={variant.source === selected.source && variant.id === selected.id} onClick={() => setSelected(variant)}>
          {sourceLabels[variant.source]}{variant.installed ? ' ✓' : ''}
        </button>)}
      </div>
    </div>}

    <div className="detail-actions">
      {pkg.installed && kindOf(pkg) === 'application' && <button className="btn btn-open" disabled={!desktop} onClick={() => open(pkg)}><ExternalLink size={16} />{t('Open')}</button>}
      {pkg.updateAvailable && !restricted && <button className="btn btn-primary" disabled={busy || !desktop || !ready} onClick={() => prepare('update', pkg)}><RefreshCw size={16} />{t('Update')}</button>}
      {!pkg.installed && !restricted && <button className="btn btn-primary" disabled={busy || !desktop || !ready} onClick={() => prepare('install', pkg)}><Download size={16} />{t('Install')}</button>}
      {pkg.installed && !restricted && <button className="btn btn-danger-quiet" disabled={busy || !desktop || !ready} onClick={() => prepare('remove', pkg)}><Trash2 size={16} />{t('Remove')}</button>}
      <span className="detail-safety"><ShieldCheck size={15} />{t('Before every change you will see a summary to confirm.')}</span>
    </div>

    {restricted && <div className="alert alert-warning snap-restriction" role="status"><ShieldCheck size={18} /><div>
      <strong>{t('Snap management is limited to strict confinement')}</strong>
      <p>{t('This package uses “{}”: you can read its details, but installation and removal are blocked. Nexa Store only manages strict Snap apps from the stable channel.', pkg.confinement ?? '')}</p>
    </div></div>}

    {pkg.source === 'appimage' && <div className="alert alert-warning snap-restriction" role="status"><ShieldCheck size={18} /><div>
      <strong>{t('AppImages are not sandboxed')}</strong>
      <p>{t("An AppImage runs with all the permissions of your user. Nexa Store downloads it from the project's official GitHub release and checks it, but cannot vouch for the software itself: install it only from authors you trust.")}</p>
    </div></div>}

    {request.loading && !details ? <div className="loading-inline" role="status"><LoaderCircle size={20} className="spin" />{t('Reading the details…')}</div>
      : request.error ? <div className="alert alert-error" role="alert"><AlertCircle size={18} /><p>{request.error}</p></div>
      : details && <div className="detail-body">
        <section>
          <h3>{t('Description')}</h3>
          <div className="detail-description">{formatDescription(details.description) || t('No further description available.')}</div>
          {details.homepage && <p className="detail-homepage"><ExternalLink size={14} />{details.homepage}</p>}
        </section>
        <section>
          <h3>{t('Information')}</h3>
          <dl className="facts">{facts.filter(([, value]) => value).map(([label, value]) => <div key={label}><dt>{label}</dt><dd>{value}</dd></div>)}</dl>
        </section>
      </div>}
  </Modal>;
}
