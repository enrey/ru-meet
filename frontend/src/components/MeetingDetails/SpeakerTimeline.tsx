'use client';

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Check, Pencil, X } from 'lucide-react';
import { toast } from 'sonner';
import type { SpeakerTurn } from '@/services/storageService';

const COLORS = ['#6366f1', '#06b6d4', '#f59e0b', '#ec4899', '#10b981', '#8b5cf6'];

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

export function SpeakerTimeline({
  meetingId,
  onSelectTurn,
  onSpeakerRenamed,
}: {
  meetingId: string;
  onSelectTurn?: (speaker: string, time: number) => void;
  onSpeakerRenamed?: (oldName: string, newName: string) => void;
}) {
  const [turns, setTurns] = useState<SpeakerTurn[]>([]);
  const [inProgress, setInProgress] = useState(false);
  const [editingSpeaker, setEditingSpeaker] = useState<string | null>(null);
  const [draftName, setDraftName] = useState('');
  const [isSaving, setIsSaving] = useState(false);
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

  const cancelEdit = () => {
    if (isSaving) return;
    setEditingSpeaker(null);
    setDraftName('');
  };

  const saveEdit = async () => {
    if (!editingSpeaker || isSaving) return;
    const name = draftName.trim();
    if (!name || name.length > 80) {
      toast.error('Speaker name must be between 1 and 80 characters');
      return;
    }
    if (name === editingSpeaker) {
      cancelEdit();
      return;
    }
    setIsSaving(true);
    try {
      await invoke('rename_meeting_speaker', { meetingId, oldName: editingSpeaker, newName: name });
      setTurns((current) => current.map((turn) => turn.speaker === editingSpeaker ? { ...turn, speaker: name } : turn));
      onSpeakerRenamed?.(editingSpeaker, name);
      setEditingSpeaker(null);
      setDraftName('');
    } catch (error) {
      toast.error('Could not rename speaker', { description: String(error) });
    } finally {
      setIsSaving(false);
    }
  };

  useEffect(() => {
    let active = true;
    const load = () => invoke<SpeakerTurn[]>('get_meeting_speaker_turns', { meetingId })
      .then((value) => { if (active) setTurns(value); })
      .catch((error) => console.warn('Failed to load speaker timeline:', error));

    setTurns([]);
    setEditingSpeaker(null);
    void load();
    void invoke<{ inProgress: boolean; meetingId?: string }>('get_diarization_status')
      .then((status) => { if (active) setInProgress(status.inProgress && status.meetingId === meetingId); })
      .catch((error) => console.warn('Failed to load diarization status:', error));
    const subscriptions = Promise.all([
      listen<{ meetingId?: string }>('diarization-progress', ({ payload }) => {
        if (payload.meetingId === meetingId) setInProgress(true);
      }),
      listen<{ meetingId?: string }>('diarization-labels-saved', ({ payload }) => {
        if (!payload.meetingId || payload.meetingId === meetingId) {
          setInProgress(false);
          void load();
        }
      }),
      listen<{ meetingId?: string }>('diarization-rerun-error', ({ payload }) => {
        if (payload.meetingId === meetingId) setInProgress(false);
      }),
      listen<{ meetingId?: string }>('diarization-cancelled', ({ payload }) => {
        if (payload.meetingId === meetingId) setInProgress(false);
      }),
    ]);
    return () => {
      active = false;
      void subscriptions.then((listeners) => listeners.forEach((unlisten) => unlisten()));
    };
  }, [meetingId]);

  const timeline = useMemo(() => {
    const valid = turns.filter((turn) => Number.isFinite(turn.start) && Number.isFinite(turn.end)
      && turn.start >= 0 && turn.end > turn.start && turn.speaker.trim());
    const duration = Math.max(0, ...valid.map((turn) => turn.end));
    const speakers = Array.from(new Set(valid.map((turn) => turn.speaker)));
    return { valid, duration, speakers };
  }, [turns]);

  if (!timeline.valid.length || inProgress) return null;

  return (
    <>
    <section
      ref={sectionRef}
      aria-label="Speaker timeline"
      className="flex shrink-0 flex-col overflow-hidden bg-white px-4 pt-3"
      style={height === null ? undefined : { height }}
    >
      <div className="mb-3 flex shrink-0 items-center justify-between gap-3">
        <h3 className="text-sm font-semibold text-gray-900">Speaker timeline</h3>
        <span className="text-xs text-gray-500">{formatTime(timeline.duration)}</span>
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
          {timeline.speakers.map((speaker, index) => (
            <div key={speaker} className="grid grid-cols-[180px_minmax(0,1fr)] items-center gap-3">
              {editingSpeaker === speaker ? (
                <div className="flex min-w-0 items-center gap-1">
                  <input
                    autoFocus
                    type="text"
                    value={draftName}
                    maxLength={80}
                    aria-label={`Rename ${speaker}`}
                    className="min-w-0 flex-1 rounded border border-blue-400 px-1.5 py-0.5 text-xs outline-none focus:ring-1 focus:ring-blue-500"
                    onChange={(event) => setDraftName(event.target.value)}
                    onFocus={(event) => event.currentTarget.select()}
                    onKeyDown={(event) => {
                      if (event.key === 'Enter') { event.preventDefault(); void saveEdit(); }
                      if (event.key === 'Escape') { event.preventDefault(); cancelEdit(); }
                    }}
                    disabled={isSaving}
                  />
                  <button type="button" aria-label="Save speaker name" title="Save" disabled={isSaving}
                    className="text-green-700 disabled:opacity-50" onClick={() => void saveEdit()}><Check size={16} /></button>
                  <button type="button" aria-label="Cancel speaker rename" title="Cancel" disabled={isSaving}
                    className="text-gray-500 disabled:opacity-50" onClick={cancelEdit}><X size={16} /></button>
                </div>
              ) : (
                <button type="button" title={`Rename ${speaker}`} aria-label={`Rename ${speaker}`}
                  className="group flex min-w-0 items-center gap-1 text-left text-xs font-medium text-gray-700 hover:text-blue-700"
                  onClick={() => { setEditingSpeaker(speaker); setDraftName(speaker); }}>
                  <span className="truncate">{speaker}</span><Pencil size={12} className="shrink-0 opacity-0 group-hover:opacity-100 group-focus-visible:opacity-100" />
                </button>
              )}
              <div className="relative h-6 overflow-hidden rounded-md bg-gray-100" aria-label={`${speaker} speech intervals`}>
                {timeline.valid.filter((turn) => turn.speaker === speaker).map((turn, turnIndex) => (
                  <button
                    key={`${turn.start}-${turn.end}-${turnIndex}`}
                    type="button"
                    className="absolute top-1 h-4 cursor-pointer rounded-sm transition-opacity hover:opacity-75 focus-visible:z-10 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-blue-700"
                    style={{
                      left: `${turn.start / timeline.duration * 100}%`,
                      width: `${(turn.end - turn.start) / timeline.duration * 100}%`,
                      backgroundColor: COLORS[index % COLORS.length],
                    }}
                    title={`${speaker}: ${formatTime(turn.start)}–${formatTime(turn.end)}`}
                    aria-label={`Show ${speaker} transcript at ${formatTime(turn.start)} to ${formatTime(turn.end)}`}
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
      aria-label="Resize speaker timeline"
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
