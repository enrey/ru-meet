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
  return (
    <motion.div
      initial={{ opacity: 0, scale: 0.95 }}
      animate={{ opacity: 1, scale: 1 }}
      transition={{ duration: 0.3, ease: 'easeOut' }}
      className="flex flex-col items-center justify-center h-full p-8 text-center"
    >
      <FileQuestion className="w-16 h-16 text-gray-300 mb-4" />
      <h3 className="text-lg font-semibold text-gray-900 mb-2">
        No Summary Generated Yet
      </h3>
      <p className="text-sm text-gray-500 mb-6 max-w-md">
        Generate an AI-powered summary of your meeting transcript to get key points, action items, and decisions.
      </p>

      {error && (
        <p role="alert" className="mb-4 max-w-md rounded-md bg-red-50 px-3 py-2 text-sm text-red-700">
          {error}
        </p>
      )}

      <textarea
        placeholder="Add summary context — people involved, meeting overview, objectives…"
        className="mb-4 min-h-[96px] w-full max-w-md resize-y rounded-md border border-gray-200 bg-white px-3 py-2 text-sm text-left shadow-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
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
                className="gap-2"
              >
                <Sparkles className="w-4 h-4" />
                {isGenerating ? 'Generating...' : error ? 'Retry summary' : 'Generate Summary'}
              </Button>
            </div>
          </TooltipTrigger>
          {!hasModel && (
            <TooltipContent>
              <p>Please select a model in Settings first</p>
            </TooltipContent>
          )}
        </Tooltip>
      </TooltipProvider>

      {!hasModel && (
        <p className="text-xs text-amber-600 mt-3">
          Please select a model in Settings first
        </p>
      )}
    </motion.div>
  );
}
