'use client';

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useMeetingSpeakers } from '@/contexts/MeetingSpeakersContext';
import { useI18n } from '@/lib/i18n';
import { SpeakerName } from './SpeakerName';

const HEIGHT_STORAGE_KEY = 'meetily.meetingDetails.speakerTimelineHeight';
const MIN_HEIGHT = 96;
const KEYBOARD_STEP = 24;

// A null height means "as tall as the speakers need"; a stored number is the
// user's own choice, which has to survive a window that later got shorter.
function maxHeight(): number {
  if (typeof window === 'undefined') return MIN_HEIGHT;
  return Math.max(MIN_HEIGHT, Math.round(window.innerHeight * 0.7));
}

function clampHeight(value: number): number {
  return Math.min(maxHeight(), Math.max(MIN_HEIGHT, Math.round(value)));
}

function readStoredHeight(): number | null {
  if (typeof window === 'undefined') return null;
  try {
    const raw = localStorage.getItem(HEIGHT_STORAGE_KEY);
    const parsed = raw == null ? NaN : Number(raw);
    return Number.isFinite(parsed) ? clampHeight(parsed) : null;
  } catch {
    return null;
  }
}

function writeStoredHeight(value: number): void {
  try {
    localStorage.setItem(HEIGHT_STORAGE_KEY, String(value));
  } catch {
    // Layout persistence is optional.
  }
}

function formatTime(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds));
  const minutes = Math.floor(total / 60);
  const hours = Math.floor(minutes / 60);
  return hours ? `${hours}:${String(minutes % 60).padStart(2, '0')}:${String(total % 60).padStart(2, '0')}`
    : `${minutes}:${String(total % 60).padStart(2, '0')}`;
}

/** Speaker turns of the meeting in `MeetingSpeakersProvider`, one row per speaker. */
export function SpeakerTimeline({
  onSelectTurn,
}: {
  onSelectTurn?: (speaker: string, time: number) => void;
}) {
  const { t } = useI18n();
  const meetingSpeakers = useMeetingSpeakers();
  const turns = useMemo(() => meetingSpeakers?.turns ?? [], [meetingSpeakers?.turns]);
  const inProgress = meetingSpeakers?.inProgress ?? false;
  const sectionRef = useRef<HTMLElement>(null);
  const [height, setHeight] = useState<number | null>(null);
  const dragging = useRef(false);

  useEffect(() => {
    setHeight(readStoredHeight());
  }, []);

  useEffect(() => {
    const onResize = () => setHeight((current) => (current === null ? null : clampHeight(current)));
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, []);

  const measuredHeight = useCallback(
    () => height ?? sectionRef.current?.getBoundingClientRect().height ?? MIN_HEIGHT,
    [height],
  );

  const onSeparatorPointerDown = useCallback((event: React.PointerEvent) => {
    event.preventDefault();
    dragging.current = true;
    event.currentTarget.setPointerCapture(event.pointerId);
  }, []);

  const onSeparatorPointerMove = useCallback((event: React.PointerEvent) => {
    if (!dragging.current || !sectionRef.current) return;
    setHeight(clampHeight(event.clientY - sectionRef.current.getBoundingClientRect().top));
  }, []);

  const onSeparatorPointerUp = useCallback(() => {
    if (!dragging.current) return;
    dragging.current = false;
    setHeight((current) => {
      if (current !== null) writeStoredHeight(current);
      return current;
    });
  }, []);

  const onSeparatorKeyDown = useCallback((event: React.KeyboardEvent) => {
    const current = measuredHeight();
    const next =
      event.key === 'ArrowUp' ? clampHeight(current - KEYBOARD_STEP) :
      event.key === 'ArrowDown' ? clampHeight(current + KEYBOARD_STEP) :
      event.key === 'Home' ? MIN_HEIGHT :
      event.key === 'End' ? maxHeight() :
      null;
    if (next === null) return;
    event.preventDefault();
    setHeight(next);
    writeStoredHeight(next);
  }, [measuredHeight]);

  const timeline = useMemo(() => {
    const valid = turns.filter((turn) => Number.isFinite(turn.start) && Number.isFinite(turn.end)
      && turn.start >= 0 && turn.end > turn.start && turn.speaker.trim());
    const duration = Math.max(0, ...valid.map((turn) => turn.end));
    const speakers = Array.from(new Set(valid.map((turn) => turn.speaker)));
    return { valid, duration, speakers };
  }, [turns]);

  if (!meetingSpeakers || !timeline.valid.length || inProgress) return null;

  return (
    <>
    <section
      ref={sectionRef}
      aria-label={t('Speaker timeline')}
      className="flex shrink-0 flex-col overflow-hidden bg-white px-8 pt-4"
      style={height === null ? undefined : { height }}
    >
      <div className="mb-3 flex shrink-0 items-center justify-between gap-3">
        <h3 className="text-sm font-semibold text-gray-900">{t('Speaker timeline')}</h3>
      </div>
      <div className="min-h-0 flex-1 overflow-auto pb-3">
        <div className="min-w-[440px] space-y-2">
          <div className="grid grid-cols-[180px_minmax(0,1fr)] gap-3 text-[10px] text-gray-500">
            <span />
            <div className="flex justify-between">
              {[0, 0.25, 0.5, 0.75, 1].map((fraction) => (
                <span key={fraction}>{formatTime(timeline.duration * fraction)}</span>
              ))}
            </div>
          </div>
          {timeline.speakers.map((speaker) => (
            <div key={speaker} className="grid grid-cols-[180px_minmax(0,1fr)] items-center gap-3">
              <SpeakerName speaker={speaker} className="text-xs" />
              <div className="relative h-6 overflow-hidden rounded-md bg-gray-100" aria-label={t('{speaker} speech intervals', { speaker: meetingSpeakers.displayName(speaker) })}>
                {timeline.valid.filter((turn) => turn.speaker === speaker).map((turn, turnIndex) => (
                  <button
                    key={`${turn.start}-${turn.end}-${turnIndex}`}
                    type="button"
                    className="absolute top-1 h-4 cursor-pointer rounded-sm transition-opacity hover:opacity-75 focus-visible:z-10 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-blue-700"
                    style={{
                      left: `${turn.start / timeline.duration * 100}%`,
                      width: `${(turn.end - turn.start) / timeline.duration * 100}%`,
                      backgroundColor: meetingSpeakers.colorFor(speaker),
                    }}
                    title={`${meetingSpeakers.displayName(speaker)}: ${formatTime(turn.start)}–${formatTime(turn.end)}`}
                    aria-label={t('Show {speaker} transcript at {start} to {end}', { speaker: meetingSpeakers.displayName(speaker), start: formatTime(turn.start), end: formatTime(turn.end) })}
                    onClick={(event) => {
                      const rect = event.currentTarget.getBoundingClientRect();
                      const fraction = event.detail === 0 || rect.width === 0
                        ? 0
                        : Math.max(0, Math.min(1, (event.clientX - rect.left) / rect.width));
                      onSelectTurn?.(speaker, turn.start + fraction * (turn.end - turn.start));
                    }}
                  />
                ))}
              </div>
            </div>
          ))}
        </div>
      </div>
    </section>
    <div
      role="separator"
      aria-orientation="horizontal"
      aria-label={t('Resize speaker timeline')}
      aria-valuenow={Math.round(measuredHeight())}
      aria-valuemin={MIN_HEIGHT}
      aria-valuemax={maxHeight()}
      tabIndex={0}
      className="group relative z-10 flex h-2 w-full shrink-0 cursor-row-resize items-center justify-stretch focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-500 focus-visible:ring-inset"
      onPointerDown={onSeparatorPointerDown}
      onPointerMove={onSeparatorPointerMove}
      onPointerUp={onSeparatorPointerUp}
      onPointerCancel={onSeparatorPointerUp}
      onKeyDown={onSeparatorKeyDown}
    >
      <div className="h-px w-full bg-gray-200 transition-[height,background-color] duration-150 ease-out group-hover:h-1 group-hover:bg-blue-400 group-active:h-1 group-active:bg-blue-500" />
    </div>
    </>
  );
}
