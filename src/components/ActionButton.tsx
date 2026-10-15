import { CheckCircle2, Download, ExternalLink, RefreshCw } from 'lucide-react';
import type { AppGroup } from '../types';
import { isInstalled, isRestricted, preferredVariant } from '../lib/groups';
import { kindOf } from '../lib/util';
import { useStore } from '../StoreContext';

/** The one action offered on a card: install, update, or "installed". */
export default function ActionButton({ group, tone = 'default' }: { group: AppGroup; tone?: 'default' | 'hero' }) {
  const primary = tone === 'hero' ? 'btn btn-light' : 'btn btn-primary btn-sm';
  const secondary = tone === 'hero' ? 'btn btn-light' : 'btn btn-secondary btn-sm';
  const { prepare, openGroup, open, busy, desktop } = useStore();
  const variant = preferredVariant(group);
  const updatable = group.variants.find(candidate => candidate.updateAvailable);
  if (updatable) {
    return <button className={primary} disabled={busy || !desktop} onClick={() => prepare('update', updatable)} aria-label={`Aggiorna ${group.name}`}><RefreshCw size={14} />Aggiorna</button>;
  }
  const launchable = group.variants.find(candidate => candidate.installed && kindOf(candidate) === 'application');
  if (launchable) {
    return <button className={tone === 'hero' ? 'btn btn-light' : 'btn btn-open btn-sm'} disabled={!desktop} onClick={() => open(launchable)} aria-label={`Apri ${group.name}`}><ExternalLink size={14} />Apri</button>;
  }
  if (isInstalled(group)) return <span className={`installed-pill ${tone === 'hero' ? 'on-dark' : ''}`}><CheckCircle2 size={14} />Installata</span>;
  if (isRestricted(variant)) {
    return <button className={secondary} onClick={() => openGroup(group)} aria-label={`Dettagli e limitazioni di ${group.name}`}>Dettagli</button>;
  }
  return <button className={primary} disabled={busy || !desktop} onClick={() => prepare('install', variant)} aria-label={`Installa ${group.name}`}><Download size={14} />Installa</button>;
}
