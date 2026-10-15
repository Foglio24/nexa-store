import type { AppGroup, PackageSource, StorePackage } from '../types';

/** Order in which sources are preferred when the user has not chosen one. */
export const sourceOrder: PackageSource[] = ['apt', 'flatpak', 'snap'];
export const sourceLabels: Record<PackageSource, string> = { apt: 'APT', flatpak: 'Flatpak', snap: 'Snap' };

const nameKey = (name: string) => name.toLowerCase().replace(/[^\p{L}\p{N}]/gu, '');
const idKey = (id: string) => id.trim().toLowerCase().replace(/\.desktop$/, '');

export const packageKey = (pkg: StorePackage) => `${pkg.source}:${pkg.scope ?? ''}:${pkg.remote ?? ''}:${pkg.id}`;

/** A snap that the app cannot manage: shown for information only. */
export const isRestricted = (pkg: StorePackage): boolean =>
  pkg.source === 'snap' && !!pkg.confinement && pkg.confinement !== 'strict';

/** Merge entries that are the same application: equal AppStream id, or equal normalised name. */
export function groupPackages(packages: StorePackage[]): AppGroup[] {
  const groups: AppGroup[] = [];
  const byId = new Map<string, AppGroup>();
  const byName = new Map<string, AppGroup>();
  for (const pkg of packages) {
    const id = pkg.appId ? idKey(pkg.appId) : '';
    const name = nameKey(pkg.name);
    let group = (id && byId.get(id)) || (name && byName.get(name)) || undefined;
    if (!group) {
      group = { key: id || name || packageKey(pkg), name: pkg.name, summary: pkg.summary, iconUrl: pkg.iconUrl, variants: [] };
      groups.push(group);
    }
    if (id && !byId.has(id)) byId.set(id, group);
    if (name && !byName.has(name)) byName.set(name, group);
    if (group.variants.some(variant => packageKey(variant) === packageKey(pkg))) continue;
    group.variants.push(pkg);
    if (!group.summary && pkg.summary) group.summary = pkg.summary;
    if (!group.iconUrl && pkg.iconUrl) group.iconUrl = pkg.iconUrl;
  }
  for (const group of groups) group.variants.sort((a, b) => sourceOrder.indexOf(a.source) - sourceOrder.indexOf(b.source));
  return groups;
}

/** The variant to act on by default: an installed one, otherwise the first manageable source. */
export function preferredVariant(group: AppGroup): StorePackage {
  return group.variants.find(variant => variant.installed)
    ?? group.variants.find(variant => !isRestricted(variant))
    ?? group.variants[0];
}

export const isInstalled = (group: AppGroup) => group.variants.some(variant => variant.installed);
export const hasUpdate = (group: AppGroup) => group.variants.some(variant => variant.updateAvailable);
export const sourcesOf = (group: AppGroup): PackageSource[] =>
  sourceOrder.filter(source => group.variants.some(variant => variant.source === source));
