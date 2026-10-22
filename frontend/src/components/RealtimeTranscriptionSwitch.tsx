'use client';

import { useCallback, useEffect, useId, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Switch } from '@/components/ui/switch';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import type { RecordingPreferences } from '@/services/configService';
import { useI18n } from '@/lib/i18n';

export function RealtimeTranscriptionSwitch() {
  const { t } = useI18n();
  const { isRecording, isStartingRecording, isStopping, isSaving } = useRecordingState();
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const id = useId();
  const load = useCallback(async () => {
    try {
      const prefs = await invoke<RecordingPreferences>('get_recording_preferences');
      setEnabled(prefs.realtime_transcription ?? false);
      setError(null);
    } catch { setError(t('Could not load recording mode')); }
  }, [t]);
  useEffect(() => { void load(); }, [load]);
  const update = async (next: boolean) => {
    setSaving(true);
    try {
      const prefs = await invoke<RecordingPreferences>('get_recording_preferences');
      await invoke('set_recording_preferences', { preferences: { ...prefs, realtime_transcription: next } });
      setEnabled(next);
      setError(null);
    } catch { setError(t('Could not save recording mode')); }
    finally { setSaving(false); }
  };
  const locked = isRecording || isStartingRecording || isStopping || isSaving;
  return (
    <div className="mx-auto w-full max-w-3xl rounded-xl border border-slate-200 bg-slate-50 px-4 py-3">
      <div className="flex min-h-11 items-center justify-between gap-4">
        <label htmlFor={id} className={`text-sm font-medium leading-5 text-slate-800 ${locked ? 'cursor-default' : 'cursor-pointer'}`}>
          {t('Transcribe text in real time (higher load)')}
        </label>
        <Switch id={id} checked={enabled ?? false} disabled={enabled === null || saving || locked}
          aria-describedby={`${id}-help`} onCheckedChange={next => void update(next)} />
      </div>
      <p id={`${id}-help`} className="mt-1 text-xs leading-5 text-slate-500">
        {locked ? t('Recording mode can be changed before the next recording.') : t('When off, audio is saved during recording and text is generated afterwards.')}
      </p>
      {error && <p role="alert" className="mt-2 text-sm text-red-700">{error}</p>}
    </div>
  );
}
