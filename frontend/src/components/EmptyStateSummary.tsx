'use client';

import { motion } from 'framer-motion';
import { FileQuestion, Sparkles } from 'lucide-react';
import { Button } from '@/components/ui/button';
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip';
import { Switch } from '@/components/ui/switch';
import { useConfig } from '@/contexts/ConfigContext';
import { useI18n } from '@/lib/i18n';

interface EmptyStateSummaryProps {
  onGenerate: () => void;
  customPrompt?: string;
  onPromptChange?: (value: string) => void;
  hasModel: boolean;
  isGenerating?: boolean;
  error?: string | null;
}

export function EmptyStateSummary({
  onGenerate,
  customPrompt = '',
  onPromptChange = () => {},
  hasModel,
  isGenerating = false,
  error = null,
}: EmptyStateSummaryProps) {
  const { t } = useI18n();
  const { isAutoSummary, toggleIsAutoSummary } = useConfig();
  return (
    <motion.div
      initial={{ opacity: 0, scale: 0.95 }}
      animate={{ opacity: 1, scale: 1 }}
      transition={{ duration: 0.3, ease: 'easeOut' }}
      className="flex flex-col items-center justify-start h-full overflow-y-auto px-8 pb-8 pt-12 text-center"
    >
      <FileQuestion className="w-16 h-16 text-slate-300 mb-4" />
      <h3 className="text-lg font-semibold text-slate-900 mb-2">
        {t('No Summary Generated Yet')}
      </h3>
      <p className="text-sm text-slate-500 mb-6 max-w-md">
        {t('Generate an AI-powered summary of your meeting transcript to get key points, action items, and decisions.')}
      </p>

      {error && (
        <p role="alert" className="mb-4 max-w-md rounded-md bg-red-50 px-3 py-2 text-sm text-red-700">
          {error}
        </p>
      )}

      <textarea
        placeholder={t('Add summary context — people involved, meeting overview, objectives…')}
        className="mb-4 min-h-[96px] w-full max-w-md resize-y rounded-md border border-slate-200 bg-white px-3 py-2 text-sm text-left shadow-sm focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
        value={customPrompt}
        onChange={(event) => onPromptChange(event.target.value)}
      />

      <TooltipProvider>
        <Tooltip>
          <TooltipTrigger asChild>
            <div>
              <Button
                onClick={onGenerate}
                disabled={!hasModel || isGenerating}
                size="lg"
                className="gap-2 bg-indigo-600 text-white hover:bg-indigo-700"
              >
                <Sparkles className="w-4 h-4" />
                {isGenerating ? t('Generating...') : error ? t('Retry summary') : t('Generate Summary')}
              </Button>
            </div>
          </TooltipTrigger>
          {!hasModel && (
            <TooltipContent>
              <p>{t('Please select a model in Settings first')}</p>
            </TooltipContent>
          )}
        </Tooltip>
      </TooltipProvider>

      {!hasModel && (
        <p className="text-xs text-amber-600 mt-3">
          {t('Please select a model in Settings first')}
        </p>
      )}

      {/* Same switch as Settings › Summary: summaries right after each recording. */}
      <label className="mt-6 flex w-full max-w-md cursor-pointer items-start gap-3 rounded-lg border border-slate-200 bg-slate-50 px-4 py-3 text-left">
        <Switch checked={isAutoSummary} onCheckedChange={toggleIsAutoSummary} className="mt-0.5" />
        <span className="text-sm">
          <span className="block font-medium text-slate-800">{t('Automatic summary')}</span>
          <span className="block text-slate-500">
            {isAutoSummary
              ? t('A summary is created automatically after each recording.')
              : t('Turn on to get a summary automatically after each recording.')}
          </span>
        </span>
      </label>
    </motion.div>
  );
}
