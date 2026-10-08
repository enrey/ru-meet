'use client';

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { AlertCircle, LoaderCircle } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { useI18n } from '@/lib/i18n';

interface DiarizationStatus {
  inProgress: boolean;
  message: string;
  meetingId?: string;
}

interface DiarizationProgressProps {
  meetingId?: string;
  onLabelsSaved?: () => Promise<void>;
  onStatusChange?: (inProgress: boolean) => void;
}

export function DiarizationProgress({ meetingId, onLabelsSaved, onStatusChange }: DiarizationProgressProps) {
  const { t } = useI18n();
  const [status, setStatus] = useState<DiarizationStatus>({ inProgress: false, message: '' });
  const refreshedMeeting = useRef<string | undefined>(undefined);

  useEffect(() => {
    onStatusChange?.(status.inProgress);
  }, [onStatusChange, status.inProgress]);

  useEffect(() => {
    let cancelled = false;
    let revision = 0;
    let listeners: (() => void)[] = [];
    const apply = (next: DiarizationStatus) => {
      if (cancelled) return;
      setStatus(next);
      if (next.inProgress) refreshedMeeting.current = undefined;
      if (next.message === 'Speaker labels are ready' && refreshedMeeting.current !== meetingId) {
        refreshedMeeting.current = meetingId;
        void onLabelsSaved?.().catch(error => console.warn('Failed to refresh speaker labels:', error));
      }
    };
    const load = async () => {
      const current = ++revision;
      try {
        const next = await invoke<DiarizationStatus>('get_diarization_status', { meetingId });
        if (!cancelled && current === revision) apply(next.meetingId === meetingId ? next : { inProgress: false, message: '' });
      } catch (error) {
        console.warn('Failed to get diarization status:', error);
      }
    };
    const update = (payload: { meetingId?: string; message?: string }, inProgress: boolean, fallback: string) => {
      if (payload.meetingId !== meetingId) return;
      ++revision;
      apply({ inProgress, message: payload.message || fallback, meetingId });
    };
    void Promise.all([
      listen<{ message?: string; meetingId?: string }>('diarization-progress', ({ payload }) => update(payload, true, 'Identifying speakers…')),
      listen<{ meetingId?: string }>('diarization-complete', ({ payload }) => update(payload, false, 'Speaker labels are ready')),
      listen<{ meetingId?: string }>('diarization-rerun-error', ({ payload }) => update(payload, false, 'Speaker diarization failed')),
      listen<{ meetingId?: string; message?: string }>('diarization-cancelling', ({ payload }) => update(payload, true, 'Stopping speaker diarization…')),
      listen<{ meetingId?: string }>('diarization-cancelled', ({ payload }) => update(payload, false, 'Speaker diarization stopped')),
      listen<{ meetingId?: string }>('diarization-labels-saved', ({ payload }) => update(payload, false, 'Speaker labels are ready')),
    ]).then(next => {
      if (cancelled) next.forEach(unlisten => unlisten());
      else { listeners = next; void load(); }
    }).catch(error => { console.warn('Failed to listen for diarization status:', error); void load(); });
    // Recover missed completion events when opening or revisiting a meeting.
    const timer = setInterval(() => void load(), 2000);
    return () => { cancelled = true; clearInterval(timer); listeners.forEach(unlisten => unlisten()); };
  }, [meetingId, onLabelsSaved]);

  const failed = status.message === 'Speaker diarization failed';
  if (!status.inProgress && !failed) return null;
  const displayMessage = status.message.startsWith('Identifying speakers')
    ? t('Identifying speakers…') : t(status.message);

  return (
    <div
      className={`flex items-center gap-2 rounded-md px-2.5 py-1.5 text-xs font-medium ${failed ? 'bg-red-50 text-red-700' : 'bg-indigo-50 text-indigo-700'}`}
      title={t(status.message)}
      role="status"
      aria-live="polite"
    >
      {failed ? <AlertCircle className="size-4" aria-hidden="true" /> : <LoaderCircle className="size-4 motion-safe:animate-spin" aria-hidden="true" />}
      <span>{displayMessage}</span>
    </div>
  );
}
