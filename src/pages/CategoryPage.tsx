import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ArrowLeft, LoaderCircle } from 'lucide-react';
import type { CategoryId, CategoryPage as CategoryPageData, SourceFilter, StorePackage } from '../types';
import { useI18n } from '../i18n';
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
  const { t, locale } = useI18n();
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
      back={<button className="back-link" onClick={onBack}><ArrowLeft size={16} />{t('Explore')}</button>}
      title={info ? t(info.label) : category} subtitle={info ? t(info.description) : undefined}
      actions={<>
        <Segmented label={t('Source')} value={source} onChange={setSource}
          options={sources.map(value => ({ value, label: value === 'all' ? t('All') : sourceLabels[value] }))} />
        <Segmented label={t('Order')} value={sort} onChange={setSort} options={[{ value: 'name', label: t('A–Z') }, { value: 'random', label: t('Random') }]} />
      </>} />
    {error && groups.length === 0 ? <ErrorState message={error} onRetry={() => void load(0)} />
      : loading && first ? <SkeletonGrid count={12} />
      : groups.length === 0 ? <EmptyState title={t('No apps in this category.')}>{t('Try another source.')}</EmptyState>
      : <>
        <p className="result-count">{t('{} apps', total.toLocaleString(locale))}</p>
        <div className="grid-cards" aria-busy={loading}>{groups.map(group => <AppCard key={group.key} group={group} />)}</div>
        {nextOffset < total && <div className="load-more"><button className="btn btn-secondary" onClick={() => void load(nextOffset)} disabled={loading}>{loading ? <><LoaderCircle size={16} className="spin" />{t('Loading…')}</> : t('Show more')}</button></div>}
      </>}
  </div>;
}
