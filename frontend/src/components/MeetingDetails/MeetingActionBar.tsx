"use client";

import { useCallback, useEffect, useState, type RefObject } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import {
  Check,
  Copy,
  FileText,
  FolderOpen,
  Languages,
  Loader2,
  MoreHorizontal,
  Pause,
  Play,
  RefreshCw,
  Save,
  Settings,
  Sparkles,
  Square,
  Trash2,
  UsersRound,
  Volume2,
} from 'lucide-react';
import { useRouter } from 'next/navigation';
import { Button } from '@/components/ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { Dialog, DialogContent, DialogTitle } from '@/components/ui/dialog';
import { Popover, PopoverAnchor, PopoverContent } from '@/components/ui/popover';
import { VisuallyHidden } from '@/components/ui/visually-hidden';
import { ModelConfig, ModelSettingsModal } from '@/components/ModelSettingsModal';
import { LanguagePickerPopover } from '@/components/LanguagePickerPopover';
import { useConfig } from '@/contexts/ConfigContext';
import { useSummaryLanguage } from '@/hooks/meeting-details/useSummaryLanguage';
import { useSummarySpeech } from '@/hooks/useSummarySpeech';
import { BlockNoteSummaryViewRef } from '@/components/AISummary/BlockNoteSummaryView';
import { hasVisibleSummaryContent } from '@/lib/summary-content';
import { MeetingSummary } from '@/types';
import { useI18n } from '@/lib/i18n';
import { ConfirmationModal } from '@/components/ConfirmationModel/confirmation-modal';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { DiarizationProgress } from './DiarizationProgress';
import { RetranscribeDialog } from './RetranscribeDialog';

type SummaryStatus = 'idle' | 'processing' | 'summarizing' | 'regenerating' | 'completed' | 'error';

interface MeetingActionBarProps {
  meetingId: string;
  meetingFolderPath?: string | null;
  transcriptCount: number;
  onCopyTranscript: () => void;
  onOpenMeetingFolder: () => Promise<void>;
  onRefetchTranscripts?: () => Promise<void>;

  summaryRef: RefObject<BlockNoteSummaryViewRef | null>;
  aiSummary: MeetingSummary | null;
  summaryStatus: SummaryStatus;
  isSummaryDirty: boolean;
  isSaving: boolean;
  onSaveAll: () => Promise<void>;
  onCopySummary: () => Promise<void>;
  onGenerateSummary: (customPrompt: string) => Promise<void>;
  onStopGeneration: () => void;
  customPrompt: string;

  modelConfig: ModelConfig;
  setModelConfig: (config: ModelConfig | ((prev: ModelConfig) => ModelConfig)) => void;
  onSaveModelConfig: (config?: ModelConfig) => Promise<void>;
  isModelConfigLoading?: boolean;
  modelSettingsOpen: boolean;
  onModelSettingsOpenChange: (open: boolean) => void;

  availableTemplates: Array<{ id: string; name: string; description: string }>;
  selectedTemplate: string;
  onTemplateSelect: (templateId: string, templateName: string) => void;
}

/**
 * The one row of meeting actions: the frequent ones stay visible, the rest
 * (copy, enhance and summary settings) live in the "more" menu.
 */
export function MeetingActionBar({
  meetingId,
  meetingFolderPath,
  transcriptCount,
  onCopyTranscript,
  onOpenMeetingFolder,
  onRefetchTranscripts,
  summaryRef,
  aiSummary,
  summaryStatus,
  isSummaryDirty,
  isSaving,
  onSaveAll,
  onCopySummary,
  onGenerateSummary,
  onStopGeneration,
  customPrompt,
  modelConfig,
  setModelConfig,
  onSaveModelConfig,
  isModelConfigLoading = false,
  modelSettingsOpen,
  onModelSettingsOpenChange,
  availableTemplates,
  selectedTemplate,
  onTemplateSelect,
}: MeetingActionBarProps) {
  const { t } = useI18n();
  const { betaFeatures } = useConfig();
  const language = useSummaryLanguage({ id: meetingId });
  const router = useRouter();
  const { setMeetings, setCurrentMeeting } = useSidebar();
  const [confirmDelete, setConfirmDelete] = useState(false);

  // Same flow as deleting from the library, then back to the library.
  const handleDelete = async () => {
    try {
      await invoke('api_delete_meeting', { meetingId });
      setMeetings((current) => current.filter((meeting) => meeting.id !== meetingId));
      setCurrentMeeting({ id: 'intro-call', title: '+ New Call' });
      toast.success(t('Meeting deleted successfully'), {
        description: t('All associated data has been removed'),
      });
      router.push('/meetings');
    } catch (error) {
      console.error('Failed to delete meeting:', error);
      toast.error(t('Failed to delete meeting'), {
        description: error instanceof Error ? error.message : String(error),
      });
    }
  };

  const hasFolder = Boolean(meetingFolderPath);
  const hasTranscripts = transcriptCount > 0;
  const hasSummary = hasVisibleSummaryContent(aiSummary);
  const isGenerating = summaryStatus === 'processing' || summaryStatus === 'summarizing' || summaryStatus === 'regenerating';
  const canEnhance = betaFeatures.importAndRetranscribe && hasFolder;

  // --- Diarization -------------------------------------------------------
  const [isStartingDiarization, setIsStartingDiarization] = useState(false);
  const [isDiarizing, setIsDiarizing] = useState(false);
  const [isCancellingDiarization, setIsCancellingDiarization] = useState(false);
  const [showRetranscribeDialog, setShowRetranscribeDialog] = useState(false);

  const handleDiarize = useCallback(async () => {
    if (!meetingFolderPath) return;
    setIsStartingDiarization(true);
    try {
      await invoke('rerun_diarization', { meetingId, meetingFolderPath });
      toast.info(t('Speaker diarization started'));
    } catch (error) {
      toast.error(t('Could not start speaker diarization'), { description: String(error) });
    } finally {
      setIsStartingDiarization(false);
    }
  }, [meetingId, meetingFolderPath, t]);

  const handleStopDiarization = useCallback(async () => {
    setIsCancellingDiarization(true);
    try {
      await invoke('cancel_diarization', { meetingId });
    } catch (error) {
      toast.error(t('Could not stop speaker diarization'), { description: String(error) });
      setIsCancellingDiarization(false);
    }
  }, [meetingId, t]);

  // --- Read the summary aloud -------------------------------------------
  // Same text the copy action uses: the editor's markdown, falling back to
  // whatever the stored summary carries.
  const getSummaryMarkdown = useCallback(async () => {
    const fromEditor = await summaryRef.current?.getMarkdown?.();
    if (fromEditor) return fromEditor;
    if (aiSummary && typeof aiSummary.markdown === 'string') return aiSummary.markdown;
    return '';
  }, [summaryRef, aiSummary]);
  const speech = useSummarySpeech(getSummaryMarkdown);

  useEffect(() => {
    if (speech.error) toast.error(speech.error);
  }, [speech.error]);
  useEffect(() => {
    if (speech.notice) toast.info(speech.notice);
  }, [speech.notice]);

  const toggleSpeech = () => {
    void speech.toggle().catch((error) => {
      console.error('Summary speech failed:', error);
      toast.error(error instanceof Error ? error.message : t('Could not read the summary aloud'));
    });
  };
  const speechTitle = !speech.isActive
    ? t('Read the summary aloud')
    : speech.isPaused ? t('Resume reading') : t('Pause reading');
  const speechLabel = !speech.isActive ? t('Listen') : speech.isPaused ? t('Resume') : t('Pause');

  // Radix returns focus to the menu trigger as the menu closes, which would
  // dismiss a dialog or popover opened from the same click; open it after.
  const afterMenuCloses = (open: () => void) => () => {
    window.setTimeout(open, 0);
  };

  return (
    <div className="flex min-w-0 shrink-0 items-center justify-end gap-2">
      <DiarizationProgress
        meetingId={meetingId}
        onLabelsSaved={onRefetchTranscripts}
        onStatusChange={(inProgress) => {
          setIsDiarizing(inProgress);
          if (!inProgress) setIsCancellingDiarization(false);
        }}
      />

      <Button variant="ghost" size="sm" className="text-slate-600" onClick={onOpenMeetingFolder} title={t('Open Recording Folder')}>
        <FolderOpen />
        <span className="hidden @[60rem]:inline">{t('Folder')}</span>
      </Button>

      {/* Running jobs stay in view so they can be stopped; starting them lives in the menu. */}
      {isDiarizing && (
        <Button
          size="sm"
          variant="destructive"
          onClick={handleStopDiarization}
          disabled={isCancellingDiarization}
          title={t('Stop speaker diarization')}
        >
          <Square />
          <span className="hidden @[60rem]:inline">{isCancellingDiarization ? t('Stopping…') : t('Stop')}</span>
        </Button>
      )}

      {speech.isActive && (
        <>
          <Button variant="outline" size="sm" onClick={toggleSpeech} title={speechTitle}>
            {speech.isBuffering && !speech.isPaused
              ? <Loader2 className="animate-spin" />
              : speech.isPaused ? <Play /> : <Pause />}
            <span>{speechLabel}</span>
          </Button>
          <Button variant="outline" size="sm" onClick={speech.stop} title={t('Stop reading')} aria-label={t('Stop reading')}>
            <Square fill="currentColor" />
          </Button>
        </>
      )}

      {hasSummary && (isSummaryDirty || isSaving) && (
        <Button
          variant="outline"
          size="sm"
          className="border-emerald-300 bg-emerald-50 text-emerald-800 hover:bg-emerald-100"
          onClick={() => void onSaveAll()}
          disabled={isSaving}
          title={isSaving ? t('Saving') : t('Save Changes')}
        >
          {isSaving ? <Loader2 className="animate-spin" /> : <Save />}
          <span className="hidden @[60rem]:inline">{isSaving ? t('Saving...') : t('Save')}</span>
        </Button>
      )}

      {hasTranscripts && (isGenerating ? (
        <Button
          variant="outline"
          size="sm"
          className="border-red-200 bg-red-50 text-red-700 hover:bg-red-100"
          onClick={onStopGeneration}
          title={t('Stop summary generation')}
        >
          <Square fill="currentColor" />
          <span className="hidden @[40rem]:inline">{t('Stop')}</span>
        </Button>
      ) : (
        <Button
          size="sm"
          className="bg-indigo-600 text-white hover:bg-indigo-700"
          onClick={() => void onGenerateSummary(customPrompt)}
          disabled={isModelConfigLoading}
          title={isModelConfigLoading
            ? t('Loading model configuration...')
            : hasSummary ? t('Regenerate AI Summary') : t('Generate AI Summary')}
        >
          {isModelConfigLoading ? <Loader2 className="animate-spin" /> : <Sparkles />}
          <span className="hidden @[40rem]:inline">{hasSummary ? t('Regenerate Summary') : t('Generate Summary')}</span>
        </Button>
      ))}

      {/* The language picker is anchored to the "more" button and opened from its menu. */}
      <Popover open={language.langPickerOpen} onOpenChange={language.setLangPickerOpen}>
        <DropdownMenu>
          <PopoverAnchor asChild>
            <DropdownMenuTrigger asChild>
              <Button variant="ghost" size="sm" className="text-slate-600" title={t('More actions')} aria-label={t('More actions')}>
                <MoreHorizontal />
              </Button>
            </DropdownMenuTrigger>
          </PopoverAnchor>
          <DropdownMenuContent align="end" className="w-64">
            {hasFolder && !isDiarizing && (
              <DropdownMenuItem onSelect={() => void handleDiarize()} disabled={isStartingDiarization}>
                <UsersRound className="h-4 w-4" />
                {t('Identify speakers')}
              </DropdownMenuItem>
            )}
            {hasSummary && speech.isAvailable && !speech.isActive && (
              <DropdownMenuItem onSelect={toggleSpeech} disabled={isGenerating}>
                <Volume2 className="h-4 w-4" />
                {t('Read the summary aloud')}
              </DropdownMenuItem>
            )}
            {((hasFolder && !isDiarizing) || (hasSummary && speech.isAvailable && !speech.isActive)) && <DropdownMenuSeparator />}
            <DropdownMenuItem onSelect={onCopyTranscript} disabled={!hasTranscripts}>
              <Copy className="h-4 w-4" />
              {t('Copy Transcript')}
            </DropdownMenuItem>
            {hasSummary && (
              <DropdownMenuItem onSelect={() => void onCopySummary()}>
                <Copy className="h-4 w-4" />
                {t('Copy Summary')}
              </DropdownMenuItem>
            )}
            {canEnhance && (
              <DropdownMenuItem onSelect={afterMenuCloses(() => setShowRetranscribeDialog(true))}>
                <RefreshCw className="h-4 w-4" />
                {t('Enhance')}
              </DropdownMenuItem>
            )}

            {(hasTranscripts || hasSummary) && (
              <>
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  onSelect={afterMenuCloses(() => language.setLangPickerOpen(true))}
                  title={`${t('Summary language: {language}', { language: language.effectiveLangLabel })}${language.isLocalFallbackLanguage ? ` (${t('saved on this device')})` : ''}`}
                >
                  <Languages className="h-4 w-4" />
                  <span className="flex-1">{t('Summary language')}</span>
                  <span className="text-xs text-muted-foreground">{language.effectiveLangLabel}</span>
                </DropdownMenuItem>
                <DropdownMenuItem onSelect={afterMenuCloses(() => onModelSettingsOpenChange(true))}>
                  <Settings className="h-4 w-4" />
                  {t('AI Model')}
                </DropdownMenuItem>
                {availableTemplates.length > 0 && (
                  <DropdownMenuSub>
                    <DropdownMenuSubTrigger>
                      <FileText className="mr-2 h-4 w-4" />
                      {t('Template')}
                    </DropdownMenuSubTrigger>
                    <DropdownMenuSubContent className="w-56">
                      {availableTemplates.map((template) => (
                        <DropdownMenuItem
                          key={template.id}
                          onSelect={() => onTemplateSelect(template.id, template.name)}
                          title={t(template.description)}
                          className="flex items-center justify-between gap-2"
                        >
                          <span>{t(template.name)}</span>
                          {selectedTemplate === template.id && <Check className="h-4 w-4 text-emerald-600" />}
                        </DropdownMenuItem>
                      ))}
                    </DropdownMenuSubContent>
                  </DropdownMenuSub>
                )}
              </>
            )}

            <DropdownMenuSeparator />
            <DropdownMenuItem
              onSelect={afterMenuCloses(() => setConfirmDelete(true))}
              className="text-red-600 focus:bg-red-50 focus:text-red-700"
            >
              <Trash2 className="h-4 w-4" />
              {t('Delete')}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
        <PopoverContent align="end" className="w-auto border-0 bg-transparent p-0 shadow-none">
          <LanguagePickerPopover
            value={language.summaryLang}
            onChange={language.handleLangChange}
            onClose={() => language.setLangPickerOpen(false)}
            autoSubtitle={language.autoSubtitle}
          />
        </PopoverContent>
      </Popover>

      <ConfirmationModal
        isOpen={confirmDelete}
        text={t('Are you sure you want to delete this meeting? This action cannot be undone.')}
        onConfirm={() => {
          setConfirmDelete(false);
          void handleDelete();
        }}
        onCancel={() => setConfirmDelete(false)}
      />

      <Dialog open={modelSettingsOpen} onOpenChange={onModelSettingsOpenChange}>
        <DialogContent aria-describedby={undefined}>
          <VisuallyHidden>
            <DialogTitle>{t('Model Settings')}</DialogTitle>
          </VisuallyHidden>
          <ModelSettingsModal
            onSave={async (config) => {
              await onSaveModelConfig(config);
              onModelSettingsOpenChange(false);
            }}
            modelConfig={modelConfig}
            setModelConfig={setModelConfig}
            skipInitialFetch={true}
            layout="dialog"
          />
        </DialogContent>
      </Dialog>

      {canEnhance && meetingFolderPath && (
        <RetranscribeDialog
          open={showRetranscribeDialog}
          onOpenChange={setShowRetranscribeDialog}
          meetingId={meetingId}
          meetingFolderPath={meetingFolderPath}
          onComplete={async () => {
            await onRefetchTranscripts?.();
          }}
        />
      )}
    </div>
  );
}
