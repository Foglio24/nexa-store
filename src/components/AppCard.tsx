import type { AppGroup } from '../types';
import { sourcesOf } from '../lib/groups';
import { groupLogo } from '../lib/util';
import { useStore } from '../StoreContext';
import ActionButton from './ActionButton';
import AppIcon from './AppIcon';
import SourceBadge from './SourceBadge';

/** Grid card used on Explore and on category pages. */
export default function AppCard({ group }: { group: AppGroup }) {
  const { openGroup } = useStore();
  return <article className="app-card">
    <button className="app-card-main" onClick={() => openGroup(group)} aria-label={`Dettagli di ${group.name}`}>
      <AppIcon name={group.name} url={groupLogo(group)} size="lg" />
      <span className="app-card-text">
        <h3 className="app-name">{group.name}</h3>
        <p className="app-summary">{group.summary || 'Descrizione non disponibile.'}</p>
      </span>
    </button>
    <div className="app-card-foot">
      <div className="source-list inline" title="Sorgenti disponibili">{sourcesOf(group).map(source => <SourceBadge key={source} source={source} inline />)}</div>
      <ActionButton group={group} />
    </div>
  </article>;
}
