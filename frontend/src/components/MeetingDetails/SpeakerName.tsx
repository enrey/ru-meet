'use client';

import { useState, type ReactNode } from 'react';
import { Pencil } from 'lucide-react';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { useMeetingSpeakers } from '@/contexts/MeetingSpeakersContext';
import { useI18n } from '@/lib/i18n';

/**
 * A speaker's coloured name. Clicking it renames the speaker everywhere on
 * the meeting page; outside a meeting page it is plain text.
 */
export function SpeakerName({ speaker, className = '', showDot = true }: { speaker: string; className?: string; showDot?: boolean }) {
  const speakers = useMeetingSpeakers();
  const { t } = useI18n();

  if (!speakers) {
    return <span className={`text-xs font-medium text-indigo-600 ${className}`}>{speaker}</span>;
  }

  const name = speakers.displayName(speaker);
  const color = speakers.colorFor(speaker);

  return (
    <SpeakerRenamePopover speaker={speaker}>
      <button
        type="button"
        title={t('Rename {speaker}', { speaker: name })}
        className={`group inline-flex min-w-0 items-center gap-1.5 rounded text-left text-xs font-semibold text-slate-700 hover:text-slate-950 focus:outline-none focus-visible:ring-2 focus-visible:ring-indigo-400 ${className}`}
      >
        {showDot && <span aria-hidden className="h-2.5 w-2.5 shrink-0 rounded-full" style={{ backgroundColor: color ?? '#94a3b8' }} />}
        <span className="truncate">{name}</span>
        <Pencil size={11} className="shrink-0 text-slate-400 opacity-0 group-hover:opacity-100 group-focus-visible:opacity-100" />
      </button>
    </SpeakerRenamePopover>
  );
}

/** Rename (or merge) `speaker`, opened from `children` as the trigger. */
export function SpeakerRenamePopover({ speaker, children }: { speaker: string; children: ReactNode }) {
  const speakers = useMeetingSpeakers();
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState('');
  const [mergeTarget, setMergeTarget] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  if (!speakers) return <>{children}</>;

  const name = speakers.displayName(speaker);

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
      <PopoverTrigger asChild>{children}</PopoverTrigger>
      <PopoverContent align="start" className="w-72 p-3">
        {mergeTarget ? (
          <div className="space-y-3">
            <p className="text-sm text-slate-700">
              {t('Merge into “{name}”? Their lines become one speaker. Only re-running diarization undoes this.', {
                name: speakers.displayName(mergeTarget),
              })}
            </p>
            <div className="flex justify-end gap-2">
              <button type="button" disabled={saving} onClick={() => setMergeTarget(null)}
                className="rounded-md px-3 py-1.5 text-sm text-slate-600 hover:bg-slate-100 disabled:opacity-50">
                {t('Cancel')}
              </button>
              <button type="button" disabled={saving} onClick={() => void merge()}
                className="rounded-md bg-indigo-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-indigo-700 disabled:opacity-50">
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
            <label className="block text-xs font-medium text-slate-500" htmlFor={`speaker-name-${speaker}`}>
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
              className="w-full rounded-md border border-slate-300 px-2.5 py-1.5 text-sm outline-none focus:border-indigo-400 focus:ring-2 focus:ring-indigo-100"
            />
            <div className="flex justify-end gap-2">
              <button type="button" disabled={saving} onClick={close}
                className="rounded-md px-3 py-1.5 text-sm text-slate-600 hover:bg-slate-100 disabled:opacity-50">
                {t('Cancel')}
              </button>
              <button type="submit" disabled={saving}
                className="rounded-md bg-indigo-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-indigo-700 disabled:opacity-50">
                {t('Save')}
              </button>
            </div>
          </form>
        )}
      </PopoverContent>
    </Popover>
  );
}

/** "Speaker 3" → "3", "Анна Петрова" → "АП". */
function initials(name: string): string {
  const number = /(\d+)$/.exec(name);
  if (number) return number[1];
  return name.split(/\s+/).filter(Boolean).slice(0, 2).map((word) => word[0]).join('').toUpperCase() || '?';
}

/** Round badge in the speaker's colour; `active` fills it solid. */
export function SpeakerAvatar({ speaker, active = false }: { speaker?: string; active?: boolean }) {
  const speakers = useMeetingSpeakers();
  const color = (speaker && speakers?.colorFor(speaker)) || '#94a3b8';
  const name = speaker ? speakers?.displayName(speaker) ?? speaker : '';
  return (
    <span
      aria-hidden
      className="grid h-7 w-7 shrink-0 place-items-center rounded-full text-xs font-semibold transition-colors"
      style={active ? { backgroundColor: color, color: '#fff' } : { backgroundColor: `${color}26`, color }}
    >
      {initials(name)}
    </span>
  );
}
