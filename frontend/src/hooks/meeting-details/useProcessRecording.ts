"use client";

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { toast } from 'sonner';
import { translate } from '@/lib/i18n';

/** A step of the background processing (`crate::processing`). */
export type ProcessingStep = 'speakers' | 'summary' | 'speech';

type SummaryStatus = 'idle' | 'processing' | 'summarizing' | 'regenerating' | 'completed' | 'error';
const RUNNING: SummaryStatus[] = ['processing', 'summarizing', 'regenerating'];

interface Options {
  meetingId: string;
  /** Diarization of this meeting is running, whoever started it. */
  isDiarizing: boolean;
  summaryStatus: SummaryStatus;
}

interface MeetingEvent {
  meetingId?: string;
  step?: ProcessingStep;
}

/**
 * "Process recording": the backend runs speakers, summary and reading aloud
 * for whatever is missing, whether or not this page stays open. This hook
 * starts it and reports what is running for this meeting - the processing
 * itself, or one of its jobs started some other way.
 */
export function useProcessRecording({ meetingId, isDiarizing, summaryStatus }: Options) {
  /** The processing's current step, while it runs. */
  const [step, setStep] = useState<ProcessingStep | null>(null);
  const [speechRunning, setSpeechRunning] = useState(false);

  useEffect(() => {
    let disposed = false;
    void invoke<Array<{ id: string; processing?: ProcessingStep | null; stages?: { speech?: { state: string } } }>>('api_get_meetings')
      .then((meetings) => {
        const meeting = meetings.find((item) => item.id === meetingId);
        if (disposed || !meeting) return;
        setStep(meeting.processing ?? null);
        if (meeting.stages?.speech?.state === 'running') setSpeechRunning(true);
      })
      .catch(() => {});
    const mine = (payload: MeetingEvent | undefined) => payload?.meetingId === meetingId;
    const unlisteners = [
      listen<MeetingEvent>('meeting-processing', ({ payload }) => {
        if (mine(payload)) setStep(payload.step ?? null);
      }),
      listen<MeetingEvent>('meeting-processed', ({ payload }) => {
        if (mine(payload)) setStep(null);
      }),
      listen<MeetingEvent>('summary-audio-progress', ({ payload }) => {
        if (mine(payload)) setSpeechRunning(true);
      }),
      ...['summary-audio-ready', 'summary-audio-failed', 'summary-audio-cancelled'].map((name) =>
        listen<MeetingEvent>(name, ({ payload }) => {
          if (mine(payload)) setSpeechRunning(false);
        })),
    ];
    return () => {
      disposed = true;
      unlisteners.forEach((unlisten) => void unlisten.then((stop) => stop()));
    };
  }, [meetingId]);

  /** Something is running for this meeting, so the button must not start it again. */
  const busy = step !== null || isDiarizing || RUNNING.includes(summaryStatus) || speechRunning;

  const process = useCallback(async () => {
    if (busy) return;
    try {
      await invoke('process_meeting', { meetingId });
    } catch (error) {
      toast.error(translate('Could not process the recording'), { description: String(error) });
    }
  }, [busy, meetingId]);

  return { step, busy, speechRunning, process };
}
