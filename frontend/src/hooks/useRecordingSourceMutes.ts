'use client';

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

export type RecordingSourceMutes = { microphone: boolean; system: boolean };
const changedEvent = 'recording-source-mutes-changed';

export function notifyRecordingSourceMutesChanged(mutes: RecordingSourceMutes) {
  window.dispatchEvent(new CustomEvent<RecordingSourceMutes>(changedEvent, { detail: mutes }));
}

export function useRecordingSourceMutes() {
  const [mutes, setMutes] = useState<RecordingSourceMutes | null>(null);
  useEffect(() => {
    let cancelled = false;
    let revision = 0;
    const load = async () => {
      const current = ++revision;
      try {
        const next = await invoke<RecordingSourceMutes>('get_recording_source_mutes');
        if (!cancelled && current === revision) setMutes(next);
      } catch (error) {
        console.error('Could not load recording sources', error);
      }
    };
    const changed = (event: Event) => {
      ++revision;
      setMutes((event as CustomEvent<RecordingSourceMutes>).detail);
    };
    window.addEventListener(changedEvent, changed);
    window.addEventListener('focus', load);
    void load();
    return () => {
      cancelled = true;
      window.removeEventListener(changedEvent, changed);
      window.removeEventListener('focus', load);
    };
  }, []);
  return mutes;
}
