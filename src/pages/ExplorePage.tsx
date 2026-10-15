import { useMemo, type CSSProperties } from 'react';
import { ChevronRight, Shuffle } from 'lucide-react';
import type { ExploreHome } from '../types';
import { useT } from '../i18n';
import AppCard from '../components/AppCard';
import CategoryTile from '../components/CategoryTile';
import Hero from '../components/Hero';
import { SkeletonGrid } from '../components/Skeleton';
import { DesktopOnly, ErrorState } from '../components/States';
import { categories, categoryInfo } from '../lib/categories';
import { groupPackages } from '../lib/groups';
import { useStore } from '../StoreContext';

/** Store front page: highlighted apps, the category tiles and new proposals every visit. */
export default function ExplorePage({ data, loading, error, onShuffle, onRetry }: {
  data: ExploreHome | null; loading: boolean; error: string; onShuffle: () => void; onRetry: () => void;
}) {
  const t = useT();
  const { desktop, openCategory } = useStore();
  const hero = useMemo(() => groupPackages(data?.hero ?? []), [data]);
  const picks = useMemo(() => groupPackages(data?.picks ?? []), [data]);
  const sections = useMemo(() => (data?.sections ?? []).map(section => ({ ...section, groups: groupPackages(section.packages) })), [data]);
  if (!desktop) return <DesktopOnly />;
  if (error && !data) return <ErrorState message={error} onRetry={onRetry} />;
  return <div className="explore">
    {data ? <Hero groups={hero} /> : <div className="hero hero-skeleton skeleton" aria-hidden="true" />}

    <section className="block" aria-labelledby="categories-title">
      <h2 className="section-title" id="categories-title">{t('Categories')}</h2>
      <div className="category-grid">{categories.map(category => <CategoryTile key={category.id} category={category} />)}</div>
    </section>

    <section className="block" aria-labelledby="discover-title" aria-busy={loading}>
      <div className="section-row">
        <h2 className="section-title" id="discover-title">{t('Discover')}</h2>
        <button className="btn btn-ghost btn-sm" onClick={onShuffle} disabled={loading}><Shuffle size={15} />{t('Show more apps')}</button>
      </div>
      {data ? <div className="grid-cards">{picks.map(group => <AppCard key={group.key} group={group} />)}</div> : <SkeletonGrid count={8} />}
    </section>

    {sections.map(section => {
      const info = categoryInfo(section.category);
      const Icon = info?.icon;
      return <section className="block" key={section.category} aria-label={info ? t(info.label) : undefined}>
        <div className="section-row">
          <h2 className="section-title with-glyph" style={{ '--h': info?.hue ?? 215 } as CSSProperties}>{Icon && <span className="glyph tint"><Icon size={17} /></span>}{info ? t(info.label) : section.category}</h2>
          <button className="btn btn-ghost btn-sm" onClick={() => openCategory(section.category)}>{t('See all')}<ChevronRight size={15} /></button>
        </div>
        <div className="grid-cards">{section.groups.map(group => <AppCard key={group.key} group={group} />)}</div>
      </section>;
    })}
  </div>;
}
