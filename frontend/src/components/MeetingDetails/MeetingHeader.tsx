'use client';

import { useEffect, useRef, useState, type ReactNode } from 'react';
import { ArrowLeft, Loader2, Pencil } from 'lucide-react';
import { useRouter } from 'next/navigation';
import { getIntlLocale, useI18n, type TranslateFn } from '@/lib/i18n';
import { useMeetingSpeakers } from '@/contexts/MeetingSpeakersContext';

export type MeetingSummaryState = 'none' | 'processing' | 'ready' | 'error';
export type MeetingDetailsTab = 'summary' | 'transcript';

const TABS: { value: MeetingDetailsTab; label: string }[] = [
  { value: 'summary', label: 'Summary' },
  { value: 'transcript', label: 'Transcript' },
];

interface MeetingHeaderProps {
  title: string;
  createdAt?: string;
  durationSeconds?: number | null;
  speakerCount?: number;
  summaryState: MeetingSummaryState;
  /** Persist a new title; rejecting keeps the editor open. */
  onRename: (title: string) => Promise<void>;
  activeTab: MeetingDetailsTab;
  onTabChange: (tab: MeetingDetailsTab) => void;
  /** Tools for the active tab, right-aligned in the tab row (e.g. search). */
  tabTools?: ReactNode;
  /** Meeting actions, right-aligned next to the title. */
  children?: ReactNode;
}

function formatDuration(seconds: number, t: TranslateFn): string {
  const totalMinutes = Math.round(seconds / 60);
  if (totalMinutes < 1) return t('Less than a minute');
  const hours = Math.floor(totalMinutes / 60);
  const minutes = totalMinutes % 60;
  return hours > 0 ? t('{hours} h {minutes} min', { hours, minutes }) : t('{minutes} min', { minutes });
}

function SummaryChip({ state, t }: { state: MeetingSummaryState; t: TranslateFn }) {
  switch (state) {
    case 'processing':
      return (
        <span className="inline-flex items-center gap-1.5 rounded-full bg-amber-50 px-2 py-0.5 text-xs font-medium text-amber-700">
          <Loader2 className="h-3 w-3 animate-spin" />
          {t('Processing')}
        </span>
      );
    case 'ready':
      return (
        <span className="inline-flex items-center gap-1.5 rounded-full bg-emerald-50 px-2 py-0.5 text-xs font-medium text-emerald-700">
          <span className="h-1.5 w-1.5 rounded-full bg-current" />
          {t('Summary')}
        </span>
      );
    case 'error':
      return <span className="rounded-full bg-red-50 px-2 py-0.5 text-xs font-medium text-red-700">{t('Error')}</span>;
    default:
      return <span className="rounded-full bg-slate-100 px-2 py-0.5 text-xs font-medium text-slate-600">{t('Transcript')}</span>;
  }
}

/**
 * Header of an opened meeting: back to the library (also Alt+← and the mouse
 * "back" button), the title — renamed in place by clicking it — and its facts.
 */
export function MeetingHeader({
  title,
  createdAt,
  durationSeconds,
  speakerCount,
  summaryState,
  onRename,
  activeTab,
  onTabChange,
  tabTools,
  children,
}: MeetingHeaderProps) {
  const router = useRouter();
  const { t } = useI18n();
  // Live count: follows renames and merges made on this page.
  const liveSpeakerCount = useMeetingSpeakers()?.speakers.length;
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(title);
  const [saving, setSaving] = useState(false);
  // Enter commits and then blurs; the blur must not save a second time.
  const committingRef = useRef(false);

  useEffect(() => {
    const back = () => router.push('/meetings');
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.altKey && event.key === 'ArrowLeft') {
        event.preventDefault();
        back();
      }
    };
    const onMouseUp = (event: MouseEvent) => {
      if (event.button === 3) {
        event.preventDefault();
        back();
      }
    };
    window.addEventListener('keydown', onKeyDown);
    window.addEventListener('mouseup', onMouseUp);
    return () => {
      window.removeEventListener('keydown', onKeyDown);
      window.removeEventListener('mouseup', onMouseUp);
    };
  }, [router]);

  useEffect(() => {
    if (!editing) setDraft(title);
  }, [title, editing]);

  const commit = async () => {
    if (committingRef.current) return;
    const next = draft.trim();
    if (!next || next === title) {
      setEditing(false);
      setDraft(title);
      return;
    }
    committingRef.current = true;
    setSaving(true);
    try {
      await onRename(next);
      setEditing(false);
    } catch {
      // The caller reported the error; keep the text so it can be retried.
    } finally {
      setSaving(false);
      committingRef.current = false;
    }
  };

  const created = createdAt ? new Date(createdAt) : null;
  const date = created && !Number.isNaN(created.getTime())
    ? new Intl.DateTimeFormat(getIntlLocale(), { day: 'numeric', month: 'long', year: 'numeric', hour: '2-digit', minute: '2-digit' }).format(created)
    : null;
  const facts = [
    date,
    durationSeconds != null && Number.isFinite(durationSeconds) ? formatDuration(durationSeconds, t) : null,
    (liveSpeakerCount || speakerCount) ? t('{count} speakers', { count: liveSpeakerCount || speakerCount || 0 }) : null,
  ].filter((fact): fact is string => Boolean(fact));

  return (
    <header className="@container shrink-0 border-b border-slate-200 bg-white px-8 pt-2">
      <button
        type="button"
        onClick={() => router.push('/meetings')}
        title={`${t('Back to meetings')} (Alt+←)`}
        className="-ml-2 inline-flex items-center gap-1.5 rounded-md px-2 py-1 text-xs font-medium text-slate-500 transition-colors hover:bg-slate-100 hover:text-slate-900"
      >
        <ArrowLeft className="h-3.5 w-3.5" />
        {t('Meetings')}
      </button>

      <div className="mt-0.5 flex items-start gap-3">
        <div className="min-w-0 flex-1">
          {editing ? (
            <input
              autoFocus
              value={draft}
              disabled={saving}
              aria-label={t('Rename meeting')}
              onChange={(event) => setDraft(event.target.value)}
              onFocus={(event) => event.currentTarget.select()}
              onBlur={() => void commit()}
              onKeyDown={(event) => {
                if (event.key === 'Enter') {
                  event.preventDefault();
                  void commit();
                } else if (event.key === 'Escape') {
                  event.preventDefault();
                  setDraft(title);
                  setEditing(false);
                }
              }}
              className="-ml-1.5 w-full rounded-md border border-indigo-300 bg-white px-1.5 py-0.5 text-xl font-semibold text-slate-900 outline-none ring-2 ring-indigo-100"
            />
          ) : (
            <button
              type="button"
              onClick={() => setEditing(true)}
              title={t('Rename meeting')}
              className="group -ml-1.5 flex max-w-full items-center gap-2 rounded-md px-1.5 py-0.5 text-left transition-colors hover:bg-slate-50"
            >
              <h1 className="line-clamp-2 break-words text-xl font-semibold text-slate-900" title={title}>{title}</h1>
              <Pencil className="h-4 w-4 shrink-0 text-slate-400 opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100" />
            </button>
          )}
          <div className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-1 text-sm text-slate-500">
            {facts.map((fact, index) => (
              <span key={fact} className="inline-flex items-center gap-2 tabular-nums">
                {index > 0 && <span aria-hidden className="text-slate-300">·</span>}
                {fact}
              </span>
            ))}
            {facts.length > 0 && <span aria-hidden className="text-slate-300">·</span>}
            <SummaryChip state={summaryState} t={t} />
          </div>
        </div>
        {children}
      </div>

      <div className="mt-3 flex items-end justify-between gap-4">
      <div role="tablist" aria-label={t('Meeting sections')} className="flex gap-6">
        {TABS.map(({ value, label }) => {
          const selected = activeTab === value;
          return (
            <button
              key={value}
              type="button"
              role="tab"
              id={`meeting-tab-${value}`}
              aria-selected={selected}
              aria-controls={`meeting-panel-${value}`}
              onClick={() => onTabChange(value)}
              className={`-mb-px border-b-2 pb-2.5 text-sm font-semibold transition-colors focus:outline-none focus-visible:text-indigo-700 ${
                selected ? 'border-indigo-500 text-slate-900' : 'border-transparent text-slate-500 hover:text-slate-800'
              }`}
            >
              {t(label)}
            </button>
          );
        })}
      </div>
      {/* Fixed height whether or not the tab has tools, so the tab labels don't jump. */}
      <div className="mb-2 flex h-8 min-w-0 items-center justify-end">{tabTools}</div>
      </div>
    </header>
  );
}
