'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface TranscriptMatch {
  id: string;
  /** Position in the paginated transcript, used to load the right page. */
  offset: number;
}

const SEARCH_DEBOUNCE_MS = 250;

/**
 * Search inside one meeting's transcript. Matches come from the backend (the
 * transcript is paginated, so the loaded window cannot be searched), and
 * stepping through them loads and scrolls to each line via `onJump`.
 */
export function useTranscriptSearch({
  meetingId,
  initialQuery = '',
  jumpToTranscript,
  onJump,
}: {
  meetingId: string;
  initialQuery?: string;
  jumpToTranscript?: (id: string, offset: number) => Promise<string | null>;
  onJump: (id: string) => void;
}) {
  const [query, setQuery] = useState(initialQuery);
  const [matches, setMatches] = useState<TranscriptMatch[]>([]);
  const [index, setIndex] = useState(0);
  const [searching, setSearching] = useState(false);
  const requestRef = useRef(0);

  const goTo = useCallback(async (match: TranscriptMatch | undefined) => {
    if (!match || !jumpToTranscript) return;
    const id = await jumpToTranscript(match.id, match.offset);
    if (id) onJump(id);
  }, [jumpToTranscript, onJump]);

  useEffect(() => {
    setQuery(initialQuery);
  }, [meetingId, initialQuery]);

  useEffect(() => {
    const request = ++requestRef.current;
    const trimmed = query.trim();
    if (!trimmed) {
      setMatches([]);
      setIndex(0);
      setSearching(false);
      return;
    }
    setSearching(true);
    const timer = window.setTimeout(() => {
      invoke<TranscriptMatch[]>('api_search_meeting_transcript', { meetingId, query: trimmed })
        .then((found) => {
          if (request !== requestRef.current) return;
          setMatches(found);
          setIndex(0);
          void goTo(found[0]);
        })
        .catch((error) => {
          console.error('Transcript search failed:', error);
          if (request === requestRef.current) setMatches([]);
        })
        .finally(() => {
          if (request === requestRef.current) setSearching(false);
        });
    }, SEARCH_DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
    // goTo changes with the loaded window; only a new query starts a search.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [meetingId, query]);

  const step = useCallback((delta: number) => {
    if (matches.length === 0) return;
    const next = (index + delta + matches.length) % matches.length;
    setIndex(next);
    void goTo(matches[next]);
  }, [matches, index, goTo]);

  return {
    query,
    setQuery,
    matchCount: matches.length,
    /** 1-based position of the current match, 0 when there is none. */
    position: matches.length ? index + 1 : 0,
    currentId: matches[index]?.id ?? null,
    searching,
    next: () => step(1),
    previous: () => step(-1),
  };
}
