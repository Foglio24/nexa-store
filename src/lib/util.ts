import type { AppGroup, PackageKind, StorePackage } from '../types';
import { tr } from '../i18n';

export const errorText = (error: unknown): string =>
  typeof error === 'string' ? error : error instanceof Error ? error.message : tr('An unexpected error occurred.');

export const kindOf = (pkg: StorePackage): PackageKind => pkg.kind ?? (pkg.source === 'apt' ? 'package' : 'application');

/** Only raster data URIs from the local catalogs, https icons of the Snap Store and PNG icons of the AppImage catalog are ever shown. */
export function trustedLogoUrl(pkg: StorePackage): string | undefined {
  if (!pkg.iconUrl) return undefined;
  if (/^data:image\/(?:png|jpeg|webp);base64,[a-z0-9+/=\s]+$/i.test(pkg.iconUrl)) return pkg.iconUrl;
  if (pkg.source !== 'snap' && pkg.source !== 'appimage') return undefined;
  try {
    const url = new URL(pkg.iconUrl);
    if (url.protocol !== 'https:' || url.username || url.password) return undefined;
    // AppImage icons come from the AppImageHub database only.
    if (pkg.source === 'appimage' && !(url.hostname === 'appimage.github.io' && url.pathname.startsWith('/database/'))) return undefined;
    return url.href;
  } catch { return undefined; }
}

export function groupLogo(group: AppGroup): string | undefined {
  for (const variant of group.variants) {
    const url = trustedLogoUrl(variant);
    if (url) return url;
  }
  return undefined;
}

export const newSeed = () => Math.floor(Math.random() * 2 ** 32);

/**
 * Debian long descriptions are hard-wrapped. Join the lines of each paragraph, keeping blank-line
 * paragraph breaks and bullet lists, so the text reflows to the width of the window.
 */
export function formatDescription(text: string): string {
  return text.replace(/\r/g, '').split(/\n{2,}/).map(paragraph => {
    const lines = paragraph.split('\n').map(line => line.trimEnd());
    if (lines.some(line => /^\s*(?:[-*•]\s|\d+[.)]\s)/.test(line))) return lines.join('\n');
    return lines.map(line => line.trim()).filter(Boolean).join(' ');
  }).filter(Boolean).join('\n\n');
}
