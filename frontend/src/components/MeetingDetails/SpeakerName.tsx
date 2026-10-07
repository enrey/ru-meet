'use client';

import { useState } from 'react';
import { Pencil } from 'lucide-react';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { useMeetingSpeakers } from '@/contexts/MeetingSpeakersContext';
import { useI18n } from '@/lib/i18n';

/**
 * A speaker's coloured name. Clicking it renames the speaker everywhere on
 * the meeting page; outside a meeting page it is plain text.
 */
export function SpeakerName({ speaker, className = '' }: { speaker: string; className?: string }) {
  const speakers = useMeetingSpeakers();
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState('');
  const [mergeTarget, setMergeTarget] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  if (!speakers) {
    return <span className={`text-xs font-medium text-blue-600 ${className}`}>{speaker}</span>;
  }

  const name = speakers.displayName(speaker);
  const color = speakers.colorFor(speaker);

  const close = () => {
    if (saving) return;
    setOpen(false);
    setMergeTarget(null);
  };

  const save = async () => {
    if (saving) return;
    setSaving(true);
    try {
      const result = await speakers.requestRename(speaker, draft);
      if (result.kind === 'needs-merge') setMergeTarget(result.target);
      else if (result.kind !== 'invalid') setOpen(false);
    } finally {
      setSaving(false);
    }
  };

  const merge = async () => {
    if (!mergeTarget || saving) return;
    setSaving(true);
    try {
      if (await speakers.commitRename(speaker, mergeTarget)) {
        setOpen(false);
        setMergeTarget(null);
      }
    } finally {
      setSaving(false);
    }
  };

  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        if (next) {
          setDraft(name);
          setMergeTarget(null);
          setOpen(true);
        } else {
          close();
        }
      }}
    >
      <PopoverTrigger asChild>
        <button
          type="button"
          title={t('Rename {speaker}', { speaker: name })}
          className={`group inline-flex min-w-0 items-center gap-1.5 rounded text-left text-xs font-semibold text-gray-700 hover:text-gray-950 focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-400 ${className}`}
        >
          <span aria-hidden className="h-2.5 w-2.5 shrink-0 rounded-full" style={{ backgroundColor: color ?? '#9ca3af' }} />
          <span className="truncate">{name}</span>
          <Pencil size={11} className="shrink-0 text-gray-400 opacity-0 group-hover:opacity-100 group-focus-visible:opacity-100" />
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-72 p-3">
        {mergeTarget ? (
          <div className="space-y-3">
            <p className="text-sm text-gray-700">
              {t('Merge into “{name}”? Their lines become one speaker. Only re-running diarization undoes this.', {
                name: speakers.displayName(mergeTarget),
              })}
            </p>
            <div className="flex justify-end gap-2">
              <button type="button" disabled={saving} onClick={() => setMergeTarget(null)}
                className="rounded-md px-3 py-1.5 text-sm text-gray-600 hover:bg-gray-100 disabled:opacity-50">
                {t('Cancel')}
              </button>
              <button type="button" disabled={saving} onClick={() => void merge()}
                className="rounded-md bg-blue-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-blue-700 disabled:opacity-50">
                {t('Merge')}
              </button>
            </div>
          </div>
        ) : (
          <form
            className="space-y-3"
            onSubmit={(event) => {
              event.preventDefault();
              void save();
            }}
          >
            <label className="block text-xs font-medium text-gray-500" htmlFor={`speaker-name-${speaker}`}>
              {t('Speaker name')}
            </label>
            <input
              id={`speaker-name-${speaker}`}
              autoFocus
              value={draft}
              maxLength={80}
              disabled={saving}
              onChange={(event) => setDraft(event.target.value)}
              onFocus={(event) => event.currentTarget.select()}
              className="w-full rounded-md border border-gray-300 px-2.5 py-1.5 text-sm outline-none focus:border-blue-400 focus:ring-2 focus:ring-blue-100"
            />
            <div className="flex justify-end gap-2">
              <button type="button" disabled={saving} onClick={close}
                className="rounded-md px-3 py-1.5 text-sm text-gray-600 hover:bg-gray-100 disabled:opacity-50">
                {t('Cancel')}
              </button>
              <button type="submit" disabled={saving}
                className="rounded-md bg-blue-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-blue-700 disabled:opacity-50">
                {t('Save')}
              </button>
            </div>
          </form>
        )}
      </PopoverContent>
    </Popover>
  );
}
