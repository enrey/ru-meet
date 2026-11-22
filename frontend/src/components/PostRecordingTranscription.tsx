'use client';

import { useEffect, useId, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button } from './ui/button';
import { Switch } from './ui/switch';
import { useI18n } from '@/lib/i18n';
import type { RecordingPreferences } from '@/services/configService';

export function PostRecordingTranscription() {
  const { t } = useI18n();
  const id = useId();
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [available, setAvailable] = useState<boolean | null>(null);
  const [saving, setSaving] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    Promise.all([
      invoke<RecordingPreferences>('get_recording_preferences'),
      invoke<boolean>('term_correction_model_available'),
    ]).then(([prefs, model]) => {
      if (active) {
        setEnabled(prefs.post_recording_transcription ?? true);
        setAvailable(model);
      }
    }).catch(() => {
      if (active) setError(t('Could not load recording mode'));
    });
    return () => { active = false; };
  }, [t]);

  async function update(next: boolean) {
    setSaving(true);
    setError(null);
    try {
      const prefs = await invoke<RecordingPreferences>('get_recording_preferences');
      await invoke('set_recording_preferences', {
        preferences: { ...prefs, post_recording_transcription: next },
      });
      setEnabled(next);
    } catch {
      setError(t('Could not save recording mode'));
    } finally {
      setSaving(false);
    }
  }

  async function install() {
    setInstalling(true);
    setError(null);
    try {
      await invoke('install_term_correction_model');
      setAvailable(true);
    } catch {
      setError(t('Could not install GigaAM Multilingual 220M'));
    } finally {
      setInstalling(false);
    }
  }

  return (
    <section className="mt-4 space-y-3 rounded-xl border border-slate-200 p-4">
      <div className="flex min-h-11 items-center justify-between gap-4">
        <label className="text-sm font-medium" htmlFor={id}>
          {t('Separate transcription and term correction after recording')}
        </label>
        <Switch id={id} checked={enabled ?? false} disabled={enabled === null || saving}
          aria-describedby={`${id}-model`} onCheckedChange={next => void update(next)} />
      </div>
      <div className="flex flex-wrap items-center justify-between gap-3">
        <p id={`${id}-model`} role="status" className="text-sm text-slate-600">
          {available === null ? t('Checking GigaAM Multilingual 220M…') :
            available ? t('GigaAM Multilingual 220M is installed') : t('GigaAM Multilingual 220M is available to install (225 MB)')}
        </p>
        {available === false && (
          <Button type="button" variant="outline" className="min-h-11" disabled={installing} onClick={() => void install()}>
            {installing ? t('Installing…') : t('Install model')}
          </Button>
        )}
      </div>
      {error && <p role="alert" className="text-sm text-red-700">{error}</p>}
    </section>
  );
}
