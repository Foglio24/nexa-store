import type { CSSProperties } from 'react';
import type { CategoryInfo } from '../lib/categories';
import { useStore } from '../StoreContext';

export default function CategoryTile({ category }: { category: CategoryInfo }) {
  const { openCategory } = useStore();
  const Icon = category.icon;
  return <button className="category-tile tint" style={{ '--h': category.hue } as CSSProperties} onClick={() => openCategory(category.id)} title={category.description}>
    <span className="category-glyph"><Icon size={22} strokeWidth={1.9} /></span>
    <span className="category-label">{category.label}</span>
  </button>;
}
