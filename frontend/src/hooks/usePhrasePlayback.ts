'use client';

import { invoke } from '@tauri-apps/api/core';
import { convertFileSrc } from '@tauri-apps/api/core';
import { useCallback, useEffect, useRef, useState } from 'react';

/**
 * Play a single transcript phrase from the meeting's recording.
 *
 * One shared `Audio` element is reused for every phrase: seeking an existing
 * element is instant, while creating one per row would re-fetch the recording
 * each time. Playback stops itself at the phrase's end rather than running on
 * into the next speaker.
 */
export function usePhrasePlayback(meetingId?: string) {
  const [playingId, setPlayingId] = useState<string | null>(null);
  const [isAvailable, setIsAvailable] = useState(false);
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const sourceRef = useRef<string | null>(null);
  // Set while a phrase is playing so the timeupdate handler knows where to stop.
  const stopAtRef = useRef<number | null>(null);

  // Resolve the recording once per meeting. The command also grants the
  // webview read access to that file, so it must run before any playback.
  useEffect(() => {
    let cancelled = false;
    sourceRef.current = null;
    setIsAvailable(false);
    if (!meetingId) return;

    void invoke<string | null>('get_meeting_audio_path', { meetingId })
      .then((path) => {
        if (cancelled || !path) return;
        sourceRef.current = convertFileSrc(path);
        setIsAvailable(true);
      })
      .catch((error) => {
        // Not fatal: the transcript is still readable, just not playable.
        console.warn('No playable recording for this meeting:', error);
      });

    return () => {
      cancelled = true;
    };
  }, [meetingId]);

  const stop = useCallback(() => {
    const audio = audioRef.current;
    if (audio) audio.pause();
    stopAtRef.current = null;
    setPlayingId(null);
  }, []);

  const toggle = useCallback(
    (id: string, start: number, end?: number) => {
      if (playingId === id) {
        stop();
        return;
      }
      const source = sourceRef.current;
      if (!source) return;

      let audio = audioRef.current;
      if (!audio) {
        audio = new Audio();
        audio.preload = 'metadata';
        audioRef.current = audio;
      }
      if (audio.src !== source) audio.src = source;

      // Fall back to a short window when the segment carries no end time, so a
      // missing value cannot play the rest of the meeting.
      stopAtRef.current = end && end > start ? end : start + 15;
      audio.currentTime = start;
      setPlayingId(id);
      void audio.play().catch((error) => {
        console.warn('Could not play phrase audio:', error);
        stop();
      });
    },
    [playingId, stop],
  );

  // Bind the lifecycle handlers once; they read the refs above.
  useEffect(() => {
    const audio = audioRef.current;
    if (!audio) return;

    const onTimeUpdate = () => {
      const stopAt = stopAtRef.current;
      if (stopAt !== null && audio.currentTime >= stopAt) stop();
    };
    const onEnded = () => stop();

    audio.addEventListener('timeupdate', onTimeUpdate);
    audio.addEventListener('ended', onEnded);
    return () => {
      audio.removeEventListener('timeupdate', onTimeUpdate);
      audio.removeEventListener('ended', onEnded);
    };
  }, [playingId, stop]);

  // Release the element when the transcript unmounts.
  useEffect(
    () => () => {
      audioRef.current?.pause();
      audioRef.current = null;
    },
    [],
  );

  return { playingId, isAvailable, toggle, stop };
}
