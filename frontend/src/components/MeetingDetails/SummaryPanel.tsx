"use client";

import { MeetingSummary, Summary } from '@/types';
import { BlockNoteSummaryView, BlockNoteSummaryViewRef } from '@/components/AISummary/BlockNoteSummaryView';
import { EmptyStateSummary } from '@/components/EmptyStateSummary';
import { ModelConfig } from '@/components/ModelSettingsModal';
import { RefObject } from 'react';
import { useSummaryProgress } from '@/hooks/meeting-details/useSummaryProgress';
import { hasVisibleSummaryContent } from '@/lib/summary-content';
import { getIntlLocale, useI18n } from '@/lib/i18n';

type SummaryStatus = 'idle' | 'processing' | 'summarizing' | 'regenerating' | 'completed' | 'error';

interface SummaryPanelProps {
  meeting: {
    id: string;
    title: string;
    created_at: string;
  };
  meetingTitle: string;
  summaryRef: RefObject<BlockNoteSummaryViewRef | null>;
  aiSummary: MeetingSummary | null;
  summaryStatus: SummaryStatus;
  modelConfig: ModelConfig;
  onGenerateSummary: (customPrompt: string) => Promise<void>;
  customPrompt: string;
  onPromptChange: (value: string) => void;
  onSaveSummary: (summary: MeetingSummary) => Promise<void>;
  onSummaryChange: (summary: Summary) => void;
  onDirtyChange: (isDirty: boolean) => void;
  summaryError: string | null;
  onRegenerateSummary: () => Promise<void>;
  getSummaryStatusMessage: (status: SummaryStatus) => string;
}

/** The summary pane; its actions live in the meeting's single action bar. */
export function SummaryPanel({
  meeting,
  meetingTitle,
  summaryRef,
  aiSummary,
  summaryStatus,
  modelConfig,
  onGenerateSummary,
  customPrompt,
  onPromptChange,
  onSaveSummary,
  onSummaryChange,
  onDirtyChange,
  summaryError,
  onRegenerateSummary,
  getSummaryStatusMessage,
}: SummaryPanelProps) {
  const { t, locale } = useI18n();
  const isSummaryLoading = summaryStatus === 'processing' || summaryStatus === 'summarizing' || summaryStatus === 'regenerating';
  const hasSummary = hasVisibleSummaryContent(aiSummary);
  const summaryProgress = useSummaryProgress(isSummaryLoading);

  return (
    <div className="flex-1 min-w-0 flex flex-col bg-white overflow-hidden h-full w-full @container">
      {isSummaryLoading ? (
        <div className="flex items-center justify-center flex-1">
          <div className="text-center">
            <div className="inline-block animate-spin rounded-full h-12 w-12 border-t-2 border-b-2 border-blue-500 mb-4"></div>
            <p className="text-gray-600">{t('Generating AI Summary...')}</p>
            {summaryProgress ? (
              <p className="text-sm text-gray-500 mt-2 tabular-nums">
                {t('{count} tokens generated', { count: summaryProgress.generatedTokens.toLocaleString(getIntlLocale(locale)) })}
                {' · '}
                {t('{rate} tok/s', { rate: summaryProgress.tokensPerSec.toFixed(1) })}
                {' · '}
                {t('prompt {count}', { count: summaryProgress.promptTokens.toLocaleString(getIntlLocale(locale)) })}
              </p>
            ) : (
              <p className="text-sm text-gray-500 mt-2">{t('Reading the transcript…')}</p>
            )}
          </div>
        </div>
      ) : !hasSummary ? (
        <EmptyStateSummary
          onGenerate={() => onGenerateSummary(customPrompt)}
          customPrompt={customPrompt}
          onPromptChange={onPromptChange}
          hasModel={modelConfig.provider !== null && modelConfig.model !== null}
          isGenerating={isSummaryLoading}
          error={summaryError}
        />
      ) : (
        <div className="flex-1 overflow-y-auto overflow-x-auto min-h-0">
          <div className="w-full px-8 py-6">
            <BlockNoteSummaryView
              ref={summaryRef}
              summaryData={aiSummary}
              onSave={onSaveSummary}
              onSummaryChange={onSummaryChange}
              onDirtyChange={onDirtyChange}
              status={summaryStatus}
              error={summaryError}
              onRegenerateSummary={onRegenerateSummary}
              meeting={{
                id: meeting.id,
                title: meetingTitle,
                created_at: meeting.created_at
              }}
            />
          </div>
          {summaryStatus !== 'idle' && (
            <div className={`mt-4 p-4 rounded-lg ${summaryStatus === 'error' ? 'bg-red-100 text-red-700' :
              summaryStatus === 'completed' ? 'bg-green-100 text-green-700' :
                'bg-blue-100 text-blue-700'
              }`}>
              <p className="text-sm font-medium">{getSummaryStatusMessage(summaryStatus)}</p>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
