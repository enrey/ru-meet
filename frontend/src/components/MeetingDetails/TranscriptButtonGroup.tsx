"use client";

import { useState, useCallback } from 'react';
import { Button } from '@/components/ui/button';
import { ButtonGroup } from '@/components/ui/button-group';
import { Copy, FolderOpen, RefreshCw, Square, UsersRound } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { RetranscribeDialog } from './RetranscribeDialog';
import { DiarizationProgress } from './DiarizationProgress';
import { useConfig } from '@/contexts/ConfigContext';


interface TranscriptButtonGroupProps {
  transcriptCount: number;
  onCopyTranscript: () => void;
  onOpenMeetingFolder: () => Promise<void>;
  meetingId?: string;
  meetingFolderPath?: string | null;
  onRefetchTranscripts?: () => Promise<void>;
}


export function TranscriptButtonGroup({
  transcriptCount,
  onCopyTranscript,
  onOpenMeetingFolder,
  meetingId,
  meetingFolderPath,
  onRefetchTranscripts,
}: TranscriptButtonGroupProps) {
  const { betaFeatures } = useConfig();
  const [showRetranscribeDialog, setShowRetranscribeDialog] = useState(false);
  const [isStartingDiarization, setIsStartingDiarization] = useState(false);
  const [isDiarizing, setIsDiarizing] = useState(false);
  const [isCancellingDiarization, setIsCancellingDiarization] = useState(false);

  const handleRetranscribeComplete = useCallback(async () => {
    // Refetch transcripts to show the updated data
    if (onRefetchTranscripts) {
      await onRefetchTranscripts();
    }
  }, [onRefetchTranscripts]);

  const handleDiarize = useCallback(async () => {
    if (!meetingId || !meetingFolderPath) return;
    setIsStartingDiarization(true);
    try {
      await invoke('rerun_diarization', { meetingId, meetingFolderPath });
      toast.info('Speaker diarization started');
    } catch (error) {
      toast.error('Could not start speaker diarization', { description: String(error) });
    } finally {
      setIsStartingDiarization(false);
    }
  }, [meetingId, meetingFolderPath]);

  const handleStopDiarization = useCallback(async () => {
    if (!meetingId) return;
    setIsCancellingDiarization(true);
    try {
      await invoke('cancel_diarization', { meetingId });
    } catch (error) {
      toast.error('Could not stop speaker diarization', { description: String(error) });
      setIsCancellingDiarization(false);
    }
  }, [meetingId]);

  return (
    <div className="flex items-center justify-center w-full gap-2">
      <ButtonGroup>
        <Button
          variant="outline"
          size="sm"
          className="px-2 @[22rem]:px-3"
          onClick={onCopyTranscript}
          disabled={transcriptCount === 0}
          title={transcriptCount === 0 ? 'No transcript available' : 'Copy Transcript'}
        >
          <Copy />
          <span className="hidden @[22rem]:inline">Copy</span>
        </Button>

        <Button
          size="sm"
          variant="outline"
          className="px-2 @[22rem]:px-4"
          onClick={onOpenMeetingFolder}
          title="Open Recording Folder"
        >
          <FolderOpen className="@[22rem]:mr-2" size={18} />
          <span className="hidden @[22rem]:inline">Recording</span>
        </Button>

        {meetingId && meetingFolderPath && (
          <Button
            size="sm"
            variant={isDiarizing ? 'destructive' : 'outline'}
            className="px-2 @[22rem]:px-4"
            onClick={isDiarizing ? handleStopDiarization : handleDiarize}
            disabled={isStartingDiarization || isCancellingDiarization}
            title={isDiarizing ? 'Stop speaker diarization' : 'Identify speakers in this recording'}
          >
            {isDiarizing ? <Square className="@[22rem]:mr-2" size={18} /> : <UsersRound className="@[22rem]:mr-2" size={18} />}
            <span className="hidden @[22rem]:inline">{isDiarizing ? (isCancellingDiarization ? 'Stopping…' : 'Stop') : 'Diarize'}</span>
          </Button>
        )}

        {betaFeatures.importAndRetranscribe && meetingId && meetingFolderPath && (
          <Button
            size="sm"
            variant="outline"
            className="bg-gradient-to-r from-blue-50 to-purple-50 hover:from-blue-100 hover:to-purple-100 border-blue-200 px-2 @[22rem]:px-4"
            onClick={() => setShowRetranscribeDialog(true)}
            title="Retranscribe to enhance your recorded audio"
          >
            <RefreshCw className="@[22rem]:mr-2" size={18} />
            <span className="hidden @[22rem]:inline">Enhance</span>
          </Button>
        )}
      </ButtonGroup>

      <DiarizationProgress
        meetingId={meetingId}
        onLabelsSaved={onRefetchTranscripts}
        onStatusChange={(inProgress) => {
          setIsDiarizing(inProgress);
          if (!inProgress) setIsCancellingDiarization(false);
        }}
      />

      {betaFeatures.importAndRetranscribe && meetingId && meetingFolderPath && (
        <RetranscribeDialog
          open={showRetranscribeDialog}
          onOpenChange={setShowRetranscribeDialog}
          meetingId={meetingId}
          meetingFolderPath={meetingFolderPath}
          onComplete={handleRetranscribeComplete}
        />
      )}
    </div>
  );
}
