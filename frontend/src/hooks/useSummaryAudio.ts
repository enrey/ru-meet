'use client';

import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { useCallback, useEffect, useState } from 'react';

/** A word of the reading and when it is heard, in seconds. */
export interface SpokenWord {
  text: string;
  start: number;
  end: number;
}

/** The backend's `SummaryAudio`. */
export type SummaryAudioStatus =
  | { state: 'unavailable' }
  | { state: 'noSummary' }
  | { state: 'notPrepared'; stale: boolean }
  | { state: 'preparing'; done: number; total: number }
  | { state: 'failed'; message: string }
  | { state: 'ready'; path: string; duration: number; words: SpokenWord[] };

export interface SummaryAudio {
  /** Null until the first answer. */
  status: SummaryAudioStatus | null;
  /** Webview URL of the prepared reading. */
  source: string | null;
  /** Synthesize the reading now (or again, for an edited summary). */
  prepare: () => void;
}

interface MeetingEvent {
  meetingId: string;
  done?: number;
  total?: number;
  message?: string;
}

/**
 * The summary read aloud, as a file the backend prepares ahead of time:
 * whether it exists and matches the current summary, and its preparation's
 * progress. `summaryKey` changes when the summary does, which may make the
 * file stale.
 */
export function useSummaryAudio(meetingId: string, summaryKey: string): SummaryAudio {
  const [status, setStatus] = useState<SummaryAudioStatus | null>(null);

  const refresh = useCallback(() => {
    void invoke<SummaryAudioStatus>('tts_summary_audio', { meetingId })
      .then(setStatus)
      .catch((error) => {
        console.warn('Could not read the summary reading status:', error);
        setStatus({ state: 'unavailable' });
      });
  }, [meetingId]);

  useEffect(() => {
    setStatus(null);
  }, [meetingId]);

  useEffect(() => {
    refresh();
  }, [refresh, summaryKey]);

  useEffect(() => {
    const mine = (payload: MeetingEvent) => payload.meetingId === meetingId;
    const subscriptions: Promise<UnlistenFn>[] = [
      listen<MeetingEvent>('summary-audio-progress', ({ payload }) => {
        if (mine(payload)) setStatus({ state: 'preparing', done: payload.done ?? 0, total: payload.total ?? 0 });
      }),
      listen<MeetingEvent>('summary-audio-ready', ({ payload }) => {
        if (mine(payload)) refresh();
      }),
      listen<MeetingEvent>('summary-audio-failed', ({ payload }) => {
        if (mine(payload)) setStatus({ state: 'failed', message: payload.message ?? '' });
      }),
    ];
    return () => {
      subscriptions.forEach((subscription) => {
        void subscription.then((unlisten) => unlisten());
      });
    };
  }, [meetingId, refresh]);

  const prepare = useCallback(() => {
    setStatus({ state: 'preparing', done: 0, total: 0 });
    void invoke('tts_prepare_summary_audio', { meetingId }).catch((error) => {
      setStatus({ state: 'failed', message: String(error) });
    });
  }, [meetingId]);

  // A reading made again keeps its file name; the query (ignored by the asset
  // protocol) keeps the webview from playing the old one from its cache.
  const source = status?.state === 'ready' ? `${convertFileSrc(status.path)}?v=${status.duration}` : null;
  return { status, source, prepare };
}
