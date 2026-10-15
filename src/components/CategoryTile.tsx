import type { CSSProperties } from 'react';
import type { CategoryInfo } from '../lib/categories';
import { useT } from '../i18n';
import { useStore } from '../StoreContext';

export default function CategoryTile({ category }: { category: CategoryInfo }) {
  const t = useT();
  const { openCategory } = useStore();
  const Icon = category.icon;
  return <button className="category-tile tint" style={{ '--h': category.hue } as CSSProperties} onClick={() => openCategory(category.id)} title={t(category.description)}>
    <span className="category-glyph"><Icon size={22} strokeWidth={1.9} /></span>
    <span className="category-label">{t(category.label)}</span>
  </button>;
}
