import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ArrowLeft, LoaderCircle } from 'lucide-react';
import type { CategoryId, CategoryPage as CategoryPageData, SourceFilter, StorePackage } from '../types';
import AppCard from '../components/AppCard';
import PageHeader from '../components/PageHeader';
import Segmented from '../components/Segmented';
import { SkeletonGrid } from '../components/Skeleton';
import { DesktopOnly, EmptyState, ErrorState } from '../components/States';
import { categoryInfo } from '../lib/categories';
import { groupPackages, sourceLabels } from '../lib/groups';
import { errorText, newSeed } from '../lib/util';
import { useStore } from '../StoreContext';

const PAGE = 48;
type Sort = 'name' | 'random';

export default function CategoryPage({ category, sources, refreshTick, onBack }: {
  category: CategoryId; sources: SourceFilter[]; refreshTick: number; onBack: () => void;
}) {
  const { desktop } = useStore();
  const info = categoryInfo(category);
  const [source, setSource] = useState<SourceFilter>('all');
  const [sort, setSort] = useState<Sort>('name');
  const [seed] = useState(newSeed);
  const [packages, setPackages] = useState<StorePackage[]>([]);
  const [total, setTotal] = useState(0);
  const [nextOffset, setNextOffset] = useState(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const sequence = useRef(0);

  const load = useCallback(async (offset: number) => {
    const current = ++sequence.current;
    setLoading(true); setError('');
    try {
      const page = await invoke<CategoryPageData>('browse_category', { category, source, sort, seed, offset, limit: PAGE });
      if (current !== sequence.current) return;
      setPackages(previous => offset === 0 ? page.packages : [...previous, ...page.packages]);
      setTotal(page.total); setNextOffset(offset + PAGE);
    } catch (cause) { if (current === sequence.current) setError(errorText(cause)); }
    finally { if (current === sequence.current) setLoading(false); }
  }, [category, source, sort, seed]);
  useEffect(() => { if (desktop) void load(0); }, [load, desktop, refreshTick]);

  const groups = useMemo(() => groupPackages(packages), [packages]);
  if (!desktop) return <DesktopOnly />;
  const first = nextOffset <= PAGE;
  return <div style={{ '--h': info?.hue ?? 215 } as CSSProperties}>
    <PageHeader
      back={<button className="back-link" onClick={onBack}><ArrowLeft size={16} />Esplora</button>}
      title={info?.label ?? category} subtitle={info?.description}
      actions={<>
        <Segmented label="Sorgente" value={source} onChange={setSource}
          options={sources.map(value => ({ value, label: value === 'all' ? 'Tutte' : sourceLabels[value] }))} />
        <Segmented label="Ordine" value={sort} onChange={setSort} options={[{ value: 'name', label: 'A–Z' }, { value: 'random', label: 'Casuale' }]} />
      </>} />
    {error && groups.length === 0 ? <ErrorState message={error} onRetry={() => void load(0)} />
      : loading && first ? <SkeletonGrid count={12} />
      : groups.length === 0 ? <EmptyState title="Nessuna app in questa categoria.">Prova un'altra sorgente.</EmptyState>
      : <>
        <p className="result-count">{total.toLocaleString('it-IT')} app</p>
        <div className="grid-cards" aria-busy={loading}>{groups.map(group => <AppCard key={group.key} group={group} />)}</div>
        {nextOffset < total && <div className="load-more"><button className="btn btn-secondary" onClick={() => void load(nextOffset)} disabled={loading}>{loading ? <><LoaderCircle size={16} className="spin" />Caricamento…</> : 'Mostra altre'}</button></div>}
      </>}
  </div>;
}
