"use client";
import { useState, useEffect, useRef, useCallback } from 'react';
import { motion } from 'framer-motion';
import { MeetingSummary, SummaryProcessResponse } from '@/types';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { translate } from '@/lib/i18n';
import { TranscriptPanel } from '@/components/MeetingDetails/TranscriptPanel';
import { SpeakerTimeline } from '@/components/MeetingDetails/SpeakerTimeline';
import { MeetingHeader, type MeetingDetailsTab, type MeetingSummaryState } from '@/components/MeetingDetails/MeetingHeader';
import { hasVisibleSummaryContent } from '@/lib/summary-content';
import { MeetingActionBar } from '@/components/MeetingDetails/MeetingActionBar';
import { MeetingSpeakersProvider } from '@/contexts/MeetingSpeakersContext';
import { TranscriptSearchBar } from '@/components/MeetingDetails/TranscriptSearchBar';
import { useTranscriptSearch } from '@/hooks/meeting-details/useTranscriptSearch';
import { SummaryPanel } from '@/components/MeetingDetails/SummaryPanel';
import { ModelConfig } from '@/components/ModelSettingsModal';

// Custom hooks
import { useMeetingData } from '@/hooks/meeting-details/useMeetingData';
import { useSummaryGeneration } from '@/hooks/meeting-details/useSummaryGeneration';
import { useTemplates } from '@/hooks/meeting-details/useTemplates';
import { useCopyOperations } from '@/hooks/meeting-details/useCopyOperations';
import { useMeetingOperations } from '@/hooks/meeting-details/useMeetingOperations';
import { useConfig } from '@/contexts/ConfigContext';

export default function PageContent({
  meeting,
  summaryData,
  initialSummary,
  shouldAutoGenerate = false,
  onAutoGenerateComplete,
  onMeetingUpdated,
  onRefetchTranscripts,
  // Pagination props for efficient transcript loading
  segments,
  hasMore,
  isLoadingMore,
  isLoadingPrevious,
  hasPrevious,
  totalCount,
  loadedCount,
  onLoadMore,
  onLoadPrevious,
  onJumpToSpeakerTime,
  onJumpToTranscript,
  initialSearchQuery = '',
  onSpeakerRenamed,
}: {
  meeting: any;
  summaryData: MeetingSummary | null;
  initialSummary: SummaryProcessResponse | null;
  shouldAutoGenerate?: boolean;
  onAutoGenerateComplete?: () => void;
  onMeetingUpdated?: () => Promise<void>;
  onRefetchTranscripts?: () => Promise<void>;
  // Pagination props
  segments?: any[];
  hasMore?: boolean;
  isLoadingMore?: boolean;
  isLoadingPrevious?: boolean;
  hasPrevious?: boolean;
  totalCount?: number;
  loadedCount?: number;
  onLoadMore?: () => void;
  onLoadPrevious?: () => void;
  onJumpToSpeakerTime?: (speaker: string, time: number) => Promise<string | null>;
  onJumpToTranscript?: (id: string, offset: number) => Promise<string | null>;
  /** Transcript search to open with, e.g. coming from the library search. */
  initialSearchQuery?: string;
  onSpeakerRenamed?: (oldName: string, newName: string) => void;
}) {
  console.log('📄 PAGE CONTENT: Initializing with data:', {
    meetingId: meeting.id,
    summaryDataKeys: summaryData ? Object.keys(summaryData) : null,
    transcriptsCount: meeting.transcripts?.length
  });

  // State
  const [customPrompt, setCustomPrompt] = useState<string>('');
  const isRecording = false;
  const [activeTab, setActiveTab] = useState<MeetingDetailsTab>('transcript');
  // Kept on the transcript when the meeting was opened from a transcript search.
  const openedFromSearch = initialSearchQuery.trim().length > 0;
  const [transcriptJump, setTranscriptJump] = useState<{ id: string; request: number } | null>(null);

  // Ref to store the modal open function from SummaryGeneratorButtonGroup
  const autoSwitchedSummaryMeetingIdsRef = useRef(new Set<string>());
  const manuallySelectedTabMeetingIdsRef = useRef(new Set<string>());
  const autoGenerationStartedMeetingIdRef = useRef<string | null>(null);

  // Sidebar context
  const { serverAddress } = useSidebar();

  // Get model config from ConfigContext
  const { modelConfig, setModelConfig, isModelConfigLoading } = useConfig();

  // Custom hooks
  const meetingData = useMeetingData({ meeting, summaryData, onMeetingUpdated });
  const templates = useTemplates();

  const searchInputRef = useRef<HTMLInputElement>(null);
  const showTranscriptLine = useCallback((id: string) => {
    setActiveTab('transcript');
    setTranscriptJump((current) => ({ id, request: (current?.request ?? 0) + 1 }));
  }, []);
  const transcriptSearch = useTranscriptSearch({
    meetingId: meeting.id,
    initialQuery: initialSearchQuery,
    jumpToTranscript: onJumpToTranscript,
    onJump: showTranscriptLine,
  });

  // Ctrl+F searches the transcript instead of the webview's own find.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'f') {
        event.preventDefault();
        manuallySelectedTabMeetingIdsRef.current.add(meeting.id);
        setActiveTab('transcript');
        // The transcript panel may only now become visible.
        window.setTimeout(() => {
          searchInputRef.current?.focus();
          searchInputRef.current?.select();
        }, 0);
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [meeting.id]);

  // Opened from the action bar menu and from summary errors that need a model.
  const [modelSettingsOpen, setModelSettingsOpen] = useState(false);
  const handleOpenModelSettings = () => setModelSettingsOpen(true);

  // Save model config to backend database and sync via event
  const handleSaveModelConfig = async (config?: ModelConfig) => {
    if (!config) return;
    try {
      await invoke('api_save_model_config', {
        provider: config.provider,
        model: config.model,
        whisperModel: config.whisperModel,
        apiKey: config.apiKey ?? null,
        ollamaEndpoint: config.ollamaEndpoint ?? null,
      });

      // Emit event so ConfigContext and other listeners stay in sync
      const { emit } = await import('@tauri-apps/api/event');
      await emit('model-config-updated', config);

      toast.success(translate('Model settings saved successfully'));
    } catch (error) {
      console.error('Failed to save model config:', error);
      toast.error(translate('Failed to save model settings'));
    }
  };

  const summaryGeneration = useSummaryGeneration({
    initialSummary,
    meeting,
    transcripts: meetingData.transcripts,
    modelConfig: modelConfig,
    isModelConfigLoading,
    selectedTemplate: templates.selectedTemplate,
    onMeetingUpdated,
    updateMeetingTitle: meetingData.updateMeetingTitle,
    setAiSummary: meetingData.setAiSummary,
    onOpenModelSettings: handleOpenModelSettings,
  });

  const copyOperations = useCopyOperations({
    meeting,
    transcripts: meetingData.transcripts,
    meetingTitle: meetingData.meetingTitle,
    aiSummary: meetingData.aiSummary,
    blockNoteSummaryRef: meetingData.blockNoteSummaryRef,
  });

  const meetingOperations = useMeetingOperations({
    meeting,
  });

  // Duration and speaker count come from the library listing.
  const { meetings: meetingList } = useSidebar();
  const listItem = meetingList.find((item) => item.id === meeting.id);
  const isSummaryGenerating = ['processing', 'summarizing', 'regenerating'].includes(summaryGeneration.summaryStatus);
  const summaryState: MeetingSummaryState = isSummaryGenerating ? 'processing'
    : hasVisibleSummaryContent(meetingData.aiSummary) ? 'ready'
    : summaryGeneration.summaryStatus === 'error' ? 'error'
    : 'none';

  const handleRenameMeeting = async (title: string) => {
    try {
      await invoke('api_save_meeting_title', { meetingId: meeting.id, title });
      meetingData.updateMeetingTitle(title);
      toast.success(translate('Meeting title updated successfully'));
    } catch (error) {
      toast.error(translate('Failed to update meeting title'), { description: String(error) });
      throw error;
    }
  };

  useEffect(() => {
    if (
      (meetingData.aiSummary || summaryGeneration.summaryStatus === 'completed')
      && !autoSwitchedSummaryMeetingIdsRef.current.has(meeting.id)
      && !manuallySelectedTabMeetingIdsRef.current.has(meeting.id)
      && !openedFromSearch
    ) {
      autoSwitchedSummaryMeetingIdsRef.current.add(meeting.id);
      setActiveTab('summary');
    }
  }, [meeting.id, meetingData.aiSummary, summaryGeneration.summaryStatus, openedFromSearch]);

  // Auto-generate only after the model configuration has settled.
  useEffect(() => {
    if (
      !shouldAutoGenerate
      || summaryGeneration.summaryStatus !== 'idle'
      || isModelConfigLoading
      || meetingData.transcripts.length === 0
      || autoGenerationStartedMeetingIdRef.current === meeting.id
    ) {
      return;
    }

    autoGenerationStartedMeetingIdRef.current = meeting.id;
    console.log(`🤖 Auto-generating summary with ${modelConfig.provider}/${modelConfig.model}...`);
    onAutoGenerateComplete?.();
    void summaryGeneration.handleGenerateSummary('');
  }, [
    shouldAutoGenerate,
    meeting.id,
    meetingData.transcripts.length,
    isModelConfigLoading,
    modelConfig.provider,
    modelConfig.model,
    summaryGeneration.handleGenerateSummary,
    summaryGeneration.summaryStatus,
    onAutoGenerateComplete,
  ]);

  return (
    <MeetingSpeakersProvider meetingId={meeting.id} onSpeakerRenamed={onSpeakerRenamed}>
    <motion.div
      initial={{ opacity: 0, y: 20 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.3, ease: 'easeOut' }}
      className="-ml-8 flex flex-col h-full min-w-0 bg-slate-50"
    >
      <MeetingHeader
        title={meetingData.meetingTitle}
        createdAt={meeting.created_at}
        durationSeconds={listItem?.durationSeconds}
        speakerCount={listItem?.speakerCount}
        summaryState={summaryState}
        onRename={handleRenameMeeting}
        activeTab={activeTab}
        onTabChange={(tab) => {
          manuallySelectedTabMeetingIdsRef.current.add(meeting.id);
          setActiveTab(tab);
        }}
        tabTools={activeTab === 'transcript' ? (
            <TranscriptSearchBar
              ref={searchInputRef}
              query={transcriptSearch.query}
              onQueryChange={transcriptSearch.setQuery}
              matchCount={transcriptSearch.matchCount}
              position={transcriptSearch.position}
              searching={transcriptSearch.searching}
              onNext={transcriptSearch.next}
              onPrevious={transcriptSearch.previous}
            />
        ) : undefined}
      >
        <MeetingActionBar
          meetingId={meeting.id}
          meetingFolderPath={meeting.folder_path}
          transcriptCount={totalCount ?? meetingData.transcripts.length}
          onCopyTranscript={copyOperations.handleCopyTranscript}
          onOpenMeetingFolder={meetingOperations.handleOpenMeetingFolder}
          onRefetchTranscripts={onRefetchTranscripts}
          summaryRef={meetingData.blockNoteSummaryRef}
          aiSummary={meetingData.aiSummary}
          summaryStatus={summaryGeneration.summaryStatus}
          isSummaryDirty={meetingData.isSummaryDirty}
          isSaving={meetingData.isSaving}
          onSaveAll={meetingData.saveAllChanges}
          onCopySummary={copyOperations.handleCopySummary}
          onGenerateSummary={summaryGeneration.handleGenerateSummary}
          onStopGeneration={summaryGeneration.handleStopGeneration}
          customPrompt={customPrompt}
          modelConfig={modelConfig}
          setModelConfig={setModelConfig}
          onSaveModelConfig={handleSaveModelConfig}
          isModelConfigLoading={isModelConfigLoading}
          modelSettingsOpen={modelSettingsOpen}
          onModelSettingsOpenChange={setModelSettingsOpen}
          availableTemplates={templates.availableTemplates}
          selectedTemplate={templates.selectedTemplate}
          onTemplateSelect={templates.handleTemplateSelection}
        />
      </MeetingHeader>
      {/* Both panels stay mounted so unsaved summary edits survive switching tabs. */}
      <div
        id="meeting-panel-transcript"
        role="tabpanel"
        aria-labelledby="meeting-tab-transcript"
        className={`${activeTab === 'transcript' ? 'flex' : 'hidden'} min-h-0 min-w-0 flex-1 flex-col bg-white`}
      >
        <SpeakerTimeline
          onSelectTurn={async (speaker, time) => {
            if (!onJumpToSpeakerTime) return;
            try {
              const id = await onJumpToSpeakerTime(speaker, time);
              if (id) {
                setActiveTab('transcript');
                setTranscriptJump((current) => ({ id, request: (current?.request ?? 0) + 1 }));
              } else {
                toast.info(translate('No transcript phrase was found at this time'));
              }
            } catch (error) {
              toast.error(translate('Could not jump to transcript'), { description: String(error) });
            }
          }}
        />
        <div className="min-h-0 flex-1">
          <TranscriptPanel
            transcripts={meetingData.transcripts}
            isRecording={isRecording}
            disableAutoScroll={true}
            usePagination={true}
            segments={segments}
            hasMore={hasMore}
            isLoadingMore={isLoadingMore}
            isLoadingPrevious={isLoadingPrevious}
            hasPrevious={hasPrevious}
            totalCount={totalCount}
            loadedCount={loadedCount}
            onLoadMore={onLoadMore}
            onLoadPrevious={onLoadPrevious}
            scrollTarget={transcriptJump}
            meetingId={meeting.id}
            highlightQuery={transcriptSearch.query}
          />
        </div>
      </div>
      <div
        id="meeting-panel-summary"
        role="tabpanel"
        aria-labelledby="meeting-tab-summary"
        className={`${activeTab === 'summary' ? 'flex' : 'hidden'} min-h-0 min-w-0 flex-1 flex-col bg-white`}
      >
        <SummaryPanel
          meeting={meeting}
          meetingTitle={meetingData.meetingTitle}
          summaryRef={meetingData.blockNoteSummaryRef}
          aiSummary={meetingData.aiSummary}
          summaryStatus={summaryGeneration.summaryStatus}
          modelConfig={modelConfig}
          onGenerateSummary={summaryGeneration.handleGenerateSummary}
          customPrompt={customPrompt}
          onPromptChange={setCustomPrompt}
          onSaveSummary={meetingData.handleSaveSummary}
          onSummaryChange={meetingData.handleSummaryChange}
          onDirtyChange={meetingData.setIsSummaryDirty}
          summaryError={summaryGeneration.summaryError}
          onRegenerateSummary={summaryGeneration.handleRegenerateSummary}
          getSummaryStatusMessage={summaryGeneration.getSummaryStatusMessage}
        />
      </div>
    </motion.div>
    </MeetingSpeakersProvider>
  );
}
