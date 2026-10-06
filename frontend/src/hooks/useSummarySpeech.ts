'use client';

import { invoke, convertFileSrc } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { useCallback, useEffect, useRef, useState } from 'react';

interface ChunkEvent {
  job: number;
  index: number;
  total: number;
  path: string;
}

interface JobEvent {
  job: number;
  message?: string;
}

/**
 * Read the meeting summary aloud with the offline Russian TTS model.
 *
 * The backend synthesizes the summary chunk by chunk and announces each one,
 * so playback starts after the first chunk rather than after the whole text.
 * Chunks are played in order through a single reused `Audio` element.
 *
 * Chunks already synthesized are served from a cache, and then all of them can
 * arrive before `invoke` has even returned the job id, so events for a job we
 * have not learned about yet are buffered instead of dropped.
 */
export function useSummarySpeech(getMarkdown: () => Promise<string>) {
  const [isActive, setIsActive] = useState(false);
  const [isBuffering, setIsBuffering] = useState(false);
  const [isPaused, setIsPaused] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Non-fatal progress note, e.g. that the model is still opening.
  const [notice, setNotice] = useState<string | null>(null);
  // Whether the feature is on and a model is installed; decided in settings.
  const [isAvailable, setIsAvailable] = useState(false);

  const jobRef = useRef<number | null>(null);
  const queueRef = useRef<string[]>([]);
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const completeRef = useRef(false);
  const pausedRef = useRef(false);
  // job id -> chunks that arrived before we knew the job id.
  const pendingRef = useRef(new Map<number, string[]>());
  const finishedRef = useRef(new Set<number>());

  const reset = useCallback(() => {
    jobRef.current = null;
    queueRef.current = [];
    completeRef.current = false;
    pausedRef.current = false;
    pendingRef.current.clear();
    finishedRef.current.clear();
    setIsActive(false);
    setIsBuffering(false);
    setIsPaused(false);
  }, []);

  const playNext = useCallback(() => {
    const audio = audioRef.current;
    if (!audio || jobRef.current === null) return;
    // While paused, chunks keep arriving and queue up silently.
    if (pausedRef.current) return;
    // Still busy with the previous chunk.
    if (audio.src && !audio.paused && !audio.ended) return;

    const next = queueRef.current.shift();
    if (!next) {
      // Out of audio: either the reading is over, or synthesis is still behind.
      if (completeRef.current) reset();
      else setIsBuffering(true);
      return;
    }

    setIsBuffering(false);
    audio.src = convertFileSrc(next);
    void audio.play().catch((playbackError) => {
      console.error('Summary speech playback failed:', playbackError);
      reset();
    });
  }, [reset]);

  useEffect(() => {
    void invoke<{ enabled: boolean; problem: string | null }>('tts_get_status')
      .then((status) => setIsAvailable(status.enabled && status.problem === null))
      .catch((statusError) => {
        console.warn('Could not read TTS status:', statusError);
        setIsAvailable(false);
      });
  }, []);

  useEffect(() => {
    const audio = new Audio();
    audio.addEventListener('ended', () => playNext());
    audio.addEventListener('error', () => playNext());
    audioRef.current = audio;

    const subscriptions: Promise<UnlistenFn>[] = [
      listen<ChunkEvent>('tts-chunk', (event) => {
        const { job, path } = event.payload;
        if (jobRef.current === null) {
          const buffered = pendingRef.current.get(job) ?? [];
          buffered.push(path);
          pendingRef.current.set(job, buffered);
          return;
        }
        if (job !== jobRef.current) return;
        queueRef.current.push(path);
        playNext();
      }),
      listen<JobEvent>('tts-done', (event) => {
        const { job } = event.payload;
        if (jobRef.current === null) {
          finishedRef.current.add(job);
          return;
        }
        if (job !== jobRef.current) return;
        completeRef.current = true;
        playNext();
      }),
      listen<JobEvent>('tts-error', (event) => {
        if (jobRef.current !== null && event.payload.job !== jobRef.current) return;
        setError(event.payload.message ?? 'Speech synthesis failed');
        reset();
      }),
      listen<JobEvent>('tts-waiting', (event) => {
        if (jobRef.current !== null && event.payload.job !== jobRef.current) return;
        setNotice(event.payload.message ?? null);
      }),
    ];

    return () => {
      audio.pause();
      audioRef.current = null;
      subscriptions.forEach((subscription) => {
        void subscription.then((unlisten) => unlisten());
      });
    };
  }, [playNext, reset]);

  /**
   * Stop for good: the backend job is cancelled, the queue dropped and the
   * element rewound, so the next Play reads the summary from the top.
   */
  const stop = useCallback(() => {
    void invoke('tts_stop').catch((stopError) => {
      console.warn('Could not stop the reading:', stopError);
    });
    const audio = audioRef.current;
    if (audio) {
      audio.pause();
      audio.removeAttribute('src');
      audio.load();
    }
    reset();
  }, [reset]);

  /**
   * Pause where we are. Synthesis keeps running in the background, so resuming
   * continues from the same phrase without re-synthesizing.
   */
  const pause = useCallback(() => {
    pausedRef.current = true;
    audioRef.current?.pause();
    setIsPaused(true);
    setIsBuffering(false);
  }, []);

  const resume = useCallback(() => {
    pausedRef.current = false;
    setIsPaused(false);
    const audio = audioRef.current;
    // Mid-phrase: continue it. Between phrases: take the next one.
    if (audio?.src && !audio.ended && audio.currentTime > 0) {
      void audio.play().catch((playbackError) => {
        console.error('Summary speech playback failed:', playbackError);
        reset();
      });
      return;
    }
    playNext();
  }, [playNext, reset]);

  const speak = useCallback(async () => {
    setError(null);
    setNotice(null);
    const markdown = await getMarkdown();
    if (!markdown.trim()) {
      throw new Error('Summary is empty');
    }

    queueRef.current = [];
    completeRef.current = false;
    pausedRef.current = false;
    pendingRef.current.clear();
    finishedRef.current.clear();
    setIsPaused(false);
    setIsActive(true);
    setIsBuffering(true);

    const job = await invoke<number>('tts_speak', { text: markdown });
    jobRef.current = job;

    // Adopt whatever arrived while the call was still in flight.
    queueRef.current = pendingRef.current.get(job) ?? [];
    completeRef.current = finishedRef.current.has(job);
    pendingRef.current.clear();
    finishedRef.current.clear();
    playNext();
  }, [getMarkdown, playNext]);

  /** The play button: start, pause, or resume. */
  const toggle = useCallback(async () => {
    if (isActive) {
      if (isPaused) resume();
      else pause();
      return;
    }
    try {
      await speak();
    } catch (speakError) {
      reset();
      throw speakError;
    }
  }, [isActive, isPaused, pause, reset, resume, speak]);

  return {
    toggle,
    stop,
    isActive,
    isBuffering,
    isPaused,
    isAvailable,
    error,
    notice,
  };
}
