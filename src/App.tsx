import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { AlertCircle, CheckCircle2, Compass, Info, LayoutGrid, ListChecks, LoaderCircle, RefreshCw, Search, X, ArrowRight, type LucideIcon } from 'lucide-react';
import logo from '../icon.svg';
import type {
  Action, ActivityEntry, AppGroup, CategoryId, ExploreHome, OperationResult, PackageSource, Plan,
  SourceFilter, StorePackage, SystemInfo, View,
} from './types';
import ConfirmModal from './components/ConfirmModal';
import DetailModal from './components/DetailModal';
import ActivityPage from './pages/ActivityPage';
import CategoryPage from './pages/CategoryPage';
import ExplorePage from './pages/ExplorePage';
import ListPage from './pages/ListPage';
import SearchPage from './pages/SearchPage';
import { StoreContext, type StoreActions } from './StoreContext';
import { useRequest } from './lib/useRequest';
import { errorText, newSeed } from './lib/util';

const desktop = isTauri();
const navigation: { id: View; label: string; icon: LucideIcon }[] = [
  { id: 'explore', label: 'Esplora', icon: Compass },
  { id: 'installed', label: 'Installate', icon: LayoutGrid },
  { id: 'updates', label: 'Aggiornamenti', icon: RefreshCw },
  { id: 'activity', label: 'Operazioni', icon: ListChecks },
];

function App() {
  const [view, setView] = useState<View>('explore');
  const [category, setCategory] = useState<CategoryId | null>(null);
  const [query, setQuery] = useState('');
  const [system, setSystem] = useState<SystemInfo | null>(null);
  const [systemError, setSystemError] = useState('');
  const [homeSeed, setHomeSeed] = useState(newSeed);
  const [refreshTick, setRefreshTick] = useState(0);
  const [detail, setDetail] = useState<AppGroup | null>(null);
  const [plan, setPlan] = useState<Plan | null>(null);
  const [planAction, setPlanAction] = useState<Action>('install');
  const [planSource, setPlanSource] = useState<PackageSource | undefined>();
  const [preparing, setPreparing] = useState(false);
  const [running, setRunning] = useState(false);
  const [operationError, setOperationError] = useState('');
  const [activities, setActivities] = useState<ActivityEntry[]>([]);
  const [notice, setNotice] = useState('');
  const [dismissed, setDismissed] = useState<string[]>([]);
  const transactionLock = useRef(false);
  const searchInput = useRef<HTMLInputElement>(null);
  const main = useRef<HTMLElement>(null);
  const trimmed = query.trim();
  const searching = view === 'explore' && trimmed.length >= 2;
  const busy = preparing || running;

  const home = useRequest(desktop ? () => invoke<ExploreHome>('explore_home', { seed: homeSeed }) : null, [homeSeed, refreshTick]);
  const updates = useRequest(desktop ? () => invoke<StorePackage[]>('list_updates', { source: 'all' }) : null, [refreshTick], 2500);
  const updateCount = updates.data?.length ?? 0;

  const loadSystem = useCallback(async () => {
    if (!desktop) return;
    try { setSystem(await invoke<SystemInfo>('system_info')); setSystemError(''); }
    catch (cause) { setSystemError(errorText(cause)); }
  }, []);
  useEffect(() => { void loadSystem(); }, [loadSystem]);
  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k') { event.preventDefault(); searchInput.current?.focus(); searchInput.current?.select(); }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, []);
  useEffect(() => { main.current?.scrollTo({ top: 0 }); }, [view, category, searching]);
  useEffect(() => {
    if (!notice) return;
    const timer = window.setTimeout(() => setNotice(''), 7000);
    return () => window.clearTimeout(timer);
  }, [notice]);

  const flatpakReady = !!system?.flatpakAvailable && system.flatpakRemotes.length > 0;
  const sources = useMemo<SourceFilter[]>(() => system
    ? ['all', ...(system.aptAvailable ? ['apt' as const] : []), ...(flatpakReady ? ['flatpak' as const] : []), ...(system.snapAvailable ? ['snap' as const] : [])]
    : ['all', 'apt', 'flatpak', 'snap'], [system, flatpakReady]);

  const openApp = useCallback(async (pkg: StorePackage) => {
    if (!desktop) return;
    setOperationError('');
    try { await invoke('open_app', { package: pkg }); setNotice(`Avvio di ${pkg.name}…`); }
    catch (cause) { setOperationError(errorText(cause)); }
  }, []);
  const closeDetails = useCallback(() => setDetail(null), []);
  const closePlan = useCallback(() => { if (!transactionLock.current) setPlan(null); }, []);
  const prepare = useCallback(async (action: Action, pkg?: StorePackage, specificSource?: PackageSource) => {
    if (!desktop || transactionLock.current) return;
    transactionLock.current = true; setPreparing(true); setOperationError(''); setNotice(''); setDetail(null);
    try {
      const chosen = specificSource ?? pkg?.source ?? 'apt';
      const prepared = await invoke<Plan>('prepare_operation', { action, package: pkg ?? null, source: chosen });
      setPlanAction(action); setPlanSource(chosen); setPlan(prepared);
    } catch (cause) { setOperationError(errorText(cause)); }
    finally { transactionLock.current = false; setPreparing(false); }
  }, []);
  const execute = async () => {
    if (!plan || transactionLock.current || !desktop) return;
    transactionLock.current = true; setRunning(true); setOperationError('');
    const current = plan;
    const activityId = Date.now();
    setActivities(items => [{ id: activityId, title: current.title, status: 'running', output: 'Operazione in esecuzione. L\'output completo sarà disponibile al termine.', startedAt: new Date() }, ...items]);
    setPlan(null); setView('activity'); setCategory(null); setQuery('');
    try {
      const result = await invoke<OperationResult>('execute_operation', { token: current.token });
      setActivities(items => items.map(item => item.id === activityId ? { ...item, status: result.success ? 'success' : 'error', output: result.output || (result.success ? 'Operazione completata.' : 'Operazione non completata.') } : item));
      setNotice(result.success ? 'Operazione completata.' : 'Operazione non completata: consulta i dettagli.');
      setRefreshTick(value => value + 1); void loadSystem();
    } catch (cause) {
      setActivities(items => items.map(item => item.id === activityId ? { ...item, status: 'error', output: errorText(cause) } : item));
      setOperationError(errorText(cause));
    } finally { transactionLock.current = false; setRunning(false); }
  };

  const go = (next: View) => {
    // Choosing "Esplora" while already on the front page brings new proposals.
    if (next === 'explore' && view === 'explore' && !category && !trimmed) setHomeSeed(newSeed());
    setView(next); setCategory(null); setQuery('');
  };
  const actions = useMemo<StoreActions>(() => ({
    desktop, busy,
    openGroup: setDetail,
    openPackage: pkg => setDetail({ key: pkg.id, name: pkg.name, summary: pkg.summary, iconUrl: pkg.iconUrl, variants: [pkg] }),
    prepare: (action, pkg, source) => { void prepare(action, pkg, source); },
    open: pkg => { void openApp(pkg); },
    openCategory: id => { setView('explore'); setCategory(id); setQuery(''); },
  }), [busy, prepare, openApp]);

  const visibleWarnings = (system?.warnings ?? []).filter(warning => !dismissed.includes(warning));
  const placeholder = view === 'explore' ? 'Cerca app, giochi, strumenti…' : view === 'installed' ? 'Cerca tra le installate' : view === 'updates' ? 'Cerca tra gli aggiornamenti' : 'Cerca';

  return <StoreContext.Provider value={actions}>
    <div className="app-shell">
      <aside className="sidebar">
        <button className="brand" onClick={() => go('explore')} aria-label="Nexa Store, pagina iniziale">
          <img src={logo} alt="" width={36} height={36} /><span>Nexa Store</span>
        </button>
        <nav aria-label="Navigazione principale">
          {navigation.map(({ id, label, icon: Icon }) => <button key={id} className={`nav-item ${view === id ? 'active' : ''}`} aria-label={label} aria-current={view === id ? 'page' : undefined} onClick={() => go(id)}>
            <Icon size={19} /><span>{label}</span>
            {id === 'updates' && updateCount > 0 && <span className="nav-badge" aria-label={`${updateCount} aggiornamenti`}>{updateCount}</span>}
            {id === 'activity' && running && <LoaderCircle size={15} className="spin nav-spinner" />}
          </button>)}
        </nav>
        <div className="sidebar-foot">
          <h2>Sorgenti</h2>
          <ul>
            <li><span className={`dot ${system?.aptAvailable ? 'on' : ''}`} />APT</li>
            <li><span className={`dot ${flatpakReady ? 'on' : ''}`} />Flatpak</li>
            <li><span className={`dot ${system?.snapAvailable ? 'on' : ''}`} />Snap</li>
          </ul>
          <p>{system?.osName ?? (desktop ? 'Lettura del sistema…' : 'Anteprima browser')}</p>
        </div>
      </aside>

      <main className="main" ref={main}>
        <div className="topbar">
          <label className="search">
            <Search size={19} />
            <input ref={searchInput} value={query} onChange={event => setQuery(event.target.value)} placeholder={placeholder} aria-label={view === 'explore' ? 'Cerca nello store' : placeholder} spellCheck={false} autoComplete="off" />
            {query ? <button className="icon-button" aria-label="Cancella ricerca" onClick={event => { event.preventDefault(); setQuery(''); searchInput.current?.focus(); }}><X size={17} /></button> : <kbd>Ctrl K</kbd>}
          </label>
        </div>

        <div className="content">
          {!desktop && <div className="alert alert-info" role="status"><Info size={18} /><div><strong>Anteprima dell'interfaccia</strong><p>Cataloghi e operazioni sono disponibili nell'app desktop Nexa Store.</p></div></div>}
          {systemError && <div className="alert alert-error" role="alert"><AlertCircle size={18} /><div><strong>Impossibile leggere il sistema</strong><p>{systemError}</p></div><button className="btn btn-secondary btn-sm" onClick={() => void loadSystem()}>Riprova</button></div>}
          {system?.flatpakMenuHidden && !dismissed.includes('flatpak-menu') && <div className="alert alert-info" role="status"><Info size={18} /><div>
            <strong>Le app Flatpak non compaiono ancora nel menu delle app</strong>
            <p>La tua sessione è iniziata prima del primo Flatpak e non conosce la loro cartella. Esci e rientra una volta sola: da allora le vedrai nel menu, nel dock e nella ricerca. Intanto aprile da qui con «Apri».</p>
          </div><button className="icon-button" aria-label="Nascondi avviso" onClick={() => setDismissed(items => [...items, 'flatpak-menu'])}><X size={16} /></button></div>}
          {visibleWarnings.map(warning => <div className="alert alert-warning" role="status" key={warning}><Info size={18} /><p>{warning}</p><button className="icon-button" aria-label="Nascondi avviso" onClick={() => setDismissed(items => [...items, warning])}><X size={16} /></button></div>)}
          {operationError && <div className="alert alert-error" role="alert"><AlertCircle size={18} /><div><strong>Operazione non disponibile</strong><p>{operationError}</p></div><button className="icon-button" aria-label="Chiudi errore" onClick={() => setOperationError('')}><X size={16} /></button></div>}
          {notice && <div className="alert alert-success" role="status"><CheckCircle2 size={18} /><p>{notice}</p><button className="icon-button" aria-label="Chiudi notifica" onClick={() => setNotice('')}><X size={16} /></button></div>}

          {view === 'explore' && searching && <SearchPage query={trimmed} sources={sources} refreshTick={refreshTick} />}
          {view === 'explore' && !searching && category && <CategoryPage key={category} category={category} sources={sources} refreshTick={refreshTick} onBack={() => setCategory(null)} />}
          {view === 'explore' && !searching && !category && <ExplorePage data={home.data} loading={home.loading} error={home.error} onShuffle={() => setHomeSeed(newSeed())} onRetry={home.reload} />}
          {(view === 'installed' || view === 'updates') && <ListPage key={view} mode={view} filter={query} sources={sources} refreshTick={refreshTick} flatpakReady={flatpakReady} snapReady={!!system?.snapAvailable} />}
          {view === 'activity' && <ActivityPage items={activities} onExplore={() => go('explore')} />}
        </div>
      </main>

      {preparing && <div className="toast" role="status"><LoaderCircle size={18} className="spin" /><span>Preparazione del riepilogo…</span></div>}
      {running && view !== 'activity' && <button className="toast toast-action" onClick={() => go('activity')}><LoaderCircle size={18} className="spin" /><span>Operazione in corso</span><ArrowRight size={16} /></button>}

      {detail && <DetailModal key={detail.key} group={detail} onClose={closeDetails} />}
      {plan && <ConfirmModal plan={plan} action={planAction} source={planSource} running={running} onCancel={closePlan} onConfirm={() => void execute()} />}
    </div>
  </StoreContext.Provider>;
}
export default App;
