import { useCallback, useEffect, useRef, useState } from 'react';
import { errorText } from './util';

interface State<T> { data: T | null; loading: boolean; error: string }

/**
 * Runs `load` whenever `deps` change (after `delay` ms) and keeps the previous data on screen
 * while the next request runs. Only the latest request can update the state.
 * `load` may be `null` to stay idle.
 */
export function useRequest<T>(load: (() => Promise<T>) | null, deps: unknown[], delay = 0) {
  const [state, setState] = useState<State<T>>({ data: null, loading: load !== null, error: '' });
  const [tick, setTick] = useState(0);
  const loader = useRef(load);
  loader.current = load;
  useEffect(() => {
    const run = loader.current;
    if (!run) { setState({ data: null, loading: false, error: '' }); return; }
    let cancelled = false;
    setState(previous => ({ ...previous, loading: true, error: '' }));
    const timer = window.setTimeout(async () => {
      try {
        const data = await run();
        if (!cancelled) setState({ data, loading: false, error: '' });
      } catch (cause) {
        if (!cancelled) setState({ data: null, loading: false, error: errorText(cause) });
      }
    }, delay);
    return () => { cancelled = true; window.clearTimeout(timer); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [...deps, tick, delay]);
  const reload = useCallback(() => setTick(value => value + 1), []);
  return { ...state, reload };
}
