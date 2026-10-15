import type { PackageSource } from '../types';
import { sourceLabels } from '../lib/groups';

export default function SourceBadge({ source, className = '', inline = false }: { source: PackageSource; className?: string; inline?: boolean }) {
  return <span className={`source-badge ${source} ${inline ? 'inline' : ''} ${className}`}><span className="source-dot" aria-hidden="true" />{sourceLabels[source]}</span>;
}
