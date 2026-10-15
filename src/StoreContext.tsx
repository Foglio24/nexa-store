import { createContext, useContext } from 'react';
import type { Action, AppGroup, CategoryId, PackageSource, StorePackage } from './types';

export interface StoreActions {
  /** True inside the Tauri window; false in a plain browser preview. */
  desktop: boolean;
  /** A confirmation is being prepared or an operation is running. */
  busy: boolean;
  openGroup: (group: AppGroup) => void;
  openPackage: (pkg: StorePackage) => void;
  prepare: (action: Action, pkg?: StorePackage, source?: PackageSource) => void;
  /** Start an installed app. */
  open: (pkg: StorePackage) => void;
  openCategory: (id: CategoryId) => void;
}

export const StoreContext = createContext<StoreActions | null>(null);

export function useStore(): StoreActions {
  const value = useContext(StoreContext);
  if (!value) throw new Error('StoreContext is missing');
  return value;
}
