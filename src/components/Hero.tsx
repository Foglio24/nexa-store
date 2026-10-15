import { useEffect, useState, type CSSProperties } from 'react';
import { ChevronLeft, ChevronRight } from 'lucide-react';
import type { AppGroup } from '../types';
import { useT } from '../i18n';
import { categoryInfo } from '../lib/categories';
import { sourcesOf } from '../lib/groups';
import { groupLogo } from '../lib/util';
import { useStore } from '../StoreContext';
import ActionButton from './ActionButton';
import AppIcon from './AppIcon';
import SourceBadge from './SourceBadge';

const ROTATION_MS = 9000;

/** Large banner with a few highlighted apps; rotates slowly unless hovered or reduced motion is set. */
export default function Hero({ groups }: { groups: AppGroup[] }) {
  const t = useT();
  const { openGroup } = useStore();
  const [index, setIndex] = useState(0);
  const [paused, setPaused] = useState(false);
  useEffect(() => setIndex(0), [groups]);
  useEffect(() => {
    if (paused || groups.length < 2 || window.matchMedia('(prefers-reduced-motion: reduce)').matches) return;
    const timer = window.setInterval(() => setIndex(value => (value + 1) % groups.length), ROTATION_MS);
    return () => window.clearInterval(timer);
  }, [paused, groups.length]);
  const group = groups[index];
  if (!group) return null;
  const category = categoryInfo(group.variants.flatMap(variant => variant.categories ?? [])[0] ?? '');
  const go = (step: number) => setIndex(value => (value + step + groups.length) % groups.length);
  return <section className="hero" style={{ '--h': category?.hue ?? 215 } as CSSProperties} aria-roledescription={t('carousel')} aria-label={t('Featured apps')}
    onMouseEnter={() => setPaused(true)} onMouseLeave={() => setPaused(false)} onFocus={() => setPaused(true)} onBlur={() => setPaused(false)}>
    <div className="hero-body" key={group.key}>
      <span className="hero-eyebrow">{t('Featured')}{category ? ` · ${t(category.label)}` : ''}</span>
      <h2>{group.name}</h2>
      <p>{group.summary}</p>
      <div className="hero-actions">
        <ActionButton group={group} tone="hero" />
        <button className="btn btn-hero-ghost" onClick={() => openGroup(group)}>{t('Learn more')}</button>
      </div>
      <div className="source-list on-dark">{sourcesOf(group).map(source => <SourceBadge key={source} source={source} />)}</div>
    </div>
    <div className="hero-art" aria-hidden="true"><AppIcon name={group.name} url={groupLogo(group)} size="hero" /></div>
    {groups.length > 1 && <>
      <button className="hero-arrow left" onClick={() => go(-1)} aria-label={t('Previous app')}><ChevronLeft size={20} /></button>
      <button className="hero-arrow right" onClick={() => go(1)} aria-label={t('Next app')}><ChevronRight size={20} /></button>
      <div className="hero-dots" role="group" aria-label={t('Choose the featured app')}>
        {groups.map((item, position) => <button key={item.key} className={position === index ? 'active' : ''} onClick={() => setIndex(position)} aria-label={t('Show {}', item.name)} aria-current={position === index} />)}
      </div>
    </>}
  </section>;
}
