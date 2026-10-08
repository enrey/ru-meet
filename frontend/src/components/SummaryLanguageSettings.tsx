'use client';

import { useState } from 'react';
import { Globe, Pin } from 'lucide-react';
import { Popover, PopoverTrigger, PopoverContent } from '@/components/ui/popover';
import { LanguagePickerPopover } from '@/components/LanguagePickerPopover';
import { useRecentLanguages } from '@/hooks/useRecentLanguages';
import { labelForCode } from '@/lib/summary-languages';
import { useI18n } from '@/lib/i18n';

export function SummaryLanguageSettings() {
  const { recents, pinned, addRecent, removeRecent, setPinned } = useRecentLanguages();
  const { t } = useI18n();
  const [pickerOpen, setPickerOpen] = useState(false);

  const togglePin = (code: string) => {
    setPinned(pinned === code ? null : code);
  };

  return (
    <div className="bg-white rounded-lg border border-slate-200 p-6 shadow-sm relative">
      <div className="flex items-center gap-2 mb-2">
        <Globe size={18} className="text-slate-500" />
        <h3 className="text-lg font-semibold text-slate-900">{t('Summary Language')}</h3>
      </div>
      <p className="text-sm text-slate-600 mb-4">
        {t('Pin one language as the default for new meetings. Unpinned languages remain as quick-switch options in the summary generator. Auto uses the dominant transcript language.')}
      </p>

      <div className="flex flex-wrap items-center gap-2">
        {recents.map((code) => {
          const isPinned = pinned === code;
          return (
            <span
              key={code}
              className={`inline-flex items-center rounded-full border text-sm overflow-hidden ${
                isPinned
                  ? 'bg-indigo-50 border-indigo-200 text-indigo-800'
                  : 'bg-slate-100 border-slate-200 text-slate-800'
              }`}
            >
              <button
                type="button"
                aria-label={isPinned ? t('Unpin {language} as default', { language: labelForCode(code) }) : t('Pin {language} as default', { language: labelForCode(code) })}
                aria-pressed={isPinned}
                title={isPinned ? t('Click to unset as default') : t('Click to set as default')}
                onClick={() => togglePin(code)}
                className={`flex items-center gap-1.5 pl-3 pr-2 py-1 hover:brightness-95 active:brightness-90 ${
                  isPinned ? 'text-indigo-800' : 'text-slate-800'
                }`}
              >
                <Pin
                  size={14}
                  className={isPinned ? 'text-indigo-600' : 'text-slate-400'}
                  fill={isPinned ? 'currentColor' : 'none'}
                />
                {labelForCode(code)}
              </button>
              <button
                type="button"
                aria-label={t('Remove {language}', { language: labelForCode(code) })}
                onClick={() => removeRecent(code)}
                className={`pr-2.5 pl-0.5 py-1 leading-none ${isPinned ? 'text-indigo-400 hover:text-indigo-700' : 'text-slate-400 hover:text-slate-700'}`}
              >
                ×
              </button>
            </span>
          );
        })}

        <Popover open={pickerOpen} onOpenChange={setPickerOpen}>
          <PopoverTrigger asChild>
            <button
              type="button"
              disabled={recents.length >= 5}
              className="inline-flex items-center gap-1 rounded-full border border-dashed border-slate-300 px-3 py-1 text-sm text-slate-600 hover:border-slate-400 hover:text-slate-800 disabled:cursor-not-allowed disabled:opacity-50"
            >
              ＋ {t('Add language')}
            </button>
          </PopoverTrigger>
          <PopoverContent align="start" className="w-auto p-0 border-0 shadow-none bg-transparent">
            <LanguagePickerPopover
              mode="settings"
              value={null}
              onChange={(code) => {
                if (code) addRecent(code);
                setPickerOpen(false);
              }}
              onClose={() => setPickerOpen(false)}
            />
          </PopoverContent>
        </Popover>
      </div>

      <p className="text-xs text-slate-400 mt-3">
        {pinned
          ? t('Default: {language} - click it again to unset. Max 5 quick-switch options.', { language: labelForCode(pinned) })
          : t('Click any language to set it as your default. Max 5 quick-switch options.')}
      </p>
    </div>
  );
}
