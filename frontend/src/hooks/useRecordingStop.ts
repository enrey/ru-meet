import { useCallback } from 'react';
import { useRouter } from 'next/navigation';
import { toast } from 'sonner';
import { useTranscripts } from '@/contexts/TranscriptContext';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { useRecordingState, RecordingStatus } from '@/contexts/RecordingStateContext';
import { storageService } from '@/services/storageService';
import type { FinalizedRecording } from '@/services/recordingService';
import {
  applyPinnedSummaryLanguageToMeeting,
  detectAndCacheSummaryLanguage,
} from '@/lib/summary-language-preferences';

type SummaryStatus = 'idle' | 'processing' | 'summarizing' | 'regenerating' | 'completed' | 'error';

interface UseRecordingStopReturn {
  handleRecordingStop: (result: FinalizedRecording | null) => Promise<void>;
  isStopping: boolean;
  isProcessingTranscript: boolean;
  isSavingTranscript: boolean;
  summaryStatus: SummaryStatus;
  setIsStopping: (value: boolean) => void;
}

const handledMeetingIds = new Set<string>();

export function useRecordingStop(
  setIsRecording: (value: boolean) => void,
  setIsRecordingDisabled: (value: boolean) => void
): UseRecordingStopReturn {
  const recordingState = useRecordingState();
  const { status, setStatus, isStopping, isProcessing, isSaving } = recordingState;
  const { transcriptsRef, markMeetingAsSaved } = useTranscripts();
  const { refetchMeetings, setCurrentMeeting, setIsMeetingActive } = useSidebar();
  const router = useRouter();

  const handleRecordingStop = useCallback(async (result: FinalizedRecording | null) => {
    setIsRecording(false);
    setIsRecordingDisabled(true);
    setIsMeetingActive(false);

    if (!result || handledMeetingIds.has(result.meetingId)) {
      setStatus(RecordingStatus.IDLE);
      setIsRecordingDisabled(false);
      return;
    }
    handledMeetingIds.add(result.meetingId);

    try {
      setStatus(RecordingStatus.SAVING, 'Recording finalized');
      let shouldDetectSummaryLanguage = false;
      try {
        shouldDetectSummaryLanguage = !(await applyPinnedSummaryLanguageToMeeting(result.meetingId));
      } catch (error) {
        console.warn('Failed to apply pinned summary language:', error);
      }
      if (shouldDetectSummaryLanguage) {
        try {
          await detectAndCacheSummaryLanguage(
            result.meetingId,
            transcriptsRef.current.map(transcript => transcript.text)
          );
        } catch (error) {
          console.warn('Failed to detect summary language:', error);
        }
      }

      await markMeetingAsSaved();
      await refetchMeetings();
      try {
        const meeting = await storageService.getMeeting(result.meetingId);
        setCurrentMeeting({ id: result.meetingId, title: meeting?.title ?? result.meetingName });
      } catch {
        setCurrentMeeting({ id: result.meetingId, title: result.meetingName });
      }

      setStatus(RecordingStatus.COMPLETED);
      toast.success('Recording saved successfully!', {
        description: `${result.transcriptCount} transcript segments saved.`,
        action: {
          label: 'View Meeting',
          onClick: () => router.push(`/meeting-details?id=${result.meetingId}`),
        },
        duration: 10000,
      });
      setStatus(RecordingStatus.IDLE);
    } catch (error) {
      handledMeetingIds.delete(result.meetingId);
      const message = error instanceof Error ? error.message : String(error);
      setStatus(RecordingStatus.ERROR, message);
      toast.error('Recording was saved, but the UI could not refresh', { description: message });
    } finally {
      setIsRecordingDisabled(false);
    }
  }, [markMeetingAsSaved, refetchMeetings, router, setCurrentMeeting, setIsMeetingActive,
    setIsRecording, setIsRecordingDisabled, setStatus, transcriptsRef]);

  return {
    handleRecordingStop,
    isStopping,
    isProcessingTranscript: isProcessing,
    isSavingTranscript: isSaving,
    summaryStatus: status === RecordingStatus.PROCESSING_TRANSCRIPTS ? 'processing' : 'idle',
    setIsStopping: (value: boolean) => setStatus(value ? RecordingStatus.STOPPING : RecordingStatus.IDLE),
  };
}
