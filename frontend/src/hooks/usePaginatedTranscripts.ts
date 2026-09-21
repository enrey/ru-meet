import { useState, useCallback, useRef, useEffect, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Transcript, MeetingMetadata, PaginatedTranscriptsResponse, TranscriptSegmentData } from "@/types";

const DEFAULT_PAGE_SIZE = 100;

interface UsePaginatedTranscriptsProps {
    meetingId: string | null;
    /** Optional initial timestamp (in seconds) from URL for loading the correct page */
    initialTimestamp?: number;
}

interface UsePaginatedTranscriptsReturn {
    metadata: MeetingMetadata | null;
    segments: TranscriptSegmentData[];
    transcripts: Transcript[];
    isLoading: boolean;
    isLoadingMore: boolean;
    isLoadingPrevious: boolean;
    hasPrevious: boolean;
    hasMore: boolean;
    totalCount: number;
    loadedCount: number;
    error: string | null;

    // Actions
    loadMore: () => Promise<void>;
    loadPrevious: () => Promise<void>;
    jumpToSpeakerTime: (speaker: string, time: number) => Promise<string | null>;
    renameSpeakerLocally: (oldName: string, newName: string) => void;
    reset: () => void;
    refetch: () => Promise<void>;
}

/**
 * Convert Transcript array to TranscriptSegmentData for virtualized display
 */
function convertTranscriptsToSegments(transcripts: Transcript[]): TranscriptSegmentData[] {
    return transcripts.map(t => ({
        id: t.id,
        timestamp: t.audio_start_time ?? 0,
        endTime: t.audio_end_time,
        text: t.text,
        confidence: t.confidence,
        speaker: t.speaker,
    }));
}

export function usePaginatedTranscripts({
    meetingId,
    initialTimestamp,
}: UsePaginatedTranscriptsProps): UsePaginatedTranscriptsReturn {
    const [metadata, setMetadata] = useState<MeetingMetadata | null>(null);
    const [transcripts, setTranscripts] = useState<Transcript[]>([]);
    const [totalCount, setTotalCount] = useState(0);
    const [isLoading, setIsLoading] = useState(true);
    const [isLoadingMore, setIsLoadingMore] = useState(false);
    const [isLoadingPrevious, setIsLoadingPrevious] = useState(false);
    const [baseOffset, setBaseOffset] = useState(0);
    const [hasMore, setHasMore] = useState(false);
    const [error, setError] = useState<string | null>(null);

    const offsetRef = useRef(0);
    const activeMeetingIdRef = useRef<string | null>(null);
    const requestIdRef = useRef(0);
    const jumpRequestRef = useRef(0);
    const isLoadingRef = useRef(false);
    const lastLoadTimeRef = useRef(0); // Debounce protection

    const isCurrentRequest = useCallback((requestId: number) =>
        requestIdRef.current === requestId && activeMeetingIdRef.current === meetingId,
        [meetingId]
    );

    // Reset invalidates pending reads, including same-meeting refetches.
    const reset = useCallback(() => {
        requestIdRef.current += 1;
        jumpRequestRef.current += 1;
        isLoadingRef.current = false;
        lastLoadTimeRef.current = 0;
        setMetadata(null);
        setTranscripts([]);
        setTotalCount(0);
        setIsLoading(true);
        setIsLoadingMore(false);
        setIsLoadingPrevious(false);
        setBaseOffset(0);
        setHasMore(false);
        setError(null);
        offsetRef.current = 0;
    }, []);

    // Load meeting metadata
    const loadMetadata = useCallback(async (requestId: number): Promise<MeetingMetadata | null> => {
        if (!meetingId || !isCurrentRequest(requestId)) return null;

        try {
            const data = await invoke<MeetingMetadata>('api_get_meeting_metadata', {
                meetingId,
            });
            if (!isCurrentRequest(requestId)) return null;
            setMetadata(data);
            return data;
        } catch (err) {
            if (!isCurrentRequest(requestId)) return null;
            console.error('Failed to load meeting metadata:', err);
            setError('Failed to load meeting details');
            return null;
        }
    }, [meetingId, isCurrentRequest]);

    // Load transcripts at specific offset
    const loadTranscriptsAtOffset = useCallback(async (
        requestId: number,
        offset: number,
        append: boolean = true
    ): Promise<Transcript[]> => {
        if (!meetingId || !isCurrentRequest(requestId)) return [];

        try {
            const response = await invoke<PaginatedTranscriptsResponse>(
                'api_get_meeting_transcripts',
                {
                    meetingId,
                    limit: DEFAULT_PAGE_SIZE,
                    offset,
                }
            );

            if (!isCurrentRequest(requestId)) return [];
            const newTranscripts = response.transcripts;

            if (append) {
                setTranscripts(prev => {
                    if (!isCurrentRequest(requestId)) return prev;
                    // Deduplicate by id
                    const existingIds = new Set(prev.map(t => t.id));
                    const uniqueNew = newTranscripts.filter(t => !existingIds.has(t.id));
                    // Sort by audio_start_time
                    return [...prev, ...uniqueNew].sort((a, b) =>
                        (a.audio_start_time ?? 0) - (b.audio_start_time ?? 0)
                    );
                });
            } else {
                setTranscripts(newTranscripts);
            }

            setHasMore(response.has_more);
            setTotalCount(response.total_count);
            offsetRef.current = offset + newTranscripts.length;

            return newTranscripts;
        } catch (err) {
            if (!isCurrentRequest(requestId)) return [];
            console.error('Failed to load transcripts:', err);
            setError('Failed to load transcripts');
            return [];
        }
    }, [meetingId, isCurrentRequest]);

    // Load next page with debounce protection
    const loadMore = useCallback(async () => {
        const requestId = requestIdRef.current;
        if (!isCurrentRequest(requestId)) return;
        const now = Date.now();
        // Debounce: require at least 100ms between calls
        if (now - lastLoadTimeRef.current < 100) {
            return;
        }

        if (isLoadingRef.current || !hasMore || !meetingId || isLoading) return;

        lastLoadTimeRef.current = now;
        isLoadingRef.current = true;
        setIsLoadingMore(true);
        try {
            await loadTranscriptsAtOffset(requestId, offsetRef.current, true);
        } finally {
            if (isCurrentRequest(requestId)) {
                setIsLoadingMore(false);
                isLoadingRef.current = false;
            }
        }
    }, [hasMore, meetingId, loadTranscriptsAtOffset, isLoading, isCurrentRequest]);

    const loadPrevious = useCallback(async () => {
        const requestId = requestIdRef.current;
        if (!meetingId || !isCurrentRequest(requestId) || baseOffset === 0 || isLoadingPrevious) return;
        const offset = Math.max(0, baseOffset - DEFAULT_PAGE_SIZE);
        setIsLoadingPrevious(true);
        try {
            const response = await invoke<PaginatedTranscriptsResponse>('api_get_meeting_transcripts', {
                meetingId, limit: baseOffset - offset, offset,
            });
            if (!isCurrentRequest(requestId)) return;
            setTranscripts((current) => {
                const ids = new Set(current.map((item) => item.id));
                return [...response.transcripts.filter((item) => !ids.has(item.id)), ...current];
            });
            setBaseOffset(offset);
        } catch (error) {
            console.error('Failed to load earlier transcripts:', error);
        } finally {
            if (isCurrentRequest(requestId)) setIsLoadingPrevious(false);
        }
    }, [meetingId, baseOffset, isLoadingPrevious, isCurrentRequest]);

    const jumpToSpeakerTime = useCallback(async (speaker: string, time: number): Promise<string | null> => {
        if (!meetingId || !Number.isFinite(time) || activeMeetingIdRef.current !== meetingId) return null;
        const jumpRequest = ++jumpRequestRef.current;
        const match = await invoke<{ id: string; offset: number } | null>('api_find_transcript_at_time', {
            meetingId, speaker, time,
        });
        if (!match || jumpRequest !== jumpRequestRef.current || activeMeetingIdRef.current !== meetingId) return null;
        if (transcripts.some((item) => item.id === match.id)) return match.id;
        // A jump replaces the loaded window, so invalidate any in-flight
        // infinite-scroll request before it can append an unrelated page.
        const requestId = ++requestIdRef.current;
        isLoadingRef.current = false;
        setIsLoadingMore(false);
        const offset = Math.floor(match.offset / DEFAULT_PAGE_SIZE) * DEFAULT_PAGE_SIZE;
        const response = await invoke<PaginatedTranscriptsResponse>('api_get_meeting_transcripts', {
            meetingId, limit: DEFAULT_PAGE_SIZE, offset,
        });
        if (!isCurrentRequest(requestId) || jumpRequest !== jumpRequestRef.current) return null;
        if (!response.transcripts.some((item) => item.id === match.id)) return null;
        setTranscripts(response.transcripts);
        setHasMore(response.has_more);
        setTotalCount(response.total_count);
        offsetRef.current = offset + response.transcripts.length;
        setBaseOffset(offset);
        return match.id;
    }, [meetingId, transcripts, isCurrentRequest]);

    const renameSpeakerLocally = useCallback((oldName: string, newName: string) => {
        setTranscripts((current) => current.map((item) =>
            item.speaker === oldName ? { ...item, speaker: newName } : item
        ));
    }, []);

    // Force refetch of data (e.g., after retranscription)
    const refetch = useCallback(async () => {
        if (!meetingId || activeMeetingIdRef.current !== meetingId) return;

        reset();
        const requestId = requestIdRef.current;
        try {
            await loadMetadata(requestId);
            await loadTranscriptsAtOffset(requestId, 0, false);
        } finally {
            if (isCurrentRequest(requestId)) setIsLoading(false);
        }
    }, [meetingId, reset, loadMetadata, loadTranscriptsAtOffset, isCurrentRequest]);

    // A new meeting or effect lifetime owns its own requests.
    useEffect(() => {
        activeMeetingIdRef.current = meetingId;
        if (meetingId) {
            void refetch();
        } else {
            reset();
        }

        return () => {
            requestIdRef.current += 1;
            activeMeetingIdRef.current = null;
        };
    }, [meetingId, reset, refetch]);

    // Speaker labels can be repaired in the backend (a meeting whose diarization
    // turns never reached its transcript rows); reload so the transcript shows
    // the same speakers as the timeline.
    useEffect(() => {
        if (!meetingId) return;
        let cancelled = false;
        let unlisten: UnlistenFn | undefined;
        void listen<{ meetingId?: string }>('transcript-speakers-updated', ({ payload }) => {
            if (!payload.meetingId || payload.meetingId === meetingId) void refetch();
        }).then((fn) => {
            if (cancelled) fn();
            else unlisten = fn;
        });
        return () => {
            cancelled = true;
            unlisten?.();
        };
    }, [meetingId, refetch]);

    // Convert to segments (memoized)
    const segments = useMemo(() =>
        convertTranscriptsToSegments(transcripts),
        [transcripts]
    );

    return {
        metadata,
        segments,
        transcripts,
        isLoading,
        isLoadingMore,
        isLoadingPrevious,
        hasPrevious: baseOffset > 0,
        hasMore,
        totalCount,
        loadedCount: transcripts.length,
        error,
        loadMore,
        loadPrevious,
        jumpToSpeakerTime,
        renameSpeakerLocally,
        reset,
        refetch,
    };
}
