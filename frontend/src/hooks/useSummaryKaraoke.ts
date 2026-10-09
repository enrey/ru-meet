'use client';

import { useRef } from 'react';
import type { PlaybackControls } from '@/contexts/MeetingPlaybackContext';
import type { SpokenWord } from '@/hooks/useSummaryAudio';
import {
  alignWords,
  lastStartingBy,
  normalizeWord,
  setSpokenWord,
  usePlaybackFrames,
  wordsIn,
  type TextWord,
} from '@/lib/spoken-word-highlight';

const NAME = 'spoken-word-summary';

/**
 * Highlight the word of the summary being read aloud, and keep its block in
 * view while `follow`. The reading's words are matched to the editor's text
 * in order, so formatting, list markers and edits the reading predates only
 * lose the words they touch.
 */
export function useSummaryKaraoke({
  playback,
  words,
  container,
  follow,
}: {
  /** The summary track; idle unless it is loaded in the player. */
  playback: PlaybackControls | null;
  words: SpokenWord[];
  /** CSS selector of the element holding the summary editor. */
  container: string;
  follow: boolean;
}) {
  const alignmentRef = useRef<{ words: SpokenWord[]; text: string; map: number[] } | null>(null);
  const shownRef = useRef<{ index: number; block: Element | null }>({ index: -1, block: null });
  const followRef = useRef(follow);
  followRef.current = follow;

  const clear = () => {
    if (shownRef.current.index === -1) return;
    shownRef.current = { index: -1, block: null };
    setSpokenWord(NAME, null);
  };

  usePlaybackFrames(playback, words.length > 0, (time) => {
    if (time === null) return clear();
    const index = lastStartingBy(words, time, (word) => word.start);
    if (index < 0) return clear();
    if (index === shownRef.current.index) return;

    const root = document.querySelector(`${container} .bn-editor`);
    if (!root) return clear();
    const written: TextWord[] = wordsIn(root, (node) => Boolean(node.parentElement?.closest('.bn-inline-content')));
    const text = written.map((word) => word.text).join(' ');
    let alignment = alignmentRef.current;
    if (!alignment || alignment.words !== words || alignment.text !== text) {
      alignment = {
        words,
        text,
        map: alignWords(words.map((word) => normalizeWord(word.text)), written.map((word) => normalizeWord(word.text))),
      };
      alignmentRef.current = alignment;
    }
    const at = alignment.map[index];
    if (at === undefined || at < 0) return clear();

    const range = written[at].range();
    setSpokenWord(NAME, range);
    const block = range.startContainer.parentElement?.closest('.bn-block-content') ?? null;
    if (followRef.current && block && block !== shownRef.current.block) {
      block.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
    }
    shownRef.current = { index, block };
  });
}
