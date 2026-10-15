import type { AppGroup, PackageKind, StorePackage } from '../types';

export const errorText = (error: unknown): string =>
  typeof error === 'string' ? error : error instanceof Error ? error.message : 'Si è verificato un errore inatteso.';

export const kindOf = (pkg: StorePackage): PackageKind => pkg.kind ?? (pkg.source === 'apt' ? 'package' : 'application');

/** Only raster data URIs from the local catalogs, or https icons of the Snap Store, are ever shown. */
export function trustedLogoUrl(pkg: StorePackage): string | undefined {
  if (!pkg.iconUrl) return undefined;
  if (/^data:image\/(?:png|jpeg|webp);base64,[a-z0-9+/=\s]+$/i.test(pkg.iconUrl)) return pkg.iconUrl;
  if (pkg.source !== 'snap') return undefined;
  try {
    const url = new URL(pkg.iconUrl);
    return url.protocol === 'https:' && !url.username && !url.password ? url.href : undefined;
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
