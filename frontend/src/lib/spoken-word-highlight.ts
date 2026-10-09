'use client';

import { useEffect, useRef } from 'react';
import type { PlaybackControls } from '@/contexts/MeetingPlaybackContext';

/**
 * Karaoke: the word being spoken gets a pale yellow background (see
 * `::highlight(...)` in globals.css). Done with the CSS Custom Highlight API,
 * which paints over text without touching the DOM - so it works inside the
 * summary editor too. Where the API is missing there is simply no highlight.
 */

export type HighlightName = 'spoken-word-summary' | 'spoken-word-transcript';

function registry(): HighlightRegistry | null {
  return typeof CSS !== 'undefined' && 'highlights' in CSS ? CSS.highlights : null;
}

export function setSpokenWord(name: HighlightName, range: Range | null) {
  const highlights = registry();
  if (!highlights) return;
  if (range) highlights.set(name, new Highlight(range));
  else highlights.delete(name);
}

/** How a word is compared: case, `ё` and punctuation do not count. */
export function normalizeWord(token: string): string {
  return token.toLowerCase().replace(/ё/g, 'е').replace(/[^\p{L}\p{N}]/gu, '');
}

export interface TextWord {
  text: string;
  /** The word without the punctuation around it. */
  range: () => Range;
}

/** Whitespace-separated words of the text under `root`, in reading order. */
export function wordsIn(root: Node, accept?: (node: Text) => boolean): TextWord[] {
  const words: TextWord[] = [];
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode() as Text | null; node; node = walker.nextNode() as Text | null) {
    if (accept && !accept(node)) continue;
    const textNode = node;
    for (const match of textNode.data.matchAll(/\S+/g)) {
      const token = match[0];
      const lead = token.search(/[\p{L}\p{N}]/u);
      if (lead < 0) continue;
      const last = token.search(/[\p{L}\p{N}][^\p{L}\p{N}]*$/u);
      const start = (match.index ?? 0) + lead;
      const end = (match.index ?? 0) + Math.max(last, lead) + 1;
      words.push({
        text: token,
        range: () => {
          const range = document.createRange();
          range.setStart(textNode, start);
          range.setEnd(textNode, Math.min(end, textNode.length));
          return range;
        },
      });
    }
  }
  return words;
}

/**
 * For every spoken word, the index of the written word it reads, or -1.
 * Greedy and in order, so text the reading skipped (a table, say) or words
 * written differently only cost the words involved.
 */
export function alignWords(spoken: string[], written: string[], window = 200): number[] {
  const result: number[] = [];
  let cursor = 0;
  for (const word of spoken) {
    let found = -1;
    if (word) {
      for (let at = cursor; at < Math.min(written.length, cursor + window); at += 1) {
        if (written[at] === word) {
          found = at;
          break;
        }
      }
    }
    result.push(found);
    if (found >= 0) cursor = found + 1;
  }
  return result;
}

/**
 * Re-run `update` with the playback position on every frame while playing,
 * and once whenever it is paused at a new spot. `update(null)` means nothing
 * is being spoken.
 */
export function usePlaybackFrames(
  playback: PlaybackControls | null,
  active: boolean,
  update: (time: number | null) => void,
) {
  const updateRef = useRef(update);
  updateRef.current = update;
  const isPlaying = Boolean(playback?.isPlaying);
  // While playing, frames read the position themselves.
  const pausedAt = isPlaying ? -1 : playback?.currentTime ?? 0;
  const getTime = playback?.getTime;

  useEffect(() => {
    if (!active || !getTime) {
      updateRef.current(null);
      return;
    }
    if (!isPlaying) {
      updateRef.current(pausedAt > 0 ? pausedAt : null);
      return;
    }
    let frame = 0;
    const tick = () => {
      updateRef.current(getTime());
      frame = requestAnimationFrame(tick);
    };
    tick();
    return () => cancelAnimationFrame(frame);
  }, [active, isPlaying, getTime, pausedAt]);

  useEffect(() => () => updateRef.current(null), []);
}

/** Index of the last item starting at or before `time`, or -1. */
export function lastStartingBy<T>(items: T[], time: number, startOf: (item: T) => number): number {
  let low = 0;
  let high = items.length - 1;
  let found = -1;
  while (low <= high) {
    const mid = (low + high) >> 1;
    if (startOf(items[mid]) <= time) {
      found = mid;
      low = mid + 1;
    } else {
      high = mid - 1;
    }
  }
  return found;
}

/** Relative speaking time of a written word: its letters, plus a pause after punctuation. */
export function wordWeight(token: string): { weight: number; pause: number } {
  const letters = normalizeWord(token).length;
  const last = token[token.length - 1];
  const pause = '.!?…'.includes(last) ? 4 : ',;:'.includes(last) ? 2.5 : 0;
  return { weight: letters + 1, pause };
}
