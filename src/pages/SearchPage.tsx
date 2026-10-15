import { useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { AppGroup, KindFilter, SourceFilter, StorePackage } from '../types';
import AppRow from '../components/AppRow';
import PageHeader from '../components/PageHeader';
import Segmented from '../components/Segmented';
import { SkeletonGrid } from '../components/Skeleton';
import { DesktopOnly, EmptyState, ErrorState } from '../components/States';
import { groupPackages, sourceLabels } from '../lib/groups';
import { useRequest } from '../lib/useRequest';
import { kindOf } from '../lib/util';
import { useStore } from '../StoreContext';

const normalise = (value: string) => value.toLowerCase().replace(/[^\p{L}\p{N}]/gu, '');

export default function SearchPage({ query, sources, refreshTick }: { query: string; sources: SourceFilter[]; refreshTick: number }) {
  const { desktop } = useStore();
  const [source, setSource] = useState<SourceFilter>('all');
  const [kind, setKind] = useState<KindFilter>('application');
  const request = useRequest(desktop ? () => invoke<StorePackage[]>('search_packages', { query, source }) : null, [query, source, refreshTick, desktop], 300);
  const all = request.data ?? [];
  const counts = useMemo(() => ({
    application: groupPackages(all.filter(pkg => kindOf(pkg) === 'application')).length,
    package: groupPackages(all.filter(pkg => kindOf(pkg) === 'package')).length,
  }), [all]);
  const groups = useMemo(() => {
    const key = normalise(query);
    // An app matches by its display name, its package name or the last part of its id (org.gimp.GIMP → GIMP).
    const names = (group: AppGroup) => [group.name, ...group.variants.flatMap(variant => [variant.id.replace(/^app\//, '').split('/')[0].split('.').pop() ?? '', variant.id])];
    const rank = (group: AppGroup) => Math.min(...names(group).map(name => { const n = normalise(name); return !n ? 4 : n === key ? 0 : n.startsWith(key) ? 1 : n.includes(key) ? 2 : 3; }));
    const shown = groupPackages(all.filter(pkg => kind === 'all' || kindOf(pkg) === kind));
    return shown.map((group, index) => ({ group, index, rank: rank(group) })).sort((a, b) => a.rank - b.rank || a.index - b.index).map(item => item.group);
  }, [all, kind, query]);
  if (!desktop) return <DesktopOnly />;
  const hidden = kind === 'application' ? counts.package : kind === 'package' ? counts.application : 0;
  return <div>
    <PageHeader title={`Risultati per “${query}”`}
      subtitle={request.loading && !request.data ? 'Ricerca in corso…' : `${groups.length} ${groups.length === 1 ? 'risultato' : 'risultati'}`}
      actions={<>
        <Segmented label="Tipo" value={kind} onChange={setKind} options={[
          { value: 'application', label: `App (${counts.application})` }, { value: 'package', label: `Pacchetti (${counts.package})` }, { value: 'all', label: 'Tutto' }]} />
        <Segmented label="Sorgente" value={source} onChange={setSource} options={sources.map(value => ({ value, label: value === 'all' ? 'Tutte' : sourceLabels[value] }))} />
      </>} />
    {request.error ? <ErrorState message={request.error} onRetry={request.reload} />
      : request.loading && !request.data ? <SkeletonGrid rows count={6} />
      : groups.length === 0 ? <EmptyState title={kind === 'application' ? 'Nessuna app trovata.' : kind === 'package' ? 'Nessun pacchetto tecnico trovato.' : 'Nessun risultato.'}
          action={hidden > 0 ? <button className="btn btn-secondary" onClick={() => setKind('all')}>Mostra anche {hidden} {kind === 'application' ? 'pacchetti tecnici' : 'app'}</button> : undefined}>
          {hidden > 0 ? 'Ci sono risultati di un altro tipo.' : 'Controlla il nome o prova con un\'altra sorgente.'}</EmptyState>
      : <div className="rows" aria-busy={request.loading}>{groups.map(group => <AppRow key={group.key} group={group} mode="search" />)}</div>}
  </div>;
}
