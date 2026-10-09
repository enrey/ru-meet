'use client';

import { useMemo, useRef, useState } from 'react';
import { Pencil } from 'lucide-react';
import { useMeetingSpeakers } from '@/contexts/MeetingSpeakersContext';
import { formatPlaybackTime, useMeetingPlayback } from '@/contexts/MeetingPlaybackContext';
import { useI18n, type TranslateFn } from '@/lib/i18n';
import { SpeakerRenamePopover } from './SpeakerName';

const NO_COLOR = '#94a3b8';

function formatTalkTime(seconds: number, t: TranslateFn): string {
  if (seconds < 60) return t('{seconds} s', { seconds: Math.max(1, Math.round(seconds)) });
  const minutes = Math.round(seconds / 60);
  const hours = Math.floor(minutes / 60);
  return hours ? t('{hours} h {minutes} min', { hours, minutes: minutes % 60 }) : t('{minutes} min', { minutes });
}

/**
 * Right-hand column of the transcript: who spoke and for how long, and a
 * vertical timeline of the meeting to navigate by. Picking a moment seeks the
 * recording there and reports it through `onSelectTime`.
 */
export function SpeakersRail({ onSelectTime }: { onSelectTime?: (speaker: string, time: number) => void }) {
  const { t } = useI18n();
  const meetingSpeakers = useMeetingSpeakers();
  const playback = useMeetingPlayback();
  const stripRef = useRef<HTMLDivElement>(null);
  const [hover, setHover] = useState<{ y: number; time: number; speaker?: string } | null>(null);

  const stats = useMemo(() => {
    const turns = (meetingSpeakers?.turns ?? [])
      .filter((turn) => Number.isFinite(turn.start) && Number.isFinite(turn.end)
        && turn.start >= 0 && turn.end > turn.start && turn.speaker.trim())
      .sort((a, b) => a.start - b.start);
    const talk = new Map<string, number>();
    for (const turn of turns) talk.set(turn.speaker, (talk.get(turn.speaker) ?? 0) + turn.end - turn.start);
    const total = Array.from(talk.values()).reduce((sum, seconds) => sum + seconds, 0);
    const speakers = Array.from(talk, ([speaker, seconds]) => ({ speaker, seconds, share: total ? seconds / total : 0 }))
      .sort((a, b) => b.seconds - a.seconds);
    const end = Math.max(0, ...turns.map((turn) => turn.end));
    return { turns, speakers, total, end };
  }, [meetingSpeakers?.turns]);

  if (!meetingSpeakers || meetingSpeakers.inProgress || !stats.turns.length) return null;

  const duration = Math.max(playback?.duration ?? 0, stats.end);
  const currentTime = playback?.currentTime ?? 0;
  const started = Boolean(playback && (playback.isPlaying || currentTime > 0));
  const speakingNow = started ? meetingSpeakers.speakerAt(currentTime) : undefined;
  const speakingTurn = speakingNow
    ? stats.turns.find((turn) => turn.speaker === speakingNow && turn.start <= currentTime && currentTime < turn.end)
    : undefined;

  const select = (speaker: string, time: number) => {
    playback?.seek(time);
    onSelectTime?.(speaker, time);
  };

  // Each click on a speaker moves to their next turn, wrapping to the first.
  const nextTurnOf = (speaker: string) => {
    const own = stats.turns.filter((turn) => turn.speaker === speaker);
    return own.find((turn) => turn.start > currentTime + 0.5) ?? own[0];
  };

  const timeAtY = (clientY: number) => {
    const rect = stripRef.current?.getBoundingClientRect();
    if (!rect || !rect.height) return null;
    const fraction = Math.max(0, Math.min(1, (clientY - rect.top) / rect.height));
    return { y: fraction * rect.height, time: fraction * duration };
  };

  return (
    <aside aria-label={t('Speakers')} className="hidden w-[300px] shrink-0 flex-col border-l border-slate-100 bg-white px-5 pb-4 pt-4 @[52rem]:flex">
      <div className="flex items-center gap-2">
        <h2 className="text-sm font-semibold text-slate-900">{t('Speakers')}</h2>
        <span className="rounded-full bg-slate-100 px-1.5 text-xs tabular-nums text-slate-500">{stats.speakers.length}</span>
      </div>

      <div className="mt-3 flex h-1 w-full overflow-hidden rounded-full bg-slate-100" title={t('Share of speaking time')}>
        {stats.speakers.map(({ speaker, share }) => (
          <div
            key={speaker}
            className="h-full"
            style={{ width: `${share * 100}%`, backgroundColor: meetingSpeakers.colorFor(speaker) ?? NO_COLOR }}
            title={`${meetingSpeakers.displayName(speaker)}: ${Math.round(share * 100)}%`}
          />
        ))}
      </div>
      <div className="mt-1.5 text-[11px] tabular-nums text-slate-400">
        {t('Speech: {time}', { time: formatTalkTime(stats.total, t) })}
      </div>

      <div className="mt-4 flex min-h-0 flex-1 gap-3">
        {/* Vertical timeline: top is the start of the meeting, bottom its end. */}
        <div className="flex shrink-0 flex-col items-center gap-1 font-mono text-[10px] text-slate-400">
          <span>{formatPlaybackTime(0)}</span>
          <div
            ref={stripRef}
            role="slider"
            tabIndex={0}
            aria-orientation="vertical"
            aria-label={t('Meeting timeline')}
            aria-valuemin={0}
            aria-valuemax={Math.round(duration)}
            aria-valuenow={Math.round(currentTime)}
            aria-valuetext={formatPlaybackTime(currentTime)}
            className="group relative flex w-4 flex-1 cursor-pointer justify-center focus:outline-none"
            onPointerMove={(event) => {
              const at = timeAtY(event.clientY);
              setHover(at && { ...at, speaker: meetingSpeakers.speakerAt(at.time) });
            }}
            onPointerLeave={() => setHover(null)}
            onClick={(event) => {
              const at = timeAtY(event.clientY);
              const speaker = at && meetingSpeakers.speakerAt(at.time);
              if (at && speaker) select(speaker, at.time);
            }}
            onKeyDown={(event) => {
              const step = event.shiftKey ? 60 : 10;
              const time =
                event.key === 'ArrowUp' ? currentTime - step :
                event.key === 'ArrowDown' ? currentTime + step :
                event.key === 'Home' ? 0 :
                event.key === 'End' ? duration :
                null;
              if (time === null) return;
              event.preventDefault();
              const clamped = Math.max(0, Math.min(duration, time));
              const speaker = meetingSpeakers.speakerAt(clamped);
              if (speaker) select(speaker, clamped);
            }}
          >
            <div className="relative h-full w-1 overflow-hidden rounded-full bg-slate-100 transition-[width] group-hover:w-1.5 group-focus-visible:ring-2 group-focus-visible:ring-indigo-400">
              {duration > 0 && stats.turns.map((turn, index) => (
                <div
                  key={`${turn.start}-${index}`}
                  className="absolute inset-x-0"
                  style={{
                    top: `${(turn.start / duration) * 100}%`,
                    height: `max(1px, ${((turn.end - turn.start) / duration) * 100}%)`,
                    backgroundColor: meetingSpeakers.colorFor(turn.speaker) ?? NO_COLOR,
                    opacity: speakingNow && turn.speaker !== speakingNow ? 0.45 : 1,
                  }}
                />
              ))}
            </div>
            {started && duration > 0 && (
              <div
                aria-hidden
                className="pointer-events-none absolute left-1/2 h-2.5 w-2.5 -translate-x-1/2 -translate-y-1/2 rounded-full bg-indigo-600 ring-2 ring-white"
                style={{ top: `${Math.min(100, (currentTime / duration) * 100)}%` }}
              />
            )}
            {hover && (
              <div
                aria-hidden
                className="pointer-events-none absolute left-5 z-10 -translate-y-1/2 whitespace-nowrap rounded-md bg-slate-900 px-2 py-1 font-sans text-[11px] text-white shadow-md"
                style={{ top: hover.y }}
              >
                <span className="font-mono tabular-nums">{formatPlaybackTime(hover.time)}</span>
                {hover.speaker && <span className="text-slate-300"> · {meetingSpeakers.displayName(hover.speaker)}</span>}
              </div>
            )}
          </div>
          <span>{formatPlaybackTime(duration)}</span>
        </div>

        <ul className="-mr-2 min-w-0 flex-1 space-y-0.5 overflow-y-auto pr-2">
          {stats.speakers.map(({ speaker, seconds, share }) => {
            const name = meetingSpeakers.displayName(speaker);
            const color = meetingSpeakers.colorFor(speaker) ?? NO_COLOR;
            const speaking = speakingTurn?.speaker === speaker;
            return (
              <li key={speaker} className="group flex items-center rounded-lg transition-colors hover:bg-slate-50">
                <button
                  type="button"
                  onClick={() => {
                    const turn = nextTurnOf(speaker);
                    if (turn) select(speaker, turn.start);
                  }}
                  title={t('Go to the next turn of {speaker}', { speaker: name })}
                  className="flex min-w-0 flex-1 items-center gap-2.5 rounded-lg py-1.5 pl-2 text-left focus:outline-none focus-visible:ring-2 focus-visible:ring-indigo-400"
                >
                  <span className="relative h-2 w-2 shrink-0 rounded-full" style={{ backgroundColor: color }}>
                    {speaking && playback?.isPlaying && (
                      <span className="absolute inset-0 animate-ping rounded-full opacity-60" style={{ backgroundColor: color }} />
                    )}
                  </span>
                  <span className={`min-w-0 flex-1 truncate text-sm ${speaking ? 'font-semibold text-indigo-700' : 'text-slate-800'}`}>{name}</span>
                  <span className="shrink-0 text-xs tabular-nums text-slate-400">
                    {formatTalkTime(seconds, t)} · {Math.round(share * 100)}%
                  </span>
                </button>
                <SpeakerRenamePopover speaker={speaker}>
                  <button
                    type="button"
                    title={t('Rename {speaker}', { speaker: name })}
                    aria-label={t('Rename {speaker}', { speaker: name })}
                    className="mx-1 grid h-6 w-6 shrink-0 place-items-center rounded-md text-slate-400 opacity-0 transition-opacity hover:bg-slate-100 hover:text-slate-700 focus:opacity-100 group-hover:opacity-100 data-[state=open]:opacity-100"
                  >
                    <Pencil className="h-3 w-3" />
                  </button>
                </SpeakerRenamePopover>
              </li>
            );
          })}
        </ul>
      </div>
    </aside>
  );
}
