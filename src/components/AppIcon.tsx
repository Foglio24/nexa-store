import { useState } from 'react';
import { Package } from 'lucide-react';

export type IconSize = 'sm' | 'md' | 'lg' | 'xl' | 'hero';
const glyph: Record<IconSize, number> = { sm: 18, md: 24, lg: 30, xl: 44, hero: 56 };

/** The app's real logo, or a neutral icon when none exists or it fails to load. */
export default function AppIcon({ name, url, size = 'md' }: { name: string; url?: string; size?: IconSize }) {
  const [failed, setFailed] = useState<string | null>(null);
  const src = url && failed !== url ? url : undefined;
  return <span className={`app-icon app-icon-${size} ${src ? 'has-logo' : 'missing-logo'}`}>
    {src
      ? <img className="app-logo" src={src} alt={`Logo di ${name}`} loading={size === 'xl' || size === 'hero' ? 'eager' : 'lazy'} decoding="async" referrerPolicy="no-referrer" draggable={false} onError={() => setFailed(src)} />
      : <span className="logo-fallback" role="img" aria-label={`Logo non disponibile per ${name}`}><Package size={glyph[size]} strokeWidth={1.6} aria-hidden="true" /></span>}
  </span>;
}
