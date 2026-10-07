'use client';

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { toast } from 'sonner';
import { useI18n } from '@/lib/i18n';

interface AutomationPreferences {
  autoRecordMeetings: boolean;
  autoRecordSupported: boolean;
  /** After a manual stop the detector waits for 5 s of silence before listening again. */
  pausedAfterManualStop: boolean;
}

export function useAutoRecording() {
  const [preferences, setPreferences] = useState<AutomationPreferences | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState(false);
  const { t } = useI18n();

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    let revision = 0;
    const load = async () => {
      const current = ++revision;
      try {
        const result = await invoke<AutomationPreferences>('get_automation_preferences');
        if (!cancelled && current === revision) { setPreferences(result); setError(false); }
      } catch {
        if (!cancelled && current === revision) setError(true);
      }
    };
    listen<boolean>('auto-record-meetings-changed', () => { void load(); })
      .then(fn => { if (cancelled) fn(); else { unlisten = fn; void load(); } })
      .catch(() => { void load(); });
    window.addEventListener('focus', load);
    return () => { cancelled = true; unlisten?.(); window.removeEventListener('focus', load); };
  }, []);

  const setEnabled = async (enabled: boolean) => {
    if (!preferences?.autoRecordSupported || saving) return;
    setSaving(true);
    try {
      await invoke('set_auto_record_meetings', { enabled });
      setPreferences(previous => previous && { ...previous, autoRecordMeetings: enabled });
    } catch (error) {
      toast.error(t('Could not save preference'), { description: String(error) });
    } finally {
      setSaving(false);
    }
  };

  return { preferences, saving, error, setEnabled };
}
