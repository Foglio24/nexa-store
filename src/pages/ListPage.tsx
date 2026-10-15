import { useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { CheckCircle2, RefreshCw } from 'lucide-react';
import type { AppGroup, KindFilter, SourceFilter, StorePackage } from '../types';
import { useT } from '../i18n';
import AppRow from '../components/AppRow';
import PageHeader from '../components/PageHeader';
import Segmented from '../components/Segmented';
import { SkeletonGrid } from '../components/Skeleton';
import { DesktopOnly, EmptyState, ErrorState } from '../components/States';
import { packageKey, sourceLabels } from '../lib/groups';
import { useRequest } from '../lib/useRequest';
import { kindOf, trustedLogoUrl } from '../lib/util';
import { useStore } from '../StoreContext';

const single = (pkg: StorePackage): AppGroup => ({ key: packageKey(pkg), name: pkg.name, summary: pkg.summary, iconUrl: trustedLogoUrl(pkg), variants: [pkg] });

/** Installed software or available updates: one row per package, with a text filter. */
export default function ListPage({ mode, filter, sources, refreshTick, flatpakReady, snapReady }: {
  mode: 'installed' | 'updates'; filter: string; sources: SourceFilter[]; refreshTick: number; flatpakReady: boolean; snapReady: boolean;
}) {
  const t = useT();
  const { desktop, prepare, busy } = useStore();
  const [source, setSource] = useState<SourceFilter>('all');
  const [kind, setKind] = useState<KindFilter>('application');
  const command = mode === 'installed' ? 'list_installed' : 'list_updates';
  const request = useRequest(desktop ? () => invoke<StorePackage[]>(command, { source }) : null, [command, source, refreshTick, desktop]);
  const text = filter.trim().toLowerCase();
  const matching = useMemo(() => (request.data ?? []).filter(pkg => !text || `${pkg.name} ${pkg.id} ${pkg.summary}`.toLowerCase().includes(text)), [request.data, text]);
  const shown = useMemo(() => matching.filter(pkg => kind === 'all' || kindOf(pkg) === kind).map(single), [matching, kind]);
  const apps = matching.filter(pkg => kindOf(pkg) === 'application').length;
  const technical = matching.length - apps;
  if (!desktop) return <DesktopOnly />;
  const title = mode === 'installed' ? t('Installed|list') : t('Updates');
  const subtitle = mode === 'installed' ? t('Software on this computer.') : t('Updates available from your sources.');
  return <div>
    <PageHeader title={title} subtitle={subtitle}
      actions={<>
        <Segmented label={t('Type')} value={kind} onChange={setKind} options={[
          { value: 'application', label: t('Apps ({})', apps) }, { value: 'package', label: t('Packages ({})', technical) }, { value: 'all', label: t('Everything') }]} />
        <Segmented label={t('Source')} value={source} onChange={setSource} options={sources.map(value => ({ value, label: value === 'all' ? t('All') : sourceLabels[value] }))} />
      </>} />
    {mode === 'updates' && <div className="toolbar">
      <span className="toolbar-label">{t('Check for updates:')}</span>
      <button className="btn btn-secondary btn-sm" disabled={busy} onClick={() => prepare('refresh', undefined, 'apt')}><RefreshCw size={14} />APT</button>
      {flatpakReady && <button className="btn btn-secondary btn-sm" disabled={busy} onClick={() => prepare('refresh', undefined, 'flatpak')}><RefreshCw size={14} />Flatpak</button>}
      {snapReady && <button className="btn btn-secondary btn-sm" disabled={busy} onClick={() => prepare('refresh', undefined, 'snap')} title={t('Lists the updates available in the Snap Store, without installing anything')}><RefreshCw size={14} />Snap</button>}
      <button className="btn btn-ghost btn-sm" disabled={request.loading} onClick={request.reload}>{t('Check again')}</button>
    </div>}
    {request.error ? <ErrorState message={request.error} onRetry={request.reload} />
      : request.loading && !request.data ? <SkeletonGrid rows count={7} />
      : shown.length === 0 ? <EmptyState icon={mode === 'updates' ? <CheckCircle2 size={34} strokeWidth={1.5} /> : undefined}
          title={mode === 'updates' ? (kind === 'package' ? t('No updates for technical packages.') : t('Everything is up to date.')) : text ? t('No results for this filter.') : t('No software in this view.')}
          action={matching.length > 0 && kind !== 'all' ? <button className="btn btn-secondary" onClick={() => setKind('all')}>{t('Show everything')}</button> : undefined}>
          {mode === 'updates' ? t('APT and Flatpak use the local lists, Snap checks the stable channel and AppImage compares the latest GitHub release.') : undefined}</EmptyState>
      : <div className="rows" aria-busy={request.loading}>{shown.map(group => <AppRow key={group.key} group={group} mode={mode} />)}</div>}
  </div>;
}
