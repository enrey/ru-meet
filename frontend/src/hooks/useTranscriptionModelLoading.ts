'use client';

import { useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';

/** Matches `MODEL_LOADING_EVENT` in `src-tauri/src/audio/transcription/worker.rs`. */
const MODEL_LOADING_EVENT = 'transcription-model-loading';

/** A model that is already in memory reports ready at once; don't flash for it. */
const SHOW_AFTER_MS = 400;

/**
 * True while a live recording is capturing audio but its speech recognition
 * model is still loading in the background (no transcript can appear yet).
 */
export function useTranscriptionModelLoading(isRecording: boolean): boolean {
  const [loading, setLoading] = useState(false);
  const [visible, setVisible] = useState(false);

  useEffect(() => {
    const unlisten = listen<{ loading: boolean }>(MODEL_LOADING_EVENT, (event) => {
      setLoading(event.payload.loading);
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    if (!isRecording) setLoading(false);
  }, [isRecording]);

  useEffect(() => {
    if (!loading) {
      setVisible(false);
      return;
    }
    const timer = setTimeout(() => setVisible(true), SHOW_AFTER_MS);
    return () => clearTimeout(timer);
  }, [loading]);

  return visible && isRecording;
}
