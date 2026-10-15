import { CheckCircle2, Download, ExternalLink, RefreshCw } from 'lucide-react';
import type { AppGroup } from '../types';
import { useT } from '../i18n';
import { isInstalled, isRestricted, preferredVariant } from '../lib/groups';
import { kindOf } from '../lib/util';
import { useStore } from '../StoreContext';

/** The one action offered on a card: install, update, or "installed". */
export default function ActionButton({ group, tone = 'default' }: { group: AppGroup; tone?: 'default' | 'hero' }) {
  const t = useT();
  const primary = tone === 'hero' ? 'btn btn-light' : 'btn btn-primary btn-sm';
  const secondary = tone === 'hero' ? 'btn btn-light' : 'btn btn-secondary btn-sm';
  const { prepare, openGroup, open, busy, desktop } = useStore();
  const variant = preferredVariant(group);
  const updatable = group.variants.find(candidate => candidate.updateAvailable);
  if (updatable) {
    return <button className={primary} disabled={busy || !desktop} onClick={() => prepare('update', updatable)} aria-label={t('Update {}', group.name)}><RefreshCw size={14} />{t('Update')}</button>;
  }
  const launchable = group.variants.find(candidate => candidate.installed && kindOf(candidate) === 'application');
  if (launchable) {
    return <button className={tone === 'hero' ? 'btn btn-light' : 'btn btn-open btn-sm'} disabled={!desktop} onClick={() => open(launchable)} aria-label={t('Open {}', group.name)}><ExternalLink size={14} />{t('Open')}</button>;
  }
  if (isInstalled(group)) return <span className={`installed-pill ${tone === 'hero' ? 'on-dark' : ''}`}><CheckCircle2 size={14} />{t('Installed')}</span>;
  if (isRestricted(variant)) {
    return <button className={secondary} onClick={() => openGroup(group)} aria-label={t('Details and limits of {}', group.name)}>{t('Details')}</button>;
  }
  return <button className={primary} disabled={busy || !desktop} onClick={() => prepare('install', variant)} aria-label={t('Install {}', group.name)}><Download size={14} />{t('Install')}</button>;
}
