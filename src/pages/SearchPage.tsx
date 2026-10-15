import { useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { AppGroup, KindFilter, SourceFilter, StorePackage } from '../types';
import { useT } from '../i18n';
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
  const t = useT();
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
    <PageHeader title={t('Results for “{}”', query)}
      subtitle={request.loading && !request.data ? t('Searching…') : groups.length === 1 ? t('1 result') : t('{} results', groups.length)}
      actions={<>
        <Segmented label={t('Type')} value={kind} onChange={setKind} options={[
          { value: 'application', label: t('Apps ({})', counts.application) }, { value: 'package', label: t('Packages ({})', counts.package) }, { value: 'all', label: t('Everything') }]} />
        <Segmented label={t('Source')} value={source} onChange={setSource} options={sources.map(value => ({ value, label: value === 'all' ? t('All') : sourceLabels[value] }))} />
      </>} />
    {request.error ? <ErrorState message={request.error} onRetry={request.reload} />
      : request.loading && !request.data ? <SkeletonGrid rows count={6} />
      : groups.length === 0 ? <EmptyState title={kind === 'application' ? t('No apps found.') : kind === 'package' ? t('No technical packages found.') : t('No results.')}
          action={hidden > 0 ? <button className="btn btn-secondary" onClick={() => setKind('all')}>{kind === 'application' ? t('Also show {} technical packages', hidden) : t('Also show {} apps', hidden)}</button> : undefined}>
          {hidden > 0 ? t('There are results of another type.') : t('Check the name or try another source.')}</EmptyState>
      : <div className="rows" aria-busy={request.loading}>{groups.map(group => <AppRow key={group.key} group={group} mode="search" />)}</div>}
  </div>;
}
