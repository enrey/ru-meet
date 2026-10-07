import { VirtualizedTranscriptView } from '@/components/VirtualizedTranscriptView';
import { PermissionWarning } from '@/components/PermissionWarning';
import { Button } from '@/components/ui/button';
import { ButtonGroup } from '@/components/ui/button-group';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { AudioLines, Cpu, Copy, FolderOpen, GlobeIcon } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { useTranscripts } from '@/contexts/TranscriptContext';
import { useConfig } from '@/contexts/ConfigContext';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { usePermissionCheck } from '@/hooks/usePermissionCheck';
import { ModalType } from '@/hooks/useModalState';
import { useIsLinux } from '@/hooks/usePlatform';
import { useMemo } from 'react';
import { useI18n } from '@/lib/i18n';

/**
 * TranscriptPanel Component
 *
 * Displays transcript content with controls for copying and language settings.
 * Uses TranscriptContext, ConfigContext, and RecordingStateContext internally.
 */

interface TranscriptPanelProps {
  // indicates stop-processing state for transcripts; derived from backend statuses.
  isProcessingStop: boolean;
  isStopping: boolean;
  showModal: (name: ModalType, message?: string) => void;
}

export function TranscriptPanel({
  isProcessingStop,
  isStopping,
  showModal
}: TranscriptPanelProps) {
  // Contexts
  const { transcripts, copyTranscript } = useTranscripts();
  const { transcriptModelConfig } = useConfig();
  const { isRecording, isPaused } = useRecordingState();
  const { checkPermissions, isChecking, hasSystemAudio, hasMicrophone } = usePermissionCheck();
  const isLinux = useIsLinux();
  const { t } = useI18n();

  const openRecordingFolder = async () => {
    try {
      await invoke('open_active_recording_folder');
    } catch (error) {
      toast.error(t('Failed to open recording folder'), { description: String(error) });
    }
  };

  // Convert transcripts to segments for virtualized view
  const segments = useMemo(() =>
    transcripts.map(transcript => ({
      id: transcript.id,
      timestamp: transcript.audio_start_time ?? 0,
      endTime: transcript.audio_end_time,
      text: transcript.text,
      confidence: transcript.confidence,
      speaker: transcript.speaker,
    })),
    [transcripts]
  );

  return (
    <div className="w-full min-h-0 border-r border-gray-200 bg-white flex flex-col overflow-hidden">
      {/* Title area - Sticky header */}
      <div className="shrink-0 z-10 bg-white p-4 border-gray-200">
        <div className="flex flex-col space-y-3">
          <div className="flex  flex-col space-y-2">
            <div className="flex justify-center  items-center space-x-2">
              <ButtonGroup>
                {isRecording && !isStopping && (
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={openRecordingFolder}
                    title={t('Open Recording Folder')}
                    aria-label={t('Open Folder')}
                  >
                    <FolderOpen />
                    <span className="hidden md:inline">{t('Open Folder')}</span>
                  </Button>
                )}
                {transcripts?.length > 0 && (
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={copyTranscript}
                    title={t('Copy Transcript')}
                  >
                    <Copy />
                    <span className='hidden md:inline'>
                      {t('Copy')}
                    </span>
                  </Button>
                )}
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => showModal('modelSelector')}
                  title={t('Transcription model')}
                >
                  <Cpu />
                  <span className="hidden md:inline">{t('Model')}</span>
                </Button>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => showModal('deviceSettings')}
                  title={t('Audio devices')}
                >
                  <AudioLines />
                  <span className="hidden md:inline">{t('Devices')}</span>
                </Button>
                {(transcriptModelConfig.provider === 'localWhisper' ||
                  transcriptModelConfig.provider === 'gigaam') && (
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => showModal('languageSettings')}
                    title={t('Language')}
                  >
                    <GlobeIcon />
                    <span className="hidden md:inline">{t('Language')}</span>
                  </Button>
                )}
                {transcriptModelConfig.provider === 'parakeet' && (
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <span className="inline-flex cursor-not-allowed">
                        <Button
                          variant="outline"
                          size="sm"
                          disabled
                          aria-label={t('Language selection is unavailable for Parakeet')}
                          className="rounded-l-none border-l-0"
                        >
                          <GlobeIcon />
                          <span className="hidden md:inline">{t('Language')}</span>
                        </Button>
                      </span>
                    </TooltipTrigger>
                    <TooltipContent>
                      {t('Parakeet detects the spoken language automatically and does not support manual language selection.')}
                    </TooltipContent>
                  </Tooltip>
                )}
              </ButtonGroup>
            </div>
          </div>
        </div>
      </div>

      {/* Permission Warning - Not needed on Linux */}
      {!isRecording && !isChecking && !isLinux && (
        <div className="flex justify-center px-4 pt-4">
          <PermissionWarning
            hasMicrophone={hasMicrophone}
            hasSystemAudio={hasSystemAudio}
            onRecheck={checkPermissions}
            isRechecking={isChecking}
          />
        </div>
      )}

      {/* Transcript content */}
      <div className="flex-1 min-h-0 pb-20">
        <div className="flex justify-center h-full">
          <div className="w-2/3 max-w-[750px] h-full">
            <VirtualizedTranscriptView
              segments={segments}
              isRecording={isRecording}
              isPaused={isPaused}
              isProcessing={isProcessingStop}
              isStopping={isStopping}
              enableStreaming={isRecording}
              showConfidence={true}
            />
          </div>
        </div>
      </div>
    </div>
  );
}
