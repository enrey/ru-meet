'use client';

import { Play, Sparkles } from 'lucide-react';
import { useRouter } from 'next/navigation';
import { Button } from '@/components/ui/button';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { useI18n } from '@/lib/i18n';
import type { MeetingSummaryState } from './MeetingHeader';

interface SummaryTabActionProps {
  summaryState: MeetingSummaryState;
  /** Summary or its reading is being made; the header already shows progress. */
  busy: boolean;
  /** The reading is playing; the player has the controls then. */
  playing: boolean;
  /** Whether reading aloud is on and its model installed; null while unknown. */
  speechAvailable: boolean | null;
  onCreate: () => void;
  onListen: () => void;
}

const ICON_BUTTON =
  'flex h-7 w-7 items-center justify-center rounded-md transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-indigo-400';

/**
 * The summary is optional: next to its tab, a shortcut to create it when it
 * is missing, or to listen to it once it exists. Hidden while something runs
 * or plays, since the header and the player show those.
 */
export function SummaryTabAction({
  summaryState,
  busy,
  playing,
  speechAvailable,
  onCreate,
  onListen,
}: SummaryTabActionProps) {
  const { t } = useI18n();
  const router = useRouter();

  if (busy || summaryState === 'processing') return null;

  if (summaryState === 'none' || summaryState === 'error') {
    const label = summaryState === 'error' ? t('Create the summary again') : t('Create summary');
    return (
      <button
        type="button"
        onClick={onCreate}
        title={label}
        aria-label={label}
        className={`${ICON_BUTTON} text-indigo-500 hover:bg-indigo-50 hover:text-indigo-700`}
      >
        <Sparkles className="h-4 w-4" />
      </button>
    );
  }

  if (playing || speechAvailable === null) return null;

  if (speechAvailable) {
    return (
      <button
        type="button"
        onClick={onListen}
        title={t('Listen to the summary')}
        aria-label={t('Listen to the summary')}
        className={`${ICON_BUTTON} text-indigo-500 hover:bg-indigo-50 hover:text-indigo-700`}
      >
        <Play className="h-4 w-4 fill-current" />
      </button>
    );
  }

  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          type="button"
          title={t('Listen to the summary')}
          aria-label={t('Listen to the summary')}
          className={`${ICON_BUTTON} text-slate-300 hover:bg-slate-50 hover:text-slate-500`}
        >
          <Play className="h-4 w-4 fill-current" />
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-72 space-y-3 text-sm">
        <p className="font-medium text-slate-900">{t('Reading aloud is off or not installed')}</p>
        <p className="text-slate-600">
          {t('Turn it on and download the voice model in the settings to listen to summaries. The voice runs on this computer.')}
        </p>
        <Button size="sm" className="w-full" onClick={() => router.push('/settings#beta')}>
          {t('Open reading aloud settings')}
        </Button>
      </PopoverContent>
    </Popover>
  );
}
