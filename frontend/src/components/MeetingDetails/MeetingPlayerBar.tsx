'use client';

import { useCallback, useRef, useState, type ReactNode } from 'react';
import { Pause, Play, RotateCcw, RotateCw, Volume2, VolumeX, type LucideIcon } from 'lucide-react';
import { formatPlaybackTime, PLAYBACK_RATES, useMeetingPlayback, type PlaybackControls } from '@/contexts/MeetingPlaybackContext';
import { useMeetingSpeakers } from '@/contexts/MeetingSpeakersContext';
import { useI18n } from '@/lib/i18n';

const SKIP_SECONDS = 10;

/** A circular arrow with the skip length written inside it. */
function SkipIcon({ icon: Icon }: { icon: LucideIcon }) {
  return (
    <span className="relative grid h-5 w-5 place-items-center">
      <Icon className="absolute inset-0 h-5 w-5" strokeWidth={1.75} />
      <span className="relative text-[7px] font-bold leading-none tabular-nums">{SKIP_SECONDS}</span>
    </span>
  );
}

/**
 * Slim player for the meeting's recording. Seeking reports the new position
 * (and who speaks there) through `onSeek` so the page can bring that moment
 * of the transcript into view.
 */
export function MeetingPlayerBar({ onSeek }: { onSeek?: (time: number, speaker?: string) => void }) {
  const playback = useMeetingPlayback();
  const meetingSpeakers = useMeetingSpeakers();
  if (!playback?.isAvailable) return null;
  return (
    <PlayerControls
      playback={playback}
      onSeek={(time) => onSeek?.(time, meetingSpeakers?.speakerAt(time))}
      className="border-b border-slate-100 px-8 py-2"
    />
  );
}

/** Transport, position and speed of `playback`; `label` sits after the buttons. */
export function PlayerControls({
  playback,
  onSeek,
  label,
  className = '',
}: {
  playback: PlaybackControls;
  onSeek?: (time: number) => void;
  label?: ReactNode;
  className?: string;
}) {
  const { t } = useI18n();
  const trackRef = useRef<HTMLDivElement>(null);
  // While dragging, the knob follows the pointer and the audio seeks on release.
  const [dragTime, setDragTime] = useState<number | null>(null);

  const duration = playback.duration;

  const timeAt = useCallback((clientX: number) => {
    const rect = trackRef.current?.getBoundingClientRect();
    if (!rect || !rect.width || !duration) return 0;
    return Math.max(0, Math.min(1, (clientX - rect.left) / rect.width)) * duration;
  }, [duration]);

  const seekTo = (time: number) => {
    const clamped = Math.max(0, duration ? Math.min(duration, time) : time);
    playback.seek(clamped);
    onSeek?.(clamped);
  };

  const shown = dragTime ?? playback.currentTime;
  const percent = duration ? Math.min(100, (shown / duration) * 100) : 0;

  return (
    <div className={`flex shrink-0 items-center gap-4 bg-white ${className}`}>
      <div className="flex shrink-0 items-center gap-1.5">
        <button
          type="button"
          onClick={() => seekTo(playback.currentTime - SKIP_SECONDS)}
          title={t('Back {seconds} s', { seconds: SKIP_SECONDS })}
          aria-label={t('Back {seconds} s', { seconds: SKIP_SECONDS })}
          className="grid h-8 w-8 place-items-center rounded-full text-slate-500 transition-colors hover:bg-slate-100 hover:text-slate-900"
        >
          <SkipIcon icon={RotateCcw} />
        </button>
        <button
          type="button"
          onClick={playback.toggle}
          title={`${playback.isPlaying ? t('Pause') : t('Play')} (${t('Space')})`}
          aria-label={playback.isPlaying ? t('Pause') : t('Play')}
          className="grid h-8 w-8 place-items-center rounded-full bg-indigo-600 text-white shadow-sm shadow-indigo-600/25 transition-transform hover:bg-indigo-700 active:scale-95"
        >
          {playback.isPlaying
            ? <Pause className="h-3.5 w-3.5 fill-current" />
            : <Play className="ml-0.5 h-3.5 w-3.5 fill-current" />}
        </button>
        <button
          type="button"
          onClick={() => seekTo(playback.currentTime + SKIP_SECONDS)}
          title={t('Forward {seconds} s', { seconds: SKIP_SECONDS })}
          aria-label={t('Forward {seconds} s', { seconds: SKIP_SECONDS })}
          className="grid h-8 w-8 place-items-center rounded-full text-slate-500 transition-colors hover:bg-slate-100 hover:text-slate-900"
        >
          <SkipIcon icon={RotateCw} />
        </button>
      </div>

      {label}

      <div className="flex shrink-0 items-center gap-1 font-mono text-xs tabular-nums">
        <span className="font-medium text-slate-900">{formatPlaybackTime(shown)}</span>
        <span className="text-slate-400">/</span>
        <span className="text-slate-400">{formatPlaybackTime(duration)}</span>
      </div>

      <div
        ref={trackRef}
        role="slider"
        tabIndex={0}
        aria-label={t('Playback position')}
        aria-valuemin={0}
        aria-valuemax={Math.round(duration)}
        aria-valuenow={Math.round(shown)}
        aria-valuetext={formatPlaybackTime(shown)}
        className="group relative flex min-w-0 flex-1 cursor-pointer items-center py-2 focus:outline-none"
        onPointerDown={(event) => {
          event.preventDefault();
          event.currentTarget.setPointerCapture(event.pointerId);
          event.currentTarget.focus();
          setDragTime(timeAt(event.clientX));
        }}
        onPointerMove={(event) => {
          if (dragTime !== null) setDragTime(timeAt(event.clientX));
        }}
        onPointerUp={(event) => {
          if (dragTime === null) return;
          setDragTime(null);
          seekTo(timeAt(event.clientX));
        }}
        onPointerCancel={() => setDragTime(null)}
        onKeyDown={(event) => {
          const step = event.shiftKey ? 30 : 5;
          const next =
            event.key === 'ArrowLeft' ? playback.currentTime - step :
            event.key === 'ArrowRight' ? playback.currentTime + step :
            event.key === 'Home' ? 0 :
            event.key === 'End' ? duration :
            null;
          if (next === null) return;
          event.preventDefault();
          seekTo(next);
        }}
      >
        <div className="h-[3px] w-full overflow-hidden rounded-full bg-slate-200 transition-[height] group-hover:h-1">
          <div className="h-full rounded-full bg-indigo-600" style={{ width: `${percent}%` }} />
        </div>
        <div
          aria-hidden
          className={`pointer-events-none absolute h-3 w-3 -translate-x-1/2 rounded-full bg-indigo-600 shadow-sm ring-2 ring-white transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100 ${dragTime !== null ? 'opacity-100' : 'opacity-0'}`}
          style={{ left: `${percent}%` }}
        />
      </div>

      <div className="flex shrink-0 items-center gap-3">
        <div role="group" aria-label={t('Playback speed')} className="flex items-center rounded-lg bg-slate-100 p-0.5 font-mono text-[11px] text-slate-500">
          {PLAYBACK_RATES.map((rate) => {
            const selected = playback.rate === rate;
            return (
              <button
                key={rate}
                type="button"
                aria-pressed={selected}
                onClick={() => playback.setRate(rate)}
                className={`rounded-md px-1.5 py-0.5 transition-colors ${selected ? 'bg-white font-semibold text-indigo-600 shadow-sm' : 'hover:text-slate-900'}`}
              >
                {rate}x
              </button>
            );
          })}
        </div>
        <div className="hidden items-center gap-1.5 text-slate-500 @[56rem]:flex">
          <button
            type="button"
            onClick={() => playback.setVolume(playback.volume > 0 ? 0 : 1)}
            title={playback.volume > 0 ? t('Mute') : t('Unmute')}
            aria-label={playback.volume > 0 ? t('Mute') : t('Unmute')}
            className="rounded hover:text-slate-900"
          >
            {playback.volume > 0 ? <Volume2 className="h-4 w-4" /> : <VolumeX className="h-4 w-4" />}
          </button>
          <input
            type="range"
            min={0}
            max={1}
            step={0.05}
            value={playback.volume}
            onChange={(event) => playback.setVolume(Number(event.target.value))}
            aria-label={t('Volume')}
            className="h-1 w-16 cursor-pointer accent-indigo-600"
          />
        </div>
      </div>
    </div>
  );
}
