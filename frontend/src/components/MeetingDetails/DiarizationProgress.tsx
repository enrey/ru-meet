'use client';

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { LoaderCircle } from 'lucide-react';
import { useEffect, useState } from 'react';

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
  const [status, setStatus] = useState<DiarizationStatus>({ inProgress: false, message: '' });

  useEffect(() => {
    onStatusChange?.(status.inProgress);
  }, [onStatusChange, status.inProgress]);

  useEffect(() => {
    let unlistenProgress: (() => void) | undefined;
    let unlistenComplete: (() => void) | undefined;
    let unlistenError: (() => void) | undefined;
    let unlistenRerunError: (() => void) | undefined;
    let unlistenCancelling: (() => void) | undefined;
    let unlistenCancelled: (() => void) | undefined;
    let unlistenSaved: (() => void) | undefined;

    void invoke<DiarizationStatus>('get_diarization_status')
      .then((currentStatus) => {
        setStatus(currentStatus.meetingId === meetingId ? currentStatus : { inProgress: false, message: '' });
      })
      .catch((error) => console.warn('Failed to get diarization status:', error));

    void Promise.all([
      listen<{ message?: string; meetingId?: string }>('diarization-progress', ({ payload }) => {
        if (payload.meetingId === meetingId) {
          setStatus({ inProgress: true, message: payload.message || 'Identifying speakers…', meetingId: payload.meetingId });
        }
      }),
      listen<{ meetingId?: string }>('diarization-complete', ({ payload }) => {
        if (payload.meetingId === meetingId) {
          setStatus({ inProgress: false, message: 'Speaker labels are ready', meetingId: payload.meetingId });
        }
      }),
      listen<string>('diarization-error', ({ payload }) => {
        setStatus({ inProgress: false, message: payload || 'Speaker diarization failed' });
      }),
      listen<{ meetingId?: string }>('diarization-rerun-error', ({ payload }) => {
        if (payload.meetingId === meetingId) {
          setStatus({ inProgress: false, message: 'Speaker diarization failed', meetingId: payload.meetingId });
        }
      }),
      listen<{ meetingId?: string; message?: string }>('diarization-cancelling', ({ payload }) => {
        if (payload.meetingId === meetingId) {
          setStatus({ inProgress: true, message: payload.message || 'Stopping speaker diarization…', meetingId: payload.meetingId });
        }
      }),
      listen<{ meetingId?: string }>('diarization-cancelled', ({ payload }) => {
        if (payload.meetingId === meetingId) {
          setStatus({ inProgress: false, message: 'Speaker diarization stopped', meetingId: payload.meetingId });
        }
      }),
      listen<{ meetingId?: string }>('diarization-labels-saved', ({ payload }) => {
        if (!payload.meetingId || payload.meetingId === meetingId) void onLabelsSaved?.();
      }),
    ]).then(([progress, complete, error, rerunError, cancelling, cancelled, saved]) => {
      unlistenProgress = progress;
      unlistenComplete = complete;
      unlistenError = error;
      unlistenRerunError = rerunError;
      unlistenCancelling = cancelling;
      unlistenCancelled = cancelled;
      unlistenSaved = saved;
    });

    return () => {
      unlistenProgress?.();
      unlistenComplete?.();
      unlistenError?.();
      unlistenRerunError?.();
      unlistenCancelling?.();
      unlistenCancelled?.();
      unlistenSaved?.();
    };
  }, [meetingId, onLabelsSaved]);

  if (!status.inProgress) return null;

  return (
    <div
      className="flex items-center gap-2 rounded-md bg-blue-50 px-2.5 py-1.5 text-xs font-medium text-blue-700"
      title={status.message}
      role="status"
      aria-live="polite"
    >
      <LoaderCircle className="size-4 animate-spin" aria-hidden="true" />
      <span className="hidden @[28rem]:inline">{status.message}</span>
      <span className="sr-only">{status.message}</span>
    </div>
  );
}
