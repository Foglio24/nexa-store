import { useEffect, useRef, type ReactNode } from 'react';
import { X } from 'lucide-react';

/** Accessible dialog: traps focus, closes on Escape or backdrop click unless `locked`. */
export default function Modal({ title, onClose, children, wide = false, locked = false }: {
  title: string; onClose: () => void; children: ReactNode; wide?: boolean; locked?: boolean;
}) {
  const element = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const root = element.current;
    const focusable = () => Array.from(root?.querySelectorAll<HTMLElement>('button:not([disabled]), a[href], input:not([disabled]), summary, [tabindex="0"]') ?? []);
    root?.focus();
    const keyHandler = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !locked) { event.preventDefault(); onClose(); }
      if (event.key !== 'Tab') return;
      const items = focusable();
      if (!items.length) { event.preventDefault(); root?.focus(); return; }
      const first = items[0];
      const last = items[items.length - 1];
      if (event.shiftKey && (document.activeElement === first || document.activeElement === root)) { event.preventDefault(); last.focus(); }
      else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
    };
    document.addEventListener('keydown', keyHandler);
    return () => { document.removeEventListener('keydown', keyHandler); previous?.focus(); };
  }, [onClose, locked]);
  return <div className="modal-backdrop" onMouseDown={event => { if (event.target === event.currentTarget && !locked) onClose(); }}>
    <div className={`modal ${wide ? 'modal-wide' : ''}`} role="dialog" aria-modal="true" aria-label={title} tabIndex={-1} ref={element}>
      <button className="icon-button modal-close" aria-label="Chiudi finestra" onClick={onClose} disabled={locked}><X size={20} /></button>
      {children}
    </div>
  </div>;
}
