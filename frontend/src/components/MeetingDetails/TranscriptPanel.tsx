"use client";

import { Transcript, TranscriptSegmentData } from '@/types';
import { VirtualizedTranscriptView } from '@/components/VirtualizedTranscriptView';
import { useMemo } from 'react';

interface TranscriptPanelProps {
  transcripts: Transcript[];
  isRecording: boolean;
  disableAutoScroll?: boolean;

  // Optional pagination props (when using virtualization)
  usePagination?: boolean;
  segments?: TranscriptSegmentData[];
  hasMore?: boolean;
  isLoadingMore?: boolean;
  isLoadingPrevious?: boolean;
  hasPrevious?: boolean;
  totalCount?: number;
  loadedCount?: number;
  onLoadMore?: () => void;
  onLoadPrevious?: () => void;
  scrollTarget?: { id: string; request: number } | null;

  meetingId?: string;
  /** Transcript search query to highlight. */
  highlightQuery?: string;
}

export function TranscriptPanel({
  transcripts,
  isRecording,
  disableAutoScroll = false,
  usePagination = false,
  segments,
  hasMore,
  isLoadingMore,
  isLoadingPrevious,
  hasPrevious,
  totalCount,
  loadedCount,
  onLoadMore,
  onLoadPrevious,
  scrollTarget,
  meetingId,
  highlightQuery,
}: TranscriptPanelProps) {
  // Convert transcripts to segments if pagination is not used but we want virtualization
  const convertedSegments = useMemo(() => {
    if (usePagination && segments) {
      return segments;
    }
    // Convert transcripts to segments for virtualization
    return transcripts.map(t => ({
      id: t.id,
      timestamp: t.audio_start_time ?? 0,
      endTime: t.audio_end_time,
      text: t.text,
      confidence: t.confidence,
      speaker: t.speaker,
    }));
  }, [transcripts, usePagination, segments]);

  return (
    <div className="flex h-full min-w-0 w-full bg-white flex-col relative @container">
      {/* Transcript content - use virtualized view for better performance */}
      <div className="flex-1 overflow-hidden px-4 pt-2 pb-4">
        <VirtualizedTranscriptView
          segments={convertedSegments}
          meetingId={meetingId}
          isRecording={isRecording}
          isPaused={false}
          isProcessing={false}
          isStopping={false}
          enableStreaming={false}
          showConfidence={true}
          disableAutoScroll={disableAutoScroll}
          hasMore={hasMore}
          isLoadingMore={isLoadingMore}
          isLoadingPrevious={isLoadingPrevious}
          hasPrevious={hasPrevious}
          totalCount={totalCount}
          loadedCount={loadedCount}
          onLoadMore={onLoadMore}
          onLoadPrevious={onLoadPrevious}
          scrollTarget={scrollTarget}
          highlightQuery={highlightQuery}
        />
      </div>
    </div>
  );
}
