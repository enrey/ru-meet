'use client';

import React, { createContext, useContext, useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { toast } from 'sonner';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { TranscriptRecovery } from '@/components/TranscriptRecovery';
import { RecordingStatus, useRecordingState } from '@/contexts/RecordingStateContext';
import { useTranscriptRecovery } from '@/hooks/useTranscriptRecovery';
import { indexedDBService } from '@/services/indexedDBService';
import { useI18n } from '@/lib/i18n';

interface TranscriptRecoveryContextValue {
  /** Interrupted meetings that can still be recovered from IndexedDB. */
  recoverableCount: number;
  openRecoveryDialog: () => void;
}

const TranscriptRecoveryContext = createContext<TranscriptRecoveryContextValue | null>(null);

export function useTranscriptRecoveryDialog(): TranscriptRecoveryContextValue {
  const context = useContext(TranscriptRecoveryContext);
  if (!context) {
    throw new Error('useTranscriptRecoveryDialog must be used within a TranscriptRecoveryProvider');
  }
  return context;
}

/**
 * Owns the interrupted-meeting recovery dialog for the whole app, so it can be
 * opened from the navigation on any page. It still opens by itself once per
 * session when interrupted meetings are found.
 */
export function TranscriptRecoveryProvider({ children }: { children: React.ReactNode }) {
  const [showRecoveryDialog, setShowRecoveryDialog] = useState(false);
  const { refetchMeetings } = useSidebar();
  const recordingState = useRecordingState();
  const { status } = recordingState;
  const router = useRouter();
  const { t } = useI18n();
  const {
    recoverableMeetings,
    checkForRecoverableTranscripts,
    recoverMeeting,
    loadMeetingTranscripts,
    deleteRecoverableMeeting,
  } = useTranscriptRecovery();

  // Startup recovery check
  useEffect(() => {
    const performStartupChecks = async () => {
      try {
        // Skip recovery check if currently recording or processing stop
        // This prevents the recovery dialog from showing when:
        if (recordingState.isRecording ||
          status === RecordingStatus.STOPPING ||
          status === RecordingStatus.PROCESSING_TRANSCRIPTS ||
          status === RecordingStatus.SAVING) {
          console.log('Skipping recovery check - recording in progress or processing');
          return;
        }

        // 1. Clean up old meetings (7+ days)
        try {
          await indexedDBService.deleteOldMeetings(7);
        } catch (error) {
          console.warn('⚠️ Failed to clean up old meetings:', error);
        }

        // 2. Clean up saved meetings (24+ hours after save)
        try {
          await indexedDBService.deleteSavedMeetings(24);
        } catch (error) {
          console.warn('⚠️ Failed to clean up saved meetings:', error);
        }

        // 3. Always check for recoverable meetings on startup
        // Don't skip based on sessionStorage - we need to check every time
        await checkForRecoverableTranscripts();
      } catch (error) {
        console.error('Failed to perform startup checks:', error);
      }
    };

    performStartupChecks();
  }, [checkForRecoverableTranscripts, recordingState.isRecording, status]);

  // Watch for recoverable meetings changes and show dialog once per session
  useEffect(() => {
    // Only show dialog if we have meetings and haven't shown it yet this session
    if (recoverableMeetings.length > 0) {
      const shownThisSession = sessionStorage.getItem('recovery_dialog_shown');
      if (!shownThisSession) {
        setShowRecoveryDialog(true);
        sessionStorage.setItem('recovery_dialog_shown', 'true');
      }
    }
  }, [recoverableMeetings]);

  // Handle recovery with toast notifications and navigation
  const handleRecovery = async (meetingId: string) => {
    try {
      const result = await recoverMeeting(meetingId);

      if (result.success) {
        toast.success(t('Meeting recovered successfully!'), {
          description: result.audioRecoveryStatus?.status === 'success'
            ? t('Transcripts and audio recovered')
            : t('Transcripts recovered (no audio available)'),
          action: result.meetingId ? {
            label: t('View Meeting'),
            onClick: () => {
              router.push(`/meeting-details?id=${result.meetingId}`);
            }
          } : undefined,
          duration: 10000,
        });

        // Refresh the meetings list to show the newly recovered meeting
        await refetchMeetings();

        // If no more recoverable meetings, clear session flag so dialog can show again
        if (recoverableMeetings.length === 0) {
          sessionStorage.removeItem('recovery_dialog_shown');
        }

        // Auto-navigate after a short delay
        if (result.meetingId) {
          setTimeout(() => {
            router.push(`/meeting-details?id=${result.meetingId}`);
          }, 2000);
        }
      }
    } catch (error) {
      toast.error(t('Failed to recover meeting'), {
        description: error instanceof Error ? error.message : t('Unknown error occurred'),
      });
      throw error;
    }
  };

  // Handle dialog close - clear session flag if no meetings left
  const handleDialogClose = () => {
    setShowRecoveryDialog(false);
    // If user closes dialog and there are no more meetings, clear the flag
    // This allows the dialog to show again next session if new meetings appear
    if (recoverableMeetings.length === 0) {
      sessionStorage.removeItem('recovery_dialog_shown');
    }
  };

  return (
    <TranscriptRecoveryContext.Provider
      value={{
        recoverableCount: recoverableMeetings.length,
        openRecoveryDialog: () => setShowRecoveryDialog(true),
      }}
    >
      {children}
      <TranscriptRecovery
        isOpen={showRecoveryDialog}
        onClose={handleDialogClose}
        recoverableMeetings={recoverableMeetings}
        onRecover={handleRecovery}
        onDelete={deleteRecoverableMeeting}
        onLoadPreview={loadMeetingTranscripts}
      />
    </TranscriptRecoveryContext.Provider>
  );
}
