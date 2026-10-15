"use client";

import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { toast } from 'sonner';
import { translate } from '@/lib/i18n';

/** A step of the pipeline a recording goes through after it is saved. */
export type ProcessingStep = 'speakers' | 'summary' | 'speech';

type StageState = 'none' | 'running' | 'done' | 'failed';
interface MeetingStages {
  speakers: { state: StageState };
  summary: { state: StageState };
  speech: { state: StageState };
}

type SummaryStatus = 'idle' | 'processing' | 'summarizing' | 'regenerating' | 'completed' | 'error';
const RUNNING: SummaryStatus[] = ['processing', 'summarizing', 'regenerating'];

interface Options {
  meetingId: string;
  meetingFolderPath?: string | null;
  /** Diarization of this meeting is running, whoever started it. */
  isDiarizing: boolean;
  summaryStatus: SummaryStatus;
  onGenerateSummary: (customPrompt: string) => Promise<void>;
  customPrompt: string;
}

/** Resolve with the first of `events` that names this meeting. */
function firstEvent(meetingId: string, events: string[]): { promise: Promise<string>; dispose: () => void } {
  const unlisteners: Array<Promise<() => void>> = [];
  const promise = new Promise<string>((resolve) => {
    for (const name of events) {
      unlisteners.push(listen<{ meetingId?: string }>(name, ({ payload }) => {
        if (payload?.meetingId === meetingId) resolve(name);
      }));
    }
  });
  const dispose = () => unlisteners.forEach((unlisten) => void unlisten.then((stop) => stop()));
  return { promise, dispose };
}

/**
 * Re-run what the app does on its own after a recording - speakers, summary,
 * reading aloud - for the steps that are missing or failed, e.g. when the app
 * was closed halfway. Each step follows the settings: speakers only when
 * diarization is on, reading only when it is installed and on.
 */
export function useProcessRecording({ meetingId, meetingFolderPath, isDiarizing, summaryStatus, onGenerateSummary, customPrompt }: Options) {
  const [step, setStep] = useState<ProcessingStep | null>(null);
  const [speechRunning, setSpeechRunning] = useState(false);

  // The steps keep running in the backend when the page is left; on return
  // the button must show them rather than offer to start again.
  useEffect(() => {
    let disposed = false;
    void stages().then((current) => {
      if (!disposed && current?.speech.state === 'running') setSpeechRunning(true);
    }).catch(() => {});
    const unlisteners = [
      listen<{ meetingId?: string }>('summary-audio-progress', ({ payload }) => {
        if (payload?.meetingId === meetingId) setSpeechRunning(true);
      }),
      ...['summary-audio-ready', 'summary-audio-failed', 'summary-audio-cancelled'].map((name) =>
        listen<{ meetingId?: string }>(name, ({ payload }) => {
          if (payload?.meetingId === meetingId) setSpeechRunning(false);
        })),
    ];
    return () => {
      disposed = true;
      unlisteners.forEach((unlisten) => void unlisten.then((stop) => stop()));
    };
  }, [meetingId]);

  const runningStep: ProcessingStep | null = step
    ?? (isDiarizing ? 'speakers' : RUNNING.includes(summaryStatus) ? 'summary' : speechRunning ? 'speech' : null);
  const summaryWaiter = useRef<{ from: SummaryStatus; running: boolean; resolve: (done: boolean) => void } | null>(null);
  const statusRef = useRef(summaryStatus);
  statusRef.current = summaryStatus;

  // The summary hook only reports through its status; finish the wait when
  // the run it started ends (stopping it counts as not done).
  useEffect(() => {
    const waiter = summaryWaiter.current;
    if (!waiter) return;
    if (RUNNING.includes(summaryStatus)) { waiter.running = true; return; }
    if (!waiter.running && summaryStatus === waiter.from) return;
    summaryWaiter.current = null;
    waiter.resolve(summaryStatus === 'completed');
  }, [summaryStatus]);

  const stages = async (): Promise<MeetingStages | undefined> => {
    const meetings = await invoke<Array<{ id: string; stages: MeetingStages }>>('api_get_meetings', {});
    return meetings.find((meeting) => meeting.id === meetingId)?.stages;
  };

  const runSpeakers = async (): Promise<'done' | 'failed' | 'cancelled'> => {
    const end = firstEvent(meetingId, ['diarization-complete', 'diarization-rerun-error', 'diarization-cancelled']);
    try {
      await invoke('rerun_diarization', { meetingId, meetingFolderPath });
      const event = await end.promise;
      return event === 'diarization-complete' ? 'done' : event === 'diarization-cancelled' ? 'cancelled' : 'failed';
    } catch {
      return 'failed';
    } finally {
      end.dispose();
    }
  };

  const runSummary = async (): Promise<boolean> => {
    const finished = new Promise<boolean>((resolve) => {
      summaryWaiter.current = { from: statusRef.current, running: false, resolve };
    });
    try {
      await onGenerateSummary(customPrompt);
    } catch {
      summaryWaiter.current = null;
      return false;
    }
    return finished;
  };

  const runSpeech = async (alreadyRunning: boolean): Promise<'done' | 'failed' | 'cancelled'> => {
    const end = firstEvent(meetingId, ['summary-audio-ready', 'summary-audio-failed', 'summary-audio-cancelled']);
    try {
      if (!alreadyRunning) await invoke('tts_prepare_summary_audio', { meetingId });
      const event = await end.promise;
      return event === 'summary-audio-ready' ? 'done' : event === 'summary-audio-cancelled' ? 'cancelled' : 'failed';
    } catch {
      return 'failed';
    } finally {
      end.dispose();
    }
  };

  const process = useCallback(async () => {
    if (runningStep) return;
    try {
      const [current, diarization, tts] = await Promise.all([
        stages(),
        invoke<{ enabled: boolean }>('get_diarization_settings'),
        invoke<{ enabled: boolean; problem: string | null }>('tts_get_status'),
      ]);
      if (!current) throw new Error(translate('The meeting was not found'));
      const canSpeak = tts.enabled && tts.problem === null;
      const needSpeakers = diarization.enabled && Boolean(meetingFolderPath) && current.speakers.state !== 'done';
      const needSummary = current.summary.state !== 'done';
      if (!needSpeakers && !needSummary && !(canSpeak && current.speech.state !== 'done')) {
        toast.info(translate('This recording is already processed'));
        return;
      }

      const problems: string[] = [];
      if (needSpeakers) {
        setStep('speakers');
        const result = await runSpeakers();
        if (result === 'cancelled') return;
        if (result === 'failed') problems.push(translate('speakers'));
      }
      if (needSummary) {
        setStep('summary');
        if (!(await runSummary())) {
          toast.error(translate('Processing stopped: the summary was not created'));
          return;
        }
      }
      if (canSpeak) {
        // A new summary starts its reading by itself when that is turned on.
        const speech = (await stages())?.speech.state;
        if (speech !== 'done') {
          setStep('speech');
          const result = await runSpeech(speech === 'running');
          if (result === 'cancelled') return;
          if (result === 'failed') problems.push(translate('reading aloud'));
        }
      }
      if (problems.length) {
        toast.warning(translate('Recording processed, but some steps failed: {steps}', { steps: problems.join(', ') }));
      } else {
        toast.success(translate('Recording processed'));
      }
    } catch (error) {
      toast.error(translate('Could not process the recording'), { description: String(error) });
    } finally {
      setStep(null);
    }
  }, [runningStep, meetingId, meetingFolderPath, customPrompt, onGenerateSummary]);

  return { step: runningStep, speechRunning, process };
}
