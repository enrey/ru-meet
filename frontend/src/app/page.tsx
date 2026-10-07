'use client';

import { useState } from 'react';
import { DraggableAudioStatus } from '@/components/DraggableAudioStatus';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { useRecordingState, RecordingStatus } from '@/contexts/RecordingStateContext';
import { useConfig } from '@/contexts/ConfigContext';
import { StatusOverlays } from '@/app/_components/StatusOverlays';
import { SettingsModals } from './_components/SettingsModal';
import { TranscriptPanel } from './_components/TranscriptPanel';
import { useModalState } from '@/hooks/useModalState';
import { useRecordingStateSync } from '@/hooks/useRecordingStateSync';
import { useRecordingStart } from '@/hooks/useRecordingStart';

export default function Home() {
  // Local page state (not moved to contexts)
  const [isRecording, setIsRecordingState] = useState(false);

  // Use contexts for state management
  const { transcriptModelConfig, selectedDevices } = useConfig();
  const recordingState = useRecordingState();

  // Extract status from global state
  const { status, isStopping, isProcessing, isSaving } = recordingState;

  // Hooks
  const { setIsMeetingActive, isCollapsed: sidebarCollapsed } = useSidebar();
  const { modals, messages, showModal, hideModal } = useModalState(transcriptModelConfig);
  useRecordingStateSync(isRecording, setIsRecordingState, setIsMeetingActive);
  useRecordingStart(isRecording, setIsRecordingState, showModal);

  // Computed values using global status
  const isProcessingStop = status === RecordingStatus.PROCESSING_TRANSCRIPTS || isProcessing;

  return (
    <div className="flex flex-col h-full bg-gray-50">
      {/* All Modals supported*/}
      <SettingsModals
        modals={modals}
        messages={messages}
        onClose={hideModal}
      />

      <div className="relative flex flex-1 min-h-0 overflow-hidden">
        <TranscriptPanel
          isProcessingStop={isProcessingStop}
          isStopping={isStopping}
          showModal={showModal}
        />

        <DraggableAudioStatus
          devices={selectedDevices}
          recording={recordingState.isRecording && !isStopping && !isProcessingStop && !isSaving}
          disabled={recordingState.isStartingRecording || isStopping || isProcessingStop || isSaving}
        />

        {/* Status Overlays - Processing and Saving */}
        <StatusOverlays
          isProcessing={status === RecordingStatus.PROCESSING_TRANSCRIPTS && !recordingState.isRecording}
          isSaving={status === RecordingStatus.SAVING}
          sidebarCollapsed={sidebarCollapsed}
        />
      </div>
    </div>
  );
}
