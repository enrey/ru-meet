'use client';

import { useSyncExternalStore } from 'react';

/**
 * Light/dark UI theme.
 *
 * The theme is the `dark` class on `<html>` (Tailwind `darkMode: 'class'`);
 * `theme-palette.js` remaps the palette under it. The choice is kept in
 * localStorage; without one the app follows the OS. `THEME_INIT_SCRIPT` applies
 * it before first paint so a dark start never flashes white.
 */

export type Theme = 'light' | 'dark';

const STORAGE_KEY = 'uiTheme';

export const THEME_INIT_SCRIPT = `(function(){try{var t=localStorage.getItem('${STORAGE_KEY}');if(t!=='light'&&t!=='dark')t=window.matchMedia('(prefers-color-scheme: dark)').matches?'dark':'light';document.documentElement.classList.toggle('dark',t==='dark');}catch(e){}})();`;

const listeners = new Set<() => void>();

function readTheme(): Theme {
  return document.documentElement.classList.contains('dark') ? 'dark' : 'light';
}

export function setTheme(theme: Theme) {
  document.documentElement.classList.toggle('dark', theme === 'dark');
  try {
    localStorage.setItem(STORAGE_KEY, theme);
  } catch {
    // The theme still applies for this session.
  }
  listeners.forEach((listener) => listener());
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function useTheme(): Theme {
  return useSyncExternalStore(subscribe, readTheme, () => 'light');
}
