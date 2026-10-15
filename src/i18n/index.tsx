import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { italian } from './it';

export type Language = 'en' | 'it';
type Arg = string | number;

/** Replace each `{}` in the template with the next argument, in order. */
export function fill(template: string, args: readonly Arg[]): string {
  let index = 0;
  return template.replace(/\{\}/g, () => (index < args.length ? String(args[index++]) : '{}'));
}

/**
 * Messages are written in English in the code; Italian is looked up in `it.ts`, anything missing stays
 * English. When one English word needs two Italian forms, a context follows a `|` ("Installed|list"):
 * English shows the text before it, Italian looks up the whole message.
 */
export function translate(language: Language, message: string, args: readonly Arg[] = []): string {
  const english = message.includes('|') ? message.slice(0, message.indexOf('|')) : message;
  return fill(language === 'it' ? italian[message] ?? english : english, args);
}

/** Language from a browser preference list: the first Italian or English entry wins, as in gettext. */
export function languageFromTags(tags: readonly string[]): Language {
  for (const tag of tags) {
    const primary = tag.toLowerCase().split(/[-_.@]/)[0];
    if (primary === 'it') return 'it';
    if (primary === 'en') return 'en';
  }
  return 'en';
}

const browserLanguage = (): Language => languageFromTags(navigator.languages?.length ? navigator.languages : [navigator.language ?? 'en']);

export interface I18n {
  language: Language;
  /** Locale for numbers and times; the system default when the interface is English. */
  locale: string | undefined;
  t: (message: string, ...args: Arg[]) => string;
}

let active: Language = 'en';
/** For code outside components. Inside components use `useT()`. */
export const tr = (message: string, ...args: Arg[]): string => translate(active, message, args);

const Context = createContext<I18n | null>(null);

/**
 * The interface follows the system language: Italian when the system is Italian, English for every
 * other language. The desktop app asks the backend, which reads the same environment as its own
 * messages (and honours `NEXA_LANG=it|en`); a plain browser preview uses the browser's languages.
 */
export function I18nProvider({ children }: { children: ReactNode }) {
  const [language, setLanguage] = useState<Language | null>(() => (isTauri() ? null : browserLanguage()));
  useEffect(() => {
    if (!isTauri()) return;
    let cancelled = false;
    invoke<string>('app_language')
      .then(value => { if (!cancelled) setLanguage(value === 'it' ? 'it' : 'en'); })
      .catch(() => { if (!cancelled) setLanguage(browserLanguage()); });
    return () => { cancelled = true; };
  }, []);
  useEffect(() => { if (language) document.documentElement.lang = language; }, [language]);
  const value = useMemo<I18n | null>(() => {
    if (!language) return null;
    active = language;
    return { language, locale: language === 'it' ? 'it-IT' : undefined, t: (message, ...args) => translate(language, message, args) };
  }, [language]);
  // Nothing is drawn until the language is known, so the interface never flashes in the wrong one.
  return value ? <Context.Provider value={value}>{children}</Context.Provider> : null;
}

export function useI18n(): I18n {
  const value = useContext(Context);
  if (!value) throw new Error('I18nProvider is missing');
  return value;
}
export const useT = () => useI18n().t;
