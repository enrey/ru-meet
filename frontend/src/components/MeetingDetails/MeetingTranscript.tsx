'use client';

import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';
import { Check, Copy, Play } from 'lucide-react';
import { TranscriptSegmentData } from '@/types';
import { formatPlaybackTime, useMeetingPlayback } from '@/contexts/MeetingPlaybackContext';
import { useI18n } from '@/lib/i18n';
import { SpeakerAvatar, SpeakerName } from './SpeakerName';

export interface TranscriptScrollTarget {
  id: string;
  /** Bumped by every jump, so jumping to the same line again scrolls again. */
  request: number;
  /** Don't mark the line: the player's tint already shows it. */
  quiet?: boolean;
}

/** A run of lines by one speaker, shown as one turn. */
interface Turn {
  key: string;
  speaker?: string;
  start: number;
  segments: TranscriptSegmentData[];
}

// Long monologues still break into several turns so each stays readable.
const MAX_SEGMENTS_PER_TURN = 12;
// After the reader scrolls, playback leaves the transcript alone for a while.
const USER_SCROLL_GRACE_MS = 4000;

function groupTurns(segments: TranscriptSegmentData[]): Turn[] {
  const turns: Turn[] = [];
  for (const segment of segments) {
    const last = turns[turns.length - 1];
    if (last && segment.speaker && last.speaker === segment.speaker && last.segments.length < MAX_SEGMENTS_PER_TURN) {
      last.segments.push(segment);
    } else {
      turns.push({ key: segment.id, speaker: segment.speaker, start: segment.timestamp, segments: [segment] });
    }
  }
  return turns;
}

/** The loaded line playing at `time`, or null when it is outside the loaded window. */
function segmentAt(segments: TranscriptSegmentData[], time: number, hasMore: boolean): string | null {
  let low = 0;
  let high = segments.length - 1;
  let found = -1;
  while (low <= high) {
    const mid = (low + high) >> 1;
    if (segments[mid].timestamp <= time) {
      found = mid;
      low = mid + 1;
    } else {
      high = mid - 1;
    }
  }
  if (found < 0) return null;
  const segment = segments[found];
  if (found === segments.length - 1 && hasMore && time > (segment.endTime ?? segment.timestamp) + 1) return null;
  return segment.id;
}

/** `text` with every case-insensitive occurrence of `query` wrapped in <mark>. */
function highlightMatches(text: string, query: string | undefined): React.ReactNode {
  const needle = query?.trim().toLowerCase();
  if (!needle) return text;
  const haystack = text.toLowerCase();
  // Lower-casing can change the length of a few exotic characters; then the
  // indices would not line up with the original, so don't highlight at all.
  if (haystack.length !== text.length) return text;
  const parts: React.ReactNode[] = [];
  let from = 0;
  for (let at = haystack.indexOf(needle); at >= 0; at = haystack.indexOf(needle, at + needle.length)) {
    if (at > from) parts.push(text.slice(from, at));
    parts.push(<mark key={at} className="rounded-sm bg-yellow-200 px-0.5 text-inherit">{text.slice(at, at + needle.length)}</mark>);
    from = at + needle.length;
  }
  if (from === 0) return text;
  parts.push(text.slice(from));
  return parts;
}

function cleanText(text: string, silence: string): string {
  const cleaned = text.replace(/\b(uh|um|er|ah|hmm|hm|eh|oh)\b[,\s]*/gi, ' ').replace(/\s+/g, ' ').trim();
  return cleaned || (text.trim() === '' ? `[${silence}]` : text);
}

const TurnRow = memo(function TurnRow({
  turn,
  activeId,
  isPlaying,
  targetId,
  canPlay,
  onPlay,
  highlightQuery,
}: {
  turn: Turn;
  /** The line playing now, when it belongs to this turn. */
  activeId: string | null;
  isPlaying: boolean;
  /** The line a jump or search landed on, when it belongs to this turn. */
  targetId: string | null;
  canPlay: boolean;
  onPlay: (time: number) => void;
  highlightQuery?: string;
}) {
  const { t } = useI18n();
  const [copied, setCopied] = useState(false);
  const active = activeId !== null;
  const silence = t('Silence');

  const copy = () => {
    const text = turn.segments.map((segment) => cleanText(segment.text, silence)).join(' ');
    void navigator.clipboard?.writeText(text).then(() => {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1200);
    });
  };

  return (
    <div className={`group/turn flex items-start gap-3.5 rounded-xl px-3.5 py-3 transition-colors ${active ? 'bg-indigo-50/70' : 'hover:bg-slate-50'}`}>
      {turn.speaker ? <SpeakerAvatar speaker={turn.speaker} active={active} /> : <span className="w-7 shrink-0" />}
      <div className="min-w-0 flex-1">
        <div className="mb-1 flex min-h-5 items-center gap-2">
          {turn.speaker && <SpeakerName speaker={turn.speaker} showDot={false} className={`text-sm ${active ? '!text-indigo-700' : ''}`} />}
          <button
            type="button"
            disabled={!canPlay}
            onClick={() => onPlay(turn.start)}
            title={canPlay ? t('Play from {time}', { time: formatPlaybackTime(turn.start) }) : undefined}
            className={`rounded-sm font-mono text-xs tabular-nums transition-colors enabled:hover:text-indigo-600 focus:outline-none focus-visible:ring-1 focus-visible:ring-indigo-500 ${active ? 'font-medium text-indigo-600' : 'text-slate-400'}`}
          >
            {formatPlaybackTime(turn.start)}
          </button>
          {active && isPlaying && (
            <span aria-hidden className="ml-1 inline-flex h-3 items-end gap-0.5">
              <span className="h-1.5 w-[3px] animate-pulse rounded-full bg-indigo-500" />
              <span className="h-3 w-[3px] animate-pulse rounded-full bg-indigo-500 [animation-delay:150ms]" />
              <span className="h-2 w-[3px] animate-pulse rounded-full bg-indigo-500 [animation-delay:300ms]" />
            </span>
          )}
        </div>
        <p className="text-[15px] leading-relaxed text-slate-800">
          {turn.segments.map((segment, index) => {
            const playing = segment.id === activeId;
            const target = segment.id === targetId;
            return (
              <span key={segment.id}>
                {index > 0 && ' '}
                <span
                  id={`segment-${segment.id}`}
                  onClick={canPlay ? () => {
                    // Selecting text to copy it must not start playback.
                    if (window.getSelection()?.isCollapsed !== false) onPlay(segment.timestamp);
                  } : undefined}
                  className={`rounded-sm box-decoration-clone transition-colors ${canPlay ? 'cursor-pointer hover:text-slate-950' : ''} ${
                    playing ? 'bg-indigo-100/80 text-slate-950' : target ? 'bg-indigo-50 ring-1 ring-indigo-300' : ''
                  }`}
                >
                  {highlightMatches(cleanText(segment.text, silence), highlightQuery)}
                </span>
              </span>
            );
          })}
        </p>
      </div>
      <div className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity focus-within:opacity-100 group-hover/turn:opacity-100">
        {canPlay && (
          <button
            type="button"
            onClick={() => onPlay(turn.start)}
            title={t('Play from {time}', { time: formatPlaybackTime(turn.start) })}
            aria-label={t('Play from {time}', { time: formatPlaybackTime(turn.start) })}
            className="grid h-7 w-7 place-items-center rounded-md text-slate-400 hover:bg-white hover:text-indigo-600"
          >
            <Play className="h-3.5 w-3.5" />
          </button>
        )}
        <button
          type="button"
          onClick={copy}
          title={t('Copy')}
          aria-label={t('Copy')}
          className="grid h-7 w-7 place-items-center rounded-md text-slate-400 hover:bg-white hover:text-slate-700"
        >
          {copied ? <Check className="h-3.5 w-3.5 text-emerald-600" /> : <Copy className="h-3.5 w-3.5" />}
        </button>
      </div>
    </div>
  );
});

/**
 * Transcript of a saved meeting as speaker turns. Follows the meeting player:
 * the playing line is tinted and kept in view unless the reader has just scrolled.
 */
export function MeetingTranscript({
  segments,
  hasMore = false,
  isLoadingMore = false,
  hasPrevious = false,
  isLoadingPrevious = false,
  onLoadMore,
  onLoadPrevious,
  scrollTarget,
  highlightQuery,
}: {
  segments: TranscriptSegmentData[];
  hasMore?: boolean;
  isLoadingMore?: boolean;
  hasPrevious?: boolean;
  isLoadingPrevious?: boolean;
  onLoadMore?: () => void;
  onLoadPrevious?: () => void;
  scrollTarget?: TranscriptScrollTarget | null;
  highlightQuery?: string;
}) {
  const { t } = useI18n();
  const playback = useMeetingPlayback();
  const scrollRef = useRef<HTMLDivElement>(null);
  const lastAppliedJumpRef = useRef<string | null>(null);
  const lastUserScrollRef = useRef(0);
  const followedTurnRef = useRef<number | null>(null);

  const turns = useMemo(() => groupTurns(segments), [segments]);
  const turnIndexById = useMemo(() => {
    const map = new Map<string, number>();
    turns.forEach((turn, index) => turn.segments.forEach((segment) => map.set(segment.id, index)));
    return map;
  }, [turns]);

  const virtualizer = useVirtualizer({
    count: turns.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 96,
    overscan: 6,
    getItemKey: (index) => turns[index].key,
  });

  const canPlay = Boolean(playback?.isAvailable);
  const isPlaying = Boolean(playback?.isPlaying);
  const currentTime = playback?.currentTime ?? 0;
  const started = isPlaying || currentTime > 0;
  const activeId = started ? segmentAt(segments, currentTime, hasMore) : null;
  const activeTurn = activeId ? turnIndexById.get(activeId) ?? null : null;

  // Stable across player ticks so the memoized rows don't re-render with them.
  const playbackRef = useRef(playback);
  playbackRef.current = playback;
  const play = useCallback((time: number) => playbackRef.current?.play(time), []);

  // Jumps from search and the speaker timeline.
  useEffect(() => {
    if (!scrollTarget) return;
    const key = `${scrollTarget.request}:${scrollTarget.id}`;
    if (lastAppliedJumpRef.current === key) return;
    const index = turnIndexById.get(scrollTarget.id);
    if (index === undefined) return;
    lastAppliedJumpRef.current = key;
    followedTurnRef.current = activeTurn;
    virtualizer.scrollToIndex(index, { align: 'center' });
  }, [scrollTarget, turnIndexById, virtualizer, activeTurn]);

  // Keep the playing turn in view.
  useEffect(() => {
    if (!isPlaying || activeTurn === null || followedTurnRef.current === activeTurn) return;
    followedTurnRef.current = activeTurn;
    if (Date.now() - lastUserScrollRef.current < USER_SCROLL_GRACE_MS) return;
    virtualizer.scrollToIndex(activeTurn, { align: 'center' });
  }, [isPlaying, activeTurn, virtualizer]);

  // Playback ran past the loaded lines: fetch the next page.
  useEffect(() => {
    if (!isPlaying || activeId !== null || !hasMore || isLoadingMore || !segments.length) return;
    if (currentTime > segments[segments.length - 1].timestamp) onLoadMore?.();
  }, [isPlaying, activeId, hasMore, isLoadingMore, segments, currentTime, onLoadMore]);

  // Infinite scroll, also when the first page does not fill the view.
  const maybeLoadMore = useCallback(() => {
    const element = scrollRef.current;
    if (!element || !hasMore || isLoadingMore) return;
    if (element.scrollHeight - element.scrollTop - element.clientHeight < 300) onLoadMore?.();
  }, [hasMore, isLoadingMore, onLoadMore]);

  useEffect(() => {
    maybeLoadMore();
  }, [maybeLoadMore, turns.length]);

  const markUserScroll = () => {
    lastUserScrollRef.current = Date.now();
  };

  if (!segments.length) {
    return <p className="mt-10 text-center text-sm text-slate-500">{t('No transcript yet')}</p>;
  }

  return (
    <div
      ref={scrollRef}
      className="h-full overflow-y-auto px-5 pb-6 pt-1"
      onScroll={maybeLoadMore}
      onWheel={markUserScroll}
      onTouchMove={markUserScroll}
      onPointerDown={(event) => {
        // Dragging the scrollbar: the pointer lands on the container itself.
        if (event.target === event.currentTarget) markUserScroll();
      }}
      onKeyDown={(event) => {
        if (['PageUp', 'PageDown', 'ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) markUserScroll();
      }}
    >
      {hasPrevious && (
        <div className="flex justify-center pb-2">
          <button
            type="button"
            className="rounded-md px-3 py-1 text-sm text-indigo-600 hover:bg-indigo-50 disabled:text-slate-400"
            disabled={isLoadingPrevious}
            onClick={onLoadPrevious}
          >
            {isLoadingPrevious ? t('Loading earlier…') : t('Load earlier transcript')}
          </button>
        </div>
      )}
      <div className="relative w-full" style={{ height: virtualizer.getTotalSize() }}>
        {virtualizer.getVirtualItems().map((item) => {
          const turn = turns[item.index];
          const ownsActive = activeTurn === item.index;
          const marked = scrollTarget && (!scrollTarget.quiet || !canPlay) ? scrollTarget : null;
          const target = marked && turnIndexById.get(marked.id) === item.index ? marked.id : null;
          return (
            <div
              key={item.key}
              data-index={item.index}
              ref={virtualizer.measureElement}
              className="absolute left-0 top-0 w-full pb-1"
              style={{ transform: `translateY(${item.start}px)` }}
            >
              <TurnRow
                turn={turn}
                activeId={ownsActive ? activeId : null}
                isPlaying={isPlaying}
                targetId={target}
                canPlay={canPlay}
                onPlay={play}
                highlightQuery={highlightQuery}
              />
            </div>
          );
        })}
      </div>
      {(hasMore || isLoadingMore) && (
        <div className="flex items-center justify-center gap-2 py-4 text-sm text-slate-500">
          {isLoadingMore && <span className="h-4 w-4 animate-spin rounded-full border-2 border-slate-300 border-t-slate-600" />}
          {isLoadingMore ? t('Loading more...') : null}
        </div>
      )}
    </div>
  );
}
