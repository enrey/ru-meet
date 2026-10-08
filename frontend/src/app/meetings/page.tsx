'use client';

import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useRouter } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { AudioLines, Loader2, Mic, Pencil, Search, Trash2, Upload, UsersRound, X } from 'lucide-react';
import { useSidebar, type MeetingListItem } from '@/components/Sidebar/SidebarProvider';
import { ConfirmationModal } from '@/components/ConfirmationModel/confirmation-modal';
import { Dialog, DialogContent, DialogFooter, DialogTitle } from '@/components/ui/dialog';
import { VisuallyHidden } from '@/components/ui/visually-hidden';
import { useImportDialog } from '@/contexts/ImportDialogContext';
import { useConfig } from '@/contexts/ConfigContext';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { getIntlLocale, useI18n, type TranslateFn } from '@/lib/i18n';

type SummaryFilter = 'all' | 'with' | 'without';

const FILTERS: { value: SummaryFilter; label: string }[] = [
  { value: 'all', label: 'All' },
  { value: 'with', label: 'With summary' },
  { value: 'without', label: 'Without summary' },
];

const SEARCH_DEBOUNCE_MS = 250;
const ROW_GRID = 'grid grid-cols-[minmax(0,1fr)_130px_120px_90px_130px_76px] items-center gap-4 px-5';

/** Row icon tint follows the summary state, so the list also scans by colour. */
function rowIconClass(meeting: MeetingListItem): string {
  if (meeting.summaryStatus === 'pending' || meeting.summaryStatus === 'processing') return 'bg-amber-50 text-amber-600';
  if (meeting.hasSummary) return 'bg-indigo-50 text-indigo-600';
  if (meeting.summaryStatus === 'failed' || meeting.summaryStatus === 'error') return 'bg-red-50 text-red-600';
  return 'bg-slate-100 text-slate-500';
}

interface MeetingGroup {
  label: string;
  meetings: MeetingListItem[];
}

function startOfDay(date: Date): number {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
}

/** Newest-first meetings bucketed into Today / Yesterday / Last 7 days / month. */
function groupByDate(meetings: MeetingListItem[], t: TranslateFn, now = new Date()): MeetingGroup[] {
  const today = startOfDay(now);
  const day = 24 * 60 * 60 * 1000;
  const monthFormat = new Intl.DateTimeFormat(getIntlLocale(), { month: 'long', year: 'numeric' });
  const groups: MeetingGroup[] = [];

  for (const meeting of meetings) {
    const created = new Date(meeting.createdAt);
    const createdDay = startOfDay(created);
    let label: string;
    if (createdDay >= today) label = t('Today');
    else if (createdDay >= today - day) label = t('Yesterday');
    else if (createdDay >= today - 6 * day) label = t('Last 7 days');
    else {
      const month = monthFormat.format(created);
      label = month.charAt(0).toUpperCase() + month.slice(1);
    }

    const last = groups[groups.length - 1];
    if (last?.label === label) last.meetings.push(meeting);
    else groups.push({ label, meetings: [meeting] });
  }
  return groups;
}

function formatDuration(seconds: number | null, t: TranslateFn): string {
  if (seconds === null || !Number.isFinite(seconds)) return '—';
  const totalMinutes = Math.round(seconds / 60);
  if (totalMinutes < 1) return t('Less than a minute');
  const hours = Math.floor(totalMinutes / 60);
  const minutes = totalMinutes % 60;
  return hours > 0 ? t('{hours} h {minutes} min', { hours, minutes }) : t('{minutes} min', { minutes });
}

function StatusChip({ meeting, t }: { meeting: MeetingListItem; t: TranslateFn }) {
  const status = meeting.summaryStatus;
  if (status === 'pending' || status === 'processing') {
    return (
      <span className="inline-flex items-center gap-1.5 rounded-full bg-amber-50 px-2.5 py-0.5 text-xs font-medium text-amber-700">
        <Loader2 className="h-3 w-3 animate-spin" />
        {t('Processing')}
      </span>
    );
  }
  if (meeting.hasSummary) {
    return (
      <span className="inline-flex items-center gap-1.5 rounded-full bg-emerald-50 px-2.5 py-0.5 text-xs font-medium text-emerald-700">
        <span className="h-1.5 w-1.5 rounded-full bg-current" />
        {t('Summary')}
      </span>
    );
  }
  if (status === 'failed' || status === 'error') {
    return (
      <span className="inline-flex items-center rounded-full bg-red-50 px-2.5 py-0.5 text-xs font-medium text-red-700">
        {t('Error')}
      </span>
    );
  }
  return (
    <span className="inline-flex items-center rounded-full bg-slate-100 px-2.5 py-0.5 text-xs font-medium text-slate-600">
      {t('Transcript')}
    </span>
  );
}

export default function MeetingsLibraryPage() {
  const router = useRouter();
  const { t } = useI18n();
  const {
    meetings,
    setMeetings,
    refetchMeetings,
    currentMeeting,
    setCurrentMeeting,
    handleRecordingToggle,
    searchTranscripts,
    searchResults,
    isSearching,
  } = useSidebar();
  const { isRecording } = useRecordingState();
  const { openImportDialog } = useImportDialog();
  const { betaFeatures } = useConfig();

  const [query, setQuery] = useState('');
  const [filter, setFilter] = useState<SummaryFilter>('all');
  const [pendingDeleteId, setPendingDeleteId] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<{ id: string; title: string } | null>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);

  // Summaries finish in the background, so statuses are refreshed on every visit.
  useEffect(() => {
    void refetchMeetings();
  }, [refetchMeetings]);

  useEffect(() => {
    const handle = setTimeout(() => void searchTranscripts(query), SEARCH_DEBOUNCE_MS);
    return () => clearTimeout(handle);
    // searchTranscripts is recreated on every provider render; the query is the trigger.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [query]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k') {
        event.preventDefault();
        searchInputRef.current?.focus();
        searchInputRef.current?.select();
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, []);

  const trimmedQuery = query.trim();
  const matchContexts = useMemo(
    () => new Map(trimmedQuery ? searchResults.map((result) => [result.id, result.matchContext]) : []),
    [searchResults, trimmedQuery],
  );

  const visibleMeetings = useMemo(() => {
    const needle = trimmedQuery.toLowerCase();
    return meetings.filter((meeting) => {
      if (filter === 'with' && !meeting.hasSummary) return false;
      if (filter === 'without' && meeting.hasSummary) return false;
      return !needle || meeting.title.toLowerCase().includes(needle) || matchContexts.has(meeting.id);
    });
  }, [meetings, filter, trimmedQuery, matchContexts]);

  const groups = useMemo(() => groupByDate(visibleMeetings, t), [visibleMeetings, t]);
  const timeFormat = useMemo(() => new Intl.DateTimeFormat(getIntlLocale(), { hour: '2-digit', minute: '2-digit' }), []);
  const dateFormat = useMemo(() => new Intl.DateTimeFormat(getIntlLocale(), { day: 'numeric', month: 'short', weekday: 'short' }), []);

  const openMeeting = useCallback((meeting: MeetingListItem) => {
    setCurrentMeeting({ id: meeting.id, title: meeting.title });
    // A transcript match opens the meeting searched for the same text.
    const search = matchContexts.has(meeting.id) ? `&q=${encodeURIComponent(trimmedQuery)}` : '';
    router.push(`/meeting-details?id=${meeting.id}${search}`);
  }, [router, setCurrentMeeting, matchContexts, trimmedQuery]);

  const handleDelete = async (meetingId: string) => {
    try {
      await invoke('api_delete_meeting', { meetingId });
      setMeetings((current) => current.filter((m) => m.id !== meetingId));
      if (currentMeeting?.id === meetingId) {
        setCurrentMeeting({ id: 'intro-call', title: '+ New Call' });
      }
      toast.success(t('Meeting deleted successfully'), {
        description: t('All associated data has been removed'),
      });
    } catch (error) {
      console.error('Failed to delete meeting:', error);
      toast.error(t('Failed to delete meeting'), {
        description: error instanceof Error ? error.message : String(error),
      });
    }
  };

  const handleRenameConfirm = async () => {
    if (!renaming) return;
    const title = renaming.title.trim();
    if (!title) {
      toast.error(t('Meeting title cannot be empty'));
      return;
    }
    try {
      await invoke('api_save_meeting_title', { meetingId: renaming.id, title });
      setMeetings((current) => current.map((m) => (m.id === renaming.id ? { ...m, title } : m)));
      if (currentMeeting?.id === renaming.id) {
        setCurrentMeeting({ id: renaming.id, title });
      }
      toast.success(t('Meeting title updated successfully'));
      setRenaming(null);
    } catch (error) {
      console.error('Failed to update meeting title:', error);
      toast.error(t('Failed to update meeting title'), {
        description: error instanceof Error ? error.message : String(error),
      });
    }
  };

  return (
    <div className="-ml-8 flex h-full flex-col bg-slate-50 py-7 pl-8 pr-8">
      <header className="flex flex-wrap items-center gap-3">
        <h1 className="text-2xl font-bold tracking-tight text-slate-900">{t('Meetings')}</h1>
        <span className="rounded-full bg-slate-200/70 px-2.5 py-0.5 text-sm font-semibold tabular-nums text-slate-600">{meetings.length}</span>
        <div className="ml-auto flex items-center gap-2">
          {betaFeatures.importAndRetranscribe && (
            <button
              type="button"
              onClick={() => openImportDialog()}
              className="inline-flex items-center gap-2 rounded-lg border border-slate-200 bg-white px-3.5 py-2 text-sm font-semibold text-slate-700 shadow-sm transition-colors hover:bg-slate-50"
            >
              <Upload className="h-4 w-4" />
              {t('Import Audio')}
            </button>
          )}
          <button
            type="button"
            onClick={isRecording ? () => router.push('/') : handleRecordingToggle}
            className="inline-flex items-center gap-2 rounded-lg bg-red-500 px-4 py-2 text-sm font-semibold text-white shadow-sm transition-colors hover:bg-red-600 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-red-300"
          >
            <Mic className="h-4 w-4" />
            {isRecording ? t('Recording in progress...') : t('New recording')}
          </button>
        </div>
      </header>

      <div className="mt-5 flex flex-wrap items-center gap-3">
        <div className="flex h-10 w-full max-w-md items-center gap-2 rounded-lg border border-slate-200 bg-white px-3 shadow-sm focus-within:border-indigo-300 focus-within:ring-2 focus-within:ring-indigo-100">
          {isSearching ? <Loader2 className="h-4 w-4 animate-spin text-slate-400" /> : <Search className="h-4 w-4 text-slate-400" />}
          <input
            ref={searchInputRef}
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => event.key === 'Escape' && setQuery('')}
            placeholder={t('Search titles and transcripts')}
            className="min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-slate-400"
          />
          {query ? (
            <button type="button" onClick={() => setQuery('')} aria-label={t('Clear')} className="text-slate-400 hover:text-slate-600">
              <X className="h-4 w-4" />
            </button>
          ) : (
            <kbd className="rounded border border-slate-200 bg-slate-50 px-1.5 py-0.5 text-[11px] text-slate-500">Ctrl K</kbd>
          )}
        </div>
        <div className="flex gap-1 rounded-lg bg-slate-200/70 p-1" role="tablist">
          {FILTERS.map(({ value, label }) => (
            <button
              key={value}
              type="button"
              role="tab"
              aria-selected={filter === value}
              onClick={() => setFilter(value)}
              className={`rounded-md px-3 py-1 text-sm font-semibold transition-colors ${
                filter === value ? 'bg-white text-slate-900 shadow-sm' : 'text-slate-600 hover:text-slate-900'
              }`}
            >
              {t(label)}
            </button>
          ))}
        </div>
      </div>

      <div className="mt-4 flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl border border-slate-200 bg-white shadow-sm">
        <div className={`${ROW_GRID} h-10 shrink-0 border-b border-slate-200 bg-slate-100/80 text-[11px] font-bold uppercase tracking-wider text-slate-500`}>
          <div>{t('Title')}</div>
          <div>{t('Date')}</div>
          <div>{t('Duration')}</div>
          <div>{t('Speakers')}</div>
          <div>{t('Status')}</div>
          <div />
        </div>

        <div className="custom-scrollbar min-h-0 flex-1 overflow-y-auto">
          {groups.length === 0 ? (
            <div className="flex h-full flex-col items-center justify-center gap-2 p-10 text-center">
              <div className="mb-1 grid h-14 w-14 place-items-center rounded-2xl bg-slate-100 text-slate-400">
                <AudioLines className="h-7 w-7" />
              </div>
              <p className="font-semibold text-slate-700">
                {meetings.length === 0 ? t('No meetings yet') : t('Nothing found')}
              </p>
              <p className="text-sm text-slate-500">
                {meetings.length === 0
                  ? t('Record a meeting or import audio, and it will appear here.')
                  : t('Try a different query or filter.')}
              </p>
            </div>
          ) : (
            groups.map((group) => (
              <section key={group.label}>
                <h2 className="sticky top-0 z-[1] flex items-center gap-2 border-b border-slate-100 bg-slate-50/95 px-5 py-2 text-xs font-semibold text-slate-600 backdrop-blur">
                  {group.label}
                  <span className="rounded-full bg-slate-200/70 px-1.5 text-[11px] font-medium tabular-nums text-slate-500">
                    {group.meetings.length}
                  </span>
                </h2>
                {group.meetings.map((meeting) => {
                  const created = new Date(meeting.createdAt);
                  const match = matchContexts.get(meeting.id);
                  const isActive = currentMeeting?.id === meeting.id;
                  return (
                    <div
                      key={meeting.id}
                      role="button"
                      tabIndex={0}
                      onClick={() => openMeeting(meeting)}
                      onKeyDown={(event) => {
                        if (event.key === 'Enter' || event.key === ' ') {
                          event.preventDefault();
                          openMeeting(meeting);
                        }
                      }}
                      className={`${ROW_GRID} group relative h-[68px] cursor-pointer border-b border-slate-100 outline-none transition-colors duration-150 last:border-b-0 hover:bg-slate-50 focus-visible:bg-indigo-50/60 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-indigo-200 ${
                        isActive ? 'bg-indigo-50/60 before:absolute before:inset-y-2 before:left-0 before:w-1 before:rounded-r before:bg-indigo-500' : ''
                      }`}
                    >
                      <div className="flex min-w-0 items-center gap-3">
                        <div className={`grid h-9 w-9 shrink-0 place-items-center rounded-lg ${rowIconClass(meeting)}`}>
                          <AudioLines className="h-4 w-4" />
                        </div>
                        <div className="min-w-0">
                          <div className="truncate text-sm font-semibold text-slate-900" title={meeting.title}>
                            {meeting.title}
                          </div>
                          <div className="mt-0.5 truncate text-xs text-slate-500" title={match ?? meeting.preview ?? undefined}>
                            {match ? (
                              <>
                                <span className="font-medium text-amber-600">{t('Match:')}</span> {match}
                              </>
                            ) : (
                              meeting.preview ?? ' '
                            )}
                          </div>
                        </div>
                      </div>
                      <div className="text-sm tabular-nums text-slate-800">
                        {timeFormat.format(created)}
                        <div className="text-xs text-slate-500">{dateFormat.format(created)}</div>
                      </div>
                      <div className="text-sm tabular-nums text-slate-700">{formatDuration(meeting.durationSeconds, t)}</div>
                      <div className="text-sm tabular-nums text-slate-700">
                        {meeting.speakerCount > 0 ? (
                          <span className="inline-flex items-center gap-1.5" title={t('{count} speakers', { count: meeting.speakerCount })}>
                            <UsersRound className="h-4 w-4 text-slate-400" />
                            {meeting.speakerCount}
                          </span>
                        ) : (
                          <span className="text-slate-400">—</span>
                        )}
                      </div>
                      <div>
                        <StatusChip meeting={meeting} t={t} />
                      </div>
                      <div className="flex justify-end gap-1 opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100">
                        <button
                          type="button"
                          aria-label={t('Rename')}
                          title={t('Rename')}
                          onClick={(event) => {
                            event.stopPropagation();
                            setRenaming({ id: meeting.id, title: meeting.title });
                          }}
                          className="grid h-8 w-8 place-items-center rounded-md border border-slate-200 bg-white text-slate-500 shadow-sm transition-colors hover:border-indigo-200 hover:text-indigo-600"
                        >
                          <Pencil className="h-3.5 w-3.5" />
                        </button>
                        <button
                          type="button"
                          aria-label={t('Delete')}
                          title={t('Delete')}
                          onClick={(event) => {
                            event.stopPropagation();
                            setPendingDeleteId(meeting.id);
                          }}
                          className="grid h-8 w-8 place-items-center rounded-md border border-slate-200 bg-white text-slate-500 shadow-sm transition-colors hover:border-red-200 hover:text-red-600"
                        >
                          <Trash2 className="h-3.5 w-3.5" />
                        </button>
                      </div>
                    </div>
                  );
                })}
              </section>
            ))
          )}
        </div>
      </div>

      <ConfirmationModal
        isOpen={pendingDeleteId !== null}
        text={t('Are you sure you want to delete this meeting? This action cannot be undone.')}
        onConfirm={() => {
          if (pendingDeleteId) void handleDelete(pendingDeleteId);
          setPendingDeleteId(null);
        }}
        onCancel={() => setPendingDeleteId(null)}
      />

      <Dialog open={renaming !== null} onOpenChange={(open) => !open && setRenaming(null)}>
        <DialogContent className="sm:max-w-[425px]">
          <VisuallyHidden>
            <DialogTitle>{t('Edit Meeting Title')}</DialogTitle>
          </VisuallyHidden>
          <div className="py-4">
            <h3 className="mb-4 text-lg font-semibold">{t('Edit Meeting Title')}</h3>
            <label htmlFor="meeting-title" className="mb-2 block text-sm font-medium text-slate-700">
              {t('Meeting Title')}
            </label>
            <input
              id="meeting-title"
              type="text"
              value={renaming?.title ?? ''}
              onChange={(event) => setRenaming((current) => current && { ...current, title: event.target.value })}
              onKeyDown={(event) => event.key === 'Enter' && void handleRenameConfirm()}
              className="w-full rounded-md border border-slate-300 px-3 py-2 focus:border-transparent focus:outline-none focus:ring-2 focus:ring-indigo-500"
              placeholder={t('Enter meeting title')}
              autoFocus
            />
          </div>
          <DialogFooter>
            <button
              type="button"
              onClick={() => setRenaming(null)}
              className="rounded-md bg-slate-100 px-4 py-2 text-sm font-medium text-slate-700 transition-colors hover:bg-slate-200"
            >
              {t('Cancel')}
            </button>
            <button
              type="button"
              onClick={() => void handleRenameConfirm()}
              className="rounded-md bg-indigo-600 px-4 py-2 text-sm font-medium text-white transition-colors hover:bg-indigo-700"
            >
              {t('Save')}
            </button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
