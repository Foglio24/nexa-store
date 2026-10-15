export type PackageSource = 'apt' | 'flatpak' | 'snap' | 'appimage';
export type SourceFilter = PackageSource | 'all';
export type PackageKind = 'application' | 'package';
export type KindFilter = PackageKind | 'all';
export type View = 'explore' | 'installed' | 'updates' | 'activity';
export type Action = 'install' | 'remove' | 'update' | 'refresh';
export type CategoryId =
  | 'games' | 'internet' | 'audio-video' | 'graphics' | 'office'
  | 'development' | 'education' | 'science' | 'utilities' | 'system';

export interface StorePackage {
  id: string;
  name: string;
  summary: string;
  version: string;
  source: PackageSource;
  installed: boolean;
  updateAvailable: boolean;
  remote?: string;
  scope?: string;
  publisher?: string;
  channel?: string;
  confinement?: string;
  kind?: PackageKind;
  iconUrl?: string;
  appId?: string;
  categories?: string[];
}
export interface SystemInfo {
  ubuntu: boolean;
  osName: string;
  aptAvailable: boolean;
  flatpakAvailable: boolean;
  snapAvailable: boolean;
  /** AppImage needs no tool of its own, so this is always true; its catalog may still be empty. */
  appimageAvailable: boolean;
  flatpakRemotes: string[];
  /** User Flatpak launchers are not visible to this session until the next login. */
  flatpakMenuHidden: boolean;
  warnings: string[];
}
export interface PackageDetails {
  package: StorePackage;
  description: string;
  homepage?: string;
  size?: string;
  license?: string;
}
export interface Plan {
  token: string;
  title: string;
  summary: string;
  command: string;
  details: string[];
  requiresAdmin: boolean;
}
export interface OperationResult { success: boolean; output: string }
export interface ActivityEntry {
  id: number;
  title: string;
  status: 'running' | 'success' | 'error';
  output: string;
  startedAt: Date;
}
export interface HomeSection { category: CategoryId; packages: StorePackage[] }
export interface ExploreHome { hero: StorePackage[]; picks: StorePackage[]; sections: HomeSection[] }
export interface CategoryPage { packages: StorePackage[]; total: number }

/** The same application as offered by one or more sources. */
export interface AppGroup {
  key: string;
  name: string;
  summary: string;
  iconUrl?: string;
  variants: StorePackage[];
}
