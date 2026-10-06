'use client';

import { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ru, ruPatterns } from './ru';

export { ruPlural } from './plural';

/**
 * UI localization.
 *
 * English source text is the message key: `t('Settings')`. Russian lives in
 * `ru.ts`, keyed by that same English text; a missing entry falls back to the
 * key, so untranslated strings stay readable instead of breaking.
 * Placeholders use `{name}` and are filled from `params`.
 */

export type Locale = 'en' | 'ru';
export type TranslationParams = Record<string, string | number>;
export type Translation = string | ((params: TranslationParams) => string);
export type TranslateFn = (key: string, params?: TranslationParams) => string;
/** A whole-string match for dynamic text; `render` receives the capture groups. */
export type TranslationPattern = [RegExp, (...groups: string[]) => string];

export const UI_LOCALES: { value: Locale; label: string }[] = [
  { value: 'en', label: 'English' },
  { value: 'ru', label: 'Русский' },
];

/** A clean install has no stored choice and starts in Russian. */
export const DEFAULT_LOCALE: Locale = 'ru';
const STORAGE_KEY = 'uiLanguage';

const DICTIONARIES: Record<Locale, Record<string, Translation>> = { en: {}, ru };

function readStoredLocale(): Locale {
  if (typeof window === 'undefined') return DEFAULT_LOCALE;
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    return saved === 'en' || saved === 'ru' ? saved : DEFAULT_LOCALE;
  } catch {
    return DEFAULT_LOCALE;
  }
}

// Module-level copy so hooks, services and toasts outside React can translate.
let currentLocale: Locale = readStoredLocale();

export function getLocale(): Locale {
  return currentLocale;
}

/** BCP 47 tag for `Intl`/`toLocale*String` formatting in the current UI language. */
export function getIntlLocale(locale: Locale = currentLocale): string {
  return locale === 'ru' ? 'ru-RU' : 'en-US';
}

function interpolate(text: string, params?: TranslationParams): string {
  if (!params) return text;
  return text.replace(/\{(\w+)\}/g, (match, name: string) =>
    name in params ? String(params[name]) : match,
  );
}

const PATTERNS: Record<Locale, TranslationPattern[]> = { en: [], ru: ruPatterns };

function translateIn(locale: Locale, key: string, params?: TranslationParams): string {
  const entry = DICTIONARIES[locale][key];
  if (typeof entry === 'function') return entry(params ?? {});
  if (entry !== undefined) return interpolate(entry, params);
  // Backend progress text carries numbers (`Transcribing segment 3 of 10...`),
  // so it is matched by shape rather than by exact key.
  for (const [pattern, render] of PATTERNS[locale]) {
    const match = pattern.exec(key);
    if (match) return render(...match.slice(1));
  }
  return interpolate(key, params);
}

/** Translate outside React (hooks' callbacks, services, toasts). */
export const translate: TranslateFn = (key, params) => translateIn(currentLocale, key, params);

interface I18nContextValue {
  locale: Locale;
  setLocale: (locale: Locale) => void;
  t: TranslateFn;
}

const I18nContext = createContext<I18nContextValue>({
  locale: currentLocale,
  setLocale: () => {},
  t: translate,
});

export function I18nProvider({ children }: { children: React.ReactNode }) {
  const [locale, setLocaleState] = useState<Locale>(currentLocale);

  useEffect(() => {
    document.documentElement.lang = locale;
    // The tray menu and system notifications are built in Rust.
    invoke('set_ui_language', { language: locale })
      .catch(error => console.warn('Failed to report UI language to the backend:', error));
  }, [locale]);

  const setLocale = useCallback((next: Locale) => {
    currentLocale = next;
    try {
      localStorage.setItem(STORAGE_KEY, next);
    } catch (error) {
      console.error('Failed to persist UI language:', error);
    }
    setLocaleState(next);
  }, []);

  const value = useMemo<I18nContextValue>(() => ({
    locale,
    setLocale,
    t: (key, params) => translateIn(locale, key, params),
  }), [locale, setLocale]);

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useI18n(): I18nContextValue {
  return useContext(I18nContext);
}
