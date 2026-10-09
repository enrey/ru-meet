'use client';

import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { convertFileSrc, invoke } from '@tauri-apps/api/core';

export const PLAYBACK_RATES = [1, 1.25, 1.5, 2] as const;

/** What the player bars show and drive. */
export interface PlaybackControls {
  /** There is a recording the webview can play. */
  isAvailable: boolean;
  isPlaying: boolean;
  /** Seconds into the recording. */
  currentTime: number;
  /** Length of the recording; 0 while unknown. */
  duration: number;
  rate: number;
  volume: number;
  /** Seek to `time` (when given) and play. */
  play: (time?: number) => void;
  pause: () => void;
  toggle: () => void;
  seek: (time: number) => void;
  skip: (seconds: number) => void;
  setRate: (rate: number) => void;
  setVolume: (volume: number) => void;
}

interface PlaybackSession {
  meetingId: string;
  title: string;
  /** Webview URL of the recording. */
  source: string;
  fallbackDuration?: number;
}

interface PlayerValue extends PlaybackControls {
  session: PlaybackSession | null;
  /** Play was pressed since the session was opened: the player outlives its page. */
  started: boolean;
  /** Meeting whose page is open, if any; that page shows its own player. */
  visibleMeetingId: string | null;
  setVisibleMeetingId: (meetingId: string | null) => void;
  /** Load a meeting's recording, paused at its start. */
  open: (session: PlaybackSession) => void;
  setTitle: (meetingId: string, title: string) => void;
  /** Stop and unload. */
  close: () => void;
}

const PlayerContext = createContext<PlayerValue | null>(null);

/** The app-wide player; null outside `MeetingPlayerProvider`. */
export function useMeetingPlayer(): PlayerValue | null {
  return useContext(PlayerContext);
}

/**
 * One `Audio` element for the whole app, so a recording keeps playing while
 * the user leaves its meeting page. Mounted once, above the routed pages.
 */
export function MeetingPlayerProvider({ children }: { children: ReactNode }) {
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const [session, setSession] = useState<PlaybackSession | null>(null);
  const [started, setStarted] = useState(false);
  const [visibleMeetingId, setVisibleMeetingId] = useState<string | null>(null);
  const [isPlaying, setIsPlaying] = useState(false);
  const [currentTime, setCurrentTime] = useState(0);
  const [mediaDuration, setMediaDuration] = useState(0);
  const [rate, setRateState] = useState(1);
  const [volume, setVolumeState] = useState(1);

  const audio = useCallback(() => {
    if (!audioRef.current) {
      const element = new Audio();
      element.preload = 'metadata';
      audioRef.current = element;
    }
    return audioRef.current;
  }, []);

  useEffect(() => {
    const element = audio();
    const onTime = () => setCurrentTime(element.currentTime);
    const onMetadata = () => setMediaDuration(Number.isFinite(element.duration) ? element.duration : 0);
    const onPlay = () => setIsPlaying(true);
    const onPause = () => setIsPlaying(false);
    element.addEventListener('timeupdate', onTime);
    element.addEventListener('seeked', onTime);
    element.addEventListener('loadedmetadata', onMetadata);
    element.addEventListener('durationchange', onMetadata);
    element.addEventListener('play', onPlay);
    element.addEventListener('pause', onPause);
    element.addEventListener('ended', onPause);
    return () => {
      element.pause();
      element.removeEventListener('timeupdate', onTime);
      element.removeEventListener('seeked', onTime);
      element.removeEventListener('loadedmetadata', onMetadata);
      element.removeEventListener('durationchange', onMetadata);
      element.removeEventListener('play', onPlay);
      element.removeEventListener('pause', onPause);
      element.removeEventListener('ended', onPause);
    };
  }, [audio]);

  // Our own playback is speech too; keep it from starting an automatic recording.
  useEffect(() => {
    void invoke('set_meeting_playback_active', { active: isPlaying })
      .catch((error) => console.warn('Could not report meeting playback:', error));
  }, [isPlaying]);

  // Actions read these instead of state, so they act on a session opened
  // earlier in the same event handler.
  const sessionRef = useRef<PlaybackSession | null>(null);
  const durationRef = useRef(0);

  const open = useCallback((next: PlaybackSession) => {
    const element = audio();
    element.pause();
    element.src = next.source;
    element.playbackRate = rate;
    element.volume = volume;
    sessionRef.current = next;
    durationRef.current = next.fallbackDuration ?? 0;
    setSession(next);
    setStarted(false);
    setCurrentTime(0);
    setMediaDuration(0);
  }, [audio, rate, volume]);

  const close = useCallback(() => {
    const element = audio();
    element.pause();
    element.removeAttribute('src');
    element.load();
    sessionRef.current = null;
    durationRef.current = 0;
    setSession(null);
    setStarted(false);
    setIsPlaying(false);
    setCurrentTime(0);
    setMediaDuration(0);
  }, [audio]);

  const setTitle = useCallback((meetingId: string, title: string) => {
    setSession((current) => (current?.meetingId === meetingId && current.title !== title ? { ...current, title } : current));
  }, []);

  const duration = mediaDuration || session?.fallbackDuration || 0;
  durationRef.current = duration;

  const seek = useCallback((time: number) => {
    if (!sessionRef.current || !Number.isFinite(time)) return;
    const element = audio();
    const limit = durationRef.current;
    element.currentTime = Math.max(0, limit ? Math.min(limit, time) : time);
    setCurrentTime(element.currentTime);
  }, [audio]);

  const play = useCallback((time?: number) => {
    if (!sessionRef.current) return;
    if (time !== undefined) seek(time);
    setStarted(true);
    void audio().play().catch((error) => console.warn('Could not play meeting audio:', error));
  }, [audio, seek]);

  const pause = useCallback(() => audio().pause(), [audio]);

  const toggle = useCallback(() => {
    if (audio().paused) play();
    else audio().pause();
  }, [audio, play]);

  const skip = useCallback((seconds: number) => seek(audio().currentTime + seconds), [audio, seek]);

  const setRate = useCallback((next: number) => {
    audio().playbackRate = next;
    setRateState(next);
  }, [audio]);

  const setVolume = useCallback((next: number) => {
    const value = Math.max(0, Math.min(1, next));
    audio().volume = value;
    setVolumeState(value);
  }, [audio]);

  const value = useMemo<PlayerValue>(() => ({
    session,
    started,
    visibleMeetingId,
    setVisibleMeetingId,
    open,
    setTitle,
    close,
    isAvailable: Boolean(session),
    isPlaying,
    currentTime,
    duration,
    rate,
    volume,
    play,
    pause,
    toggle,
    seek,
    skip,
    setRate,
    setVolume,
  }), [session, started, visibleMeetingId, open, setTitle, close, isPlaying, currentTime, duration, rate, volume, play, pause, toggle, seek, skip, setRate, setVolume]);

  return <PlayerContext.Provider value={value}>{children}</PlayerContext.Provider>;
}

const ScopeContext = createContext<PlaybackControls | null>(null);

/** The player as seen by the open meeting page; null outside a meeting page. */
export function useMeetingPlayback(): PlaybackControls | null {
  return useContext(ScopeContext);
}

/**
 * The app-wide player from the point of view of one meeting page. While
 * another meeting's recording is loaded the page shows its own recording as
 * stopped; playing or seeking here switches the player to this meeting.
 */
export function MeetingPlaybackProvider({
  meetingId,
  title,
  fallbackDuration,
  children,
}: {
  meetingId: string;
  title: string;
  /** Shown before the recording's own metadata has loaded. */
  fallbackDuration?: number;
  children: ReactNode;
}) {
  const player = useMeetingPlayer();
  const [source, setSource] = useState<string | null>(null);

  // The command also grants the webview read access to the recording, so it
  // must resolve before anything plays.
  useEffect(() => {
    let cancelled = false;
    setSource(null);
    void invoke<string | null>('get_meeting_audio_path', { meetingId })
      .then((path) => {
        if (!cancelled && path) setSource(convertFileSrc(path));
      })
      .catch((error) => {
        // Not fatal: the transcript is still readable, just not playable.
        console.warn('No playable recording for this meeting:', error);
      });
    return () => {
      cancelled = true;
    };
  }, [meetingId]);

  const setVisibleMeetingId = player?.setVisibleMeetingId;
  useEffect(() => {
    setVisibleMeetingId?.(meetingId);
    return () => setVisibleMeetingId?.(null);
  }, [meetingId, setVisibleMeetingId]);

  const owns = Boolean(player?.session && player.session.meetingId === meetingId);

  // Take over an idle player right away, so the bar shows the real length
  // and seeking works before the first play.
  const openPlayer = player?.open;
  const idle = !player?.session || !player.started;
  useEffect(() => {
    if (source && idle && !owns) openPlayer?.({ meetingId, title, source, fallbackDuration });
  }, [source, idle, owns, openPlayer, meetingId, title, fallbackDuration]);

  const setTitle = player?.setTitle;
  useEffect(() => {
    setTitle?.(meetingId, title);
  }, [setTitle, meetingId, title]);

  const value = useMemo<PlaybackControls | null>(() => {
    if (!player) return null;
    if (owns) return player;
    // Another meeting is loaded: act on this one only once asked to.
    const take = () => {
      if (source) player.open({ meetingId, title, source, fallbackDuration });
    };
    return {
      isAvailable: Boolean(source),
      isPlaying: false,
      currentTime: 0,
      duration: fallbackDuration ?? 0,
      rate: player.rate,
      volume: player.volume,
      play: (time) => {
        take();
        player.play(time);
      },
      pause: () => undefined,
      toggle: () => {
        take();
        player.play();
      },
      seek: (time) => {
        take();
        player.seek(time);
      },
      skip: (seconds) => {
        take();
        player.skip(seconds);
      },
      setRate: player.setRate,
      setVolume: player.setVolume,
    };
  }, [player, owns, source, meetingId, title, fallbackDuration]);

  useSpaceToggle(Boolean(value?.isAvailable), value?.toggle);

  return <ScopeContext.Provider value={value}>{children}</ScopeContext.Provider>;
}

/** Space plays and pauses unless it is typing or pressing a control. */
export function useSpaceToggle(enabled: boolean, toggle: (() => void) | undefined) {
  const toggleRef = useRef(toggle);
  toggleRef.current = toggle;
  useEffect(() => {
    if (!enabled) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.code !== 'Space' || event.repeat || event.ctrlKey || event.metaKey || event.altKey) return;
      const target = event.target as HTMLElement | null;
      if (target?.closest('input, textarea, select, button, a, [contenteditable=""], [contenteditable="true"], [role="menuitem"], [role="slider"]')) return;
      event.preventDefault();
      toggleRef.current?.();
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [enabled]);
}

/** Recording-relative time: MM:SS, or H:MM:SS past an hour. */
export function formatPlaybackTime(seconds: number): string {
  const total = Math.max(0, Math.floor(Number.isFinite(seconds) ? seconds : 0));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const secs = String(total % 60).padStart(2, '0');
  return hours ? `${hours}:${String(minutes).padStart(2, '0')}:${secs}` : `${String(minutes).padStart(2, '0')}:${secs}`;
}
