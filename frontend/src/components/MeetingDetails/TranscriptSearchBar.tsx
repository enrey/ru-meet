'use client';

import { forwardRef } from 'react';
import { ChevronDown, ChevronUp, Loader2, Search, X } from 'lucide-react';
import { useI18n } from '@/lib/i18n';

interface TranscriptSearchBarProps {
  query: string;
  onQueryChange: (query: string) => void;
  matchCount: number;
  position: number;
  searching: boolean;
  onNext: () => void;
  onPrevious: () => void;
}

/** Find in transcript: Enter / Shift+Enter step through matches, Esc clears. */
export const TranscriptSearchBar = forwardRef<HTMLInputElement, TranscriptSearchBarProps>(
  ({ query, onQueryChange, matchCount, position, searching, onNext, onPrevious }, ref) => {
    const { t } = useI18n();
    const hasQuery = query.trim().length > 0;

    return (
      <div className="flex min-w-0 items-center gap-2">
        <div className="flex h-8 w-72 min-w-0 items-center gap-2 rounded-md border border-slate-200 bg-white px-2.5 focus-within:border-indigo-300 focus-within:ring-2 focus-within:ring-indigo-100">
          {searching ? <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-slate-400" /> : <Search className="h-3.5 w-3.5 shrink-0 text-slate-400" />}
          <input
            ref={ref}
            value={query}
            onChange={(event) => onQueryChange(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter') {
                event.preventDefault();
                if (event.shiftKey) onPrevious();
                else onNext();
              } else if (event.key === 'Escape') {
                event.preventDefault();
                onQueryChange('');
                event.currentTarget.blur();
              }
            }}
            placeholder={t('Search in transcript')}
            aria-label={t('Search in transcript')}
            className="min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-slate-400"
          />
          {hasQuery ? (
            <button type="button" onClick={() => onQueryChange('')} aria-label={t('Clear')} className="text-slate-400 hover:text-slate-600">
              <X className="h-3.5 w-3.5" />
            </button>
          ) : (
            <kbd className="rounded border border-slate-200 bg-slate-50 px-1 text-[10px] text-slate-500">Ctrl F</kbd>
          )}
        </div>
        {hasQuery && !searching && (
          <>
            <span className="whitespace-nowrap text-xs tabular-nums text-slate-500" aria-live="polite">
              {matchCount ? t('{position} of {count}', { position, count: matchCount }) : t('No matches')}
            </span>
            <button
              type="button"
              onClick={onPrevious}
              disabled={matchCount === 0}
              aria-label={t('Previous match')}
              title={`${t('Previous match')} (Shift+Enter)`}
              className="grid h-7 w-7 place-items-center rounded-md text-slate-500 hover:bg-slate-100 disabled:opacity-40"
            >
              <ChevronUp className="h-4 w-4" />
            </button>
            <button
              type="button"
              onClick={onNext}
              disabled={matchCount === 0}
              aria-label={t('Next match')}
              title={`${t('Next match')} (Enter)`}
              className="grid h-7 w-7 place-items-center rounded-md text-slate-500 hover:bg-slate-100 disabled:opacity-40"
            >
              <ChevronDown className="h-4 w-4" />
            </button>
          </>
        )}
      </div>
    );
  },
);
TranscriptSearchBar.displayName = 'TranscriptSearchBar';
