'use client';

import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useRouter } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { toast } from 'sonner';
import { AudioLines, FileText, Loader2, Mic, Pencil, ScanSearch, Search, Sparkles, Star, Trash2, Upload, UserRound, UsersRound, Volume2, WandSparkles, X, type LucideIcon } from 'lucide-react';
import { useSidebar, type MeetingListItem, type MeetingStages, type StageState } from '@/components/Sidebar/SidebarProvider';
import { ConfirmationModal } from '@/components/ConfirmationModel/confirmation-modal';
import { Dialog, DialogContent, DialogFooter, DialogTitle } from '@/components/ui/dialog';
import { VisuallyHidden } from '@/components/ui/visually-hidden';
import { useImportDialog } from '@/contexts/ImportDialogContext';
import { useConfig } from '@/contexts/ConfigContext';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { TASK_KIND_LABELS, useTasks, type BackgroundTask } from '@/contexts/TasksContext';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { voiceprints } from '@/services/voiceprintService';
import { getIntlLocale, useI18n, type TranslateFn } from '@/lib/i18n';

type SummaryFilter = 'all' | 'with' | 'without' | 'favorites';

const FILTERS: { value: SummaryFilter; label: string }[] = [
  { value: 'all', label: 'All' },
  { value: 'with', label: 'With summary' },
  { value: 'without', label: 'Without summary' },
  { value: 'favorites', label: 'Favorites' },
];

const SEARCH_DEBOUNCE_MS = 250;
const RUNNING_REFRESH_MS = 3000;
const ROW_GRID = 'grid grid-cols-[minmax(0,1fr)_130px_110px_170px_120px_144px] items-center gap-4 px-5';
/** Lines the speakers cell fits in a row. */
const SPEAKER_LINES = 3;

const sameName = (a: string, b: string) => a.localeCompare(b, undefined, { sensitivity: 'base' }) === 0;
/** Confirmed names, then the voice library's suggestions; filters match both. */
const allSpeakerNames = (meeting: MeetingListItem) => [
  ...(meeting.speakerNames ?? []),
  ...(meeting.suggestedSpeakerNames ?? []),
];

/** Row icon tint follows the summary state, so the list also scans by colour. */
function rowIconClass(meeting: MeetingListItem): string {
  if (meeting.summaryStatus === 'pending' || meeting.summaryStatus === 'processing') return 'bg-amber-50 text-amber-600';
  if (meeting.hasSummary) return 'bg-indigo-50 text-indigo-600';
  if (meeting.summaryStatus === 'failed' || meeting.summaryStatus === 'error') return 'bg-red-50 text-red-600';
  return 'bg-slate-100 text-slate-500';
}

/** What is being done to a meeting now: a line per task or step. */
interface MeetingActivity {
  /** Something runs; otherwise everything is only waiting its turn. */
  working: boolean;
  lines: { label: string; state: 'running' | 'paused' | 'queued' }[];
}

const ACTIVITY_STATE: Record<MeetingActivity['lines'][number]['state'], { label: string; dot: string }> = {
  running: { label: 'Running', dot: 'bg-emerald-400 motion-safe:animate-pulse' },
  paused: { label: 'Paused for recording', dot: 'bg-amber-400' },
  queued: { label: 'Queued', dot: 'bg-slate-400' },
};

function meetingActivity(meeting: MeetingListItem, tasks: BackgroundTask[], t: TranslateFn): MeetingActivity | null {
  const lines: MeetingActivity['lines'] = tasks
    .filter((task) => task.meetingId === meeting.id && !task.finishedAt)
    .flatMap((task) => {
      const state = task.status === 'running' ? 'running' as const
        : task.status === 'paused' ? 'paused' as const
        : task.status === 'queued' ? 'queued' as const
        : null;
      return state ? [{ label: t(TASK_KIND_LABELS[task.kind]), state }] : [];
    });
  // Steps the backend reports as running, e.g. transcription after recording,
  // when no queued task stands for them already.
  for (const { key, label } of STAGES) {
    if (meeting.stages?.[key]?.state !== 'running') continue;
    const text = t(label.running);
    if (!lines.some((line) => line.state === 'running')) lines.push({ label: text, state: 'running' });
  }
  if (!lines.length && meeting.processing) {
    lines.push({ label: t('The recording is being processed'), state: 'running' });
  }
  if (!lines.length) return null;
  return { working: lines.some((line) => line.state !== 'queued'), lines };
}

/** Sparks fly off the wand: direction, colour and delay of each. */
const SPARKS = [
  { dx: '-15px', dy: '-14px', color: '#f59e0b', delay: '0s' },
  { dx: '16px', dy: '-12px', color: '#d946ef', delay: '0.35s' },
  { dx: '15px', dy: '14px', color: '#38bdf8', delay: '0.7s' },
  { dx: '-14px', dy: '13px', color: '#a78bfa', delay: '1.05s' },
  { dx: '1px', dy: '-18px', color: '#fbbf24', delay: '1.4s' },
];

/**
 * The row icon while the meeting is being worked on: a wand throwing sparks,
 * or a resting wand when its tasks only wait in the queue. Hover tells what.
 */
function ActivityIcon({ activity, t }: { activity: MeetingActivity; t: TranslateFn }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <div
          aria-label={activity.lines.map((line) => line.label).join(', ')}
          className={`relative grid h-9 w-9 shrink-0 place-items-center rounded-lg ${
            activity.working ? 'magic-glow text-white' : 'bg-violet-50 text-violet-400'
          }`}
        >
          <WandSparkles className={`relative z-[1] h-4 w-4 ${activity.working ? 'magic-wand' : ''}`} />
          {activity.working && SPARKS.map((spark) => (
            <span
              key={spark.delay}
              aria-hidden
              className="magic-spark"
              style={{ '--dx': spark.dx, '--dy': spark.dy, color: spark.color, animationDelay: spark.delay } as React.CSSProperties}
            />
          ))}
        </div>
      </TooltipTrigger>
      <TooltipContent side="right" className="max-w-xs bg-slate-900 px-3 py-2 text-white">
        <p className="mb-1 font-semibold">{activity.working ? t('Working on this meeting') : t('Waiting in the task queue')}</p>
        <ul className="space-y-0.5">
          {activity.lines.map((line, index) => (
            <li key={index} className="flex items-center gap-2">
              <span className={`h-1.5 w-1.5 shrink-0 rounded-full ${ACTIVITY_STATE[line.state].dot}`} />
              <span className="min-w-0 flex-1">{line.label}</span>
              <span className="shrink-0 text-slate-400">{t(ACTIVITY_STATE[line.state].label)}</span>
            </li>
          ))}
        </ul>
      </TooltipContent>
    </Tooltip>
  );
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

function isProcessing(meeting: MeetingListItem): boolean {
  return Boolean(meeting.processing)
    || Object.values(meeting.stages ?? {}).some((stage) => stage.state === 'running');
}

function formatDuration(seconds: number | null, t: TranslateFn): string {
  if (seconds === null || !Number.isFinite(seconds)) return '—';
  const totalMinutes = Math.round(seconds / 60);
  if (totalMinutes < 1) return t('Less than a minute');
  const hours = Math.floor(totalMinutes / 60);
  const minutes = totalMinutes % 60;
  return hours > 0 ? t('{hours} h {minutes} min', { hours, minutes }) : t('{minutes} min', { minutes });
}

type StageKey = keyof MeetingStages;

const STAGES: { key: StageKey; icon: LucideIcon; label: Record<Exclude<StageState, 'none'>, string> }[] = [
  {
    key: 'transcript',
    icon: FileText,
    label: { done: 'Transcript ready', running: 'Transcribing…', failed: 'Transcription failed' },
  },
  {
    key: 'speakers',
    icon: UsersRound,
    label: { done: 'Speakers identified', running: 'Identifying speakers…', failed: 'Speaker identification failed' },
  },
  {
    key: 'summary',
    icon: Sparkles,
    label: { done: 'Summary ready', running: 'Generating summary…', failed: 'Summary failed' },
  },
  {
    key: 'speech',
    icon: Volume2,
    label: { done: 'Summary read aloud', running: 'Reading the summary aloud…', failed: 'Reading aloud failed' },
  },
];

const STAGE_TONE: Record<Exclude<StageState, 'none'>, string> = {
  done: 'text-slate-500',
  running: 'text-amber-500 animate-pulse',
  failed: 'text-red-500',
};

/**
 * The meeting's processing steps as a row of small icons: only the ones done,
 * under way or failed, in pipeline order, with details on hover. A transcript
 * is the norm, so its icon appears only while it is missing or not done.
 */
function StageIcons({ meeting, t }: { meeting: MeetingListItem; t: TranslateFn }) {
  const shown = STAGES.filter(({ key }) => {
    const state = meeting.stages?.[key]?.state ?? 'none';
    if (key === 'transcript') return state !== 'done';
    return state !== 'none';
  });

  if (!shown.length) return <span className="text-sm text-slate-400">—</span>;
  return (
    <div className="flex items-center gap-2">
      {shown.map(({ key, icon: Icon, label }) => {
        const stage = meeting.stages[key];
        const state = stage.state;
        const title = state === 'none'
          ? t('No transcript')
          : stage.error ? `${t(label[state])}: ${stage.error}` : t(label[state]);
        return (
          <span key={key} title={title} aria-label={title} className="relative inline-flex">
            <Icon className={`h-4 w-4 ${state === 'none' ? 'text-slate-300' : STAGE_TONE[state]}`} />
            {state === 'failed' && (
              <span className="absolute -right-0.5 -top-0.5 h-1.5 w-1.5 rounded-full bg-red-500 ring-1 ring-white" />
            )}
          </span>
        );
      })}
    </div>
  );
}

/**
 * Pinned speaker filters as chips plus a text field. Typing narrows the list
 * by a part of a name and offers known names; Enter or a click pins one, as
 * does a click on a name in the table. Every pinned speaker must take part.
 */
function SpeakerFilter({
  pinned,
  onPin,
  onUnpin,
  onClear,
  text,
  onTextChange,
  knownNames,
  t,
}: {
  pinned: string[];
  onPin: (name: string) => void;
  onUnpin: (name: string) => void;
  onClear: () => void;
  text: string;
  onTextChange: (text: string) => void;
  knownNames: string[];
  t: TranslateFn;
}) {
  const [open, setOpen] = useState(false);
  const [highlight, setHighlight] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const needle = text.trim().toLowerCase();
  const options = useMemo(
    () => knownNames
      .filter((name) => !pinned.some((pin) => sameName(pin, name)) && (!needle || name.toLowerCase().includes(needle)))
      .slice(0, 8),
    [knownNames, pinned, needle],
  );
  const active = Math.min(highlight, Math.max(options.length - 1, 0));

  const pin = (name: string) => {
    onPin(name);
    onTextChange('');
    setHighlight(0);
  };

  return (
    <div className="relative w-full max-w-xs">
      <div
        onClick={() => inputRef.current?.focus()}
        className="flex min-h-10 w-full cursor-text flex-wrap items-center gap-1.5 rounded-lg border border-slate-200 bg-white px-3 py-1.5 shadow-sm focus-within:border-indigo-300 focus-within:ring-2 focus-within:ring-indigo-100"
      >
        <UserRound className="h-4 w-4 shrink-0 text-slate-400" aria-hidden />
        {pinned.map((name) => (
          <span key={name} className="inline-flex max-w-[11rem] items-center gap-1 rounded-md bg-indigo-50 py-0.5 pl-2 pr-1 text-xs font-medium text-indigo-800 ring-1 ring-inset ring-indigo-200">
            <span className="truncate" title={name}>{name}</span>
            <button
              type="button"
              onClick={(event) => { event.stopPropagation(); onUnpin(name); }}
              aria-label={t('Remove the filter by “{name}”', { name })}
              title={t('Remove the filter by “{name}”', { name })}
              className="grid h-4 w-4 shrink-0 place-items-center rounded text-indigo-500 hover:bg-indigo-100 hover:text-indigo-800"
            >
              <X className="h-3 w-3" />
            </button>
          </span>
        ))}
        <input
          ref={inputRef}
          value={text}
          role="combobox"
          aria-expanded={open && options.length > 0}
          aria-label={t('Filter by speaker')}
          onChange={(event) => { onTextChange(event.target.value); setHighlight(0); setOpen(true); }}
          onFocus={() => setOpen(true)}
          onBlur={() => setOpen(false)}
          onKeyDown={(event) => {
            if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
              event.preventDefault();
              setOpen(true);
              if (options.length) setHighlight((active + (event.key === 'ArrowDown' ? 1 : options.length - 1)) % options.length);
            } else if (event.key === 'Enter') {
              event.preventDefault();
              const choice = open ? options[active] : undefined;
              if (choice) pin(choice);
              else if (text.trim()) pin(text.trim());
            } else if (event.key === 'Escape') {
              if (text) onTextChange('');
              else setOpen(false);
            } else if (event.key === 'Backspace' && !text && pinned.length) {
              onUnpin(pinned[pinned.length - 1]);
            }
          }}
          placeholder={pinned.length ? '' : t('Speakers')}
          className="min-w-[4rem] flex-1 bg-transparent text-sm outline-none placeholder:text-slate-400"
        />
        {(text || pinned.length > 0) && (
          <button
            type="button"
            onClick={(event) => { event.stopPropagation(); onClear(); }}
            aria-label={t('Clear')}
            title={t('Clear')}
            className="text-slate-400 hover:text-slate-600"
          >
            <X className="h-4 w-4" />
          </button>
        )}
      </div>
      {open && options.length > 0 && (
        <ul role="listbox" className="absolute left-0 right-0 top-full z-20 mt-1 max-h-64 overflow-y-auto rounded-lg border border-slate-200 bg-white py-1 shadow-lg">
          {options.map((name, index) => (
            <li
              key={name}
              role="option"
              aria-selected={index === active}
              // Keeps the input focused, so its blur does not close the list first.
              onMouseDown={(event) => { event.preventDefault(); pin(name); }}
              onMouseEnter={() => setHighlight(index)}
              className={`flex cursor-pointer items-center gap-2 px-3 py-1.5 text-sm ${index === active ? 'bg-indigo-50 text-indigo-800' : 'text-slate-700'}`}
            >
              <UserRound className="h-3.5 w-3.5 shrink-0 text-slate-400" aria-hidden />
              <span className="truncate">{name}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/**
 * Named speakers, one per line, each a button that pins it as a filter; the
 * rest, unnamed or not fitting, as a "+N" line. Without names: the count.
 */
function SpeakersCell({
  meeting,
  pinned,
  onPin,
  t,
}: {
  meeting: MeetingListItem;
  pinned: string[];
  onPin: (name: string) => void;
  t: TranslateFn;
}) {
  const confirmed = meeting.speakerNames ?? [];
  // Two speakers of one meeting can both sound like the same person.
  const suggested = (meeting.suggestedSpeakerNames ?? [])
    .filter((name, index, all) => !confirmed.some((known) => sameName(known, name))
      && all.findIndex((other) => sameName(other, name)) === index);
  const names = [...confirmed, ...suggested];
  if (!names.length) {
    return meeting.speakerCount > 0 ? (
      <span className="inline-flex items-center gap-1.5 text-sm tabular-nums text-slate-700" title={t('{count} speakers', { count: meeting.speakerCount })}>
        <UsersRound className="h-4 w-4 text-slate-400" />
        {meeting.speakerCount}
      </span>
    ) : (
      <span className="text-sm text-slate-400">—</span>
    );
  }
  const unnamed = Math.max(meeting.speakerCount - names.length, 0);
  const fits = names.length + (unnamed > 0 ? 1 : 0) <= SPEAKER_LINES;
  const shown = fits ? names : names.slice(0, SPEAKER_LINES - 1);
  const rest = names.length - shown.length + unnamed;
  const restTitle = [
    ...names.slice(shown.length),
    ...(unnamed > 0 ? [t('Unnamed speakers: {count}', { count: unnamed })] : []),
  ].join('\n');
  return (
    <div className="flex min-w-0 flex-col items-start">
      {shown.map((name, index) => {
        const isPinned = pinned.some((pin) => sameName(pin, name));
        const isSuggested = index >= confirmed.length;
        // A pinned name shows whether the meeting really has this person:
        // confirmed in green, only suggested by the voice library in pale blue.
        const tone = isPinned
          ? isSuggested ? 'font-medium text-indigo-400' : 'font-semibold text-emerald-700'
          : isSuggested ? 'text-slate-400' : 'text-slate-700';
        return (
          <button
            key={name}
            type="button"
            onClick={(event) => { event.stopPropagation(); onPin(name); }}
            onKeyDown={(event) => event.stopPropagation()}
            title={isSuggested
              ? t('Probably {name}: open the meeting to confirm', { name })
              : isPinned ? name : t('Show only meetings with {name}', { name })}
            className={`-mx-1 max-w-[calc(100%+0.5rem)] truncate rounded px-1 text-left text-xs leading-[18px] transition-colors hover:bg-indigo-50 hover:text-indigo-700 ${tone}`}
          >
            {name}{isSuggested && '?'}
          </button>
        );
      })}
      {rest > 0 && (
        <span className="inline-flex items-center gap-1 text-xs leading-[18px] text-slate-400" title={restTitle}>
          <UsersRound className="h-3 w-3" aria-hidden />
          +{rest}
        </span>
      )}
    </div>
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
  const { tasks, refresh: refreshTasks } = useTasks();
  const matchingVoices = tasks.some((task) => task.kind === 'voice-match' && !task.finishedAt);
  const [startingMatch, setStartingMatch] = useState(false);

  const [query, setQuery] = useState('');
  const [filter, setFilter] = useState<SummaryFilter>('all');
  const [pinnedSpeakers, setPinnedSpeakers] = useState<string[]>([]);
  const [speakerText, setSpeakerText] = useState('');
  const [pendingDeleteId, setPendingDeleteId] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<{ id: string; title: string } | null>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);

  // Summaries finish in the background, so statuses are refreshed on every visit.
  useEffect(() => {
    void refetchMeetings();
  }, [refetchMeetings]);

  // ...and while any step is under way, until it is not.
  const anyRunning = meetings.some(isProcessing);
  useEffect(() => {
    if (!anyRunning) return;
    const timer = setInterval(() => void refetchMeetings(), RUNNING_REFRESH_MS);
    return () => clearInterval(timer);
  }, [anyRunning, refetchMeetings]);

  // Each meeting's matching task reports when it is through; the list shows
  // the new suggestions as they come, at most once a second.
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | null = null;
    const schedule = () => {
      if (timer) return;
      timer = setTimeout(() => { timer = null; void refetchMeetings(); }, 1000);
    };
    const stops = [
      listen('voice-suggestions-updated', schedule),
      listen('transcript-speakers-updated', schedule),
    ];
    return () => {
      if (timer) clearTimeout(timer);
      stops.forEach((stop) => void stop.then((unlisten) => unlisten()));
    };
  }, [refetchMeetings]);

  const identifySpeakers = async () => {
    if (startingMatch || matchingVoices) return;
    setStartingMatch(true);
    try {
      const queued = await voiceprints.matchAll();
      await refreshTasks();
      if (queued > 0) {
        toast(t('Meetings queued for speaker identification: {count}', { count: queued }), {
          description: t('Suggestions appear in the Speakers column in grey.'),
        });
      } else {
        toast(t('All meetings are already checked or queued'));
      }
    } catch (error) {
      toast.error(t('Could not identify speakers'), { description: t(String(error)) });
    } finally {
      setStartingMatch(false);
    }
  };

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

  // Known names, the most frequent first, for the speaker field's suggestions.
  const knownSpeakers = useMemo(() => {
    const counts = new Map<string, { name: string; count: number }>();
    for (const meeting of meetings) {
      for (const name of allSpeakerNames(meeting)) {
        const key = name.toLocaleLowerCase();
        const entry = counts.get(key);
        if (entry) entry.count += 1;
        else counts.set(key, { name, count: 1 });
      }
    }
    return [...counts.values()]
      .sort((a, b) => b.count - a.count || a.name.localeCompare(b.name))
      .map(({ name }) => name);
  }, [meetings]);

  const pinSpeaker = useCallback((name: string) => {
    setPinnedSpeakers((current) => (current.some((pin) => sameName(pin, name)) ? current : [...current, name]));
  }, []);
  const unpinSpeaker = useCallback((name: string) => {
    setPinnedSpeakers((current) => current.filter((pin) => pin !== name));
  }, []);
  const clearSpeakers = useCallback(() => {
    setPinnedSpeakers([]);
    setSpeakerText('');
  }, []);

  const visibleMeetings = useMemo(() => {
    const needle = trimmedQuery.toLowerCase();
    const speakerNeedle = speakerText.trim().toLowerCase();
    return meetings.filter((meeting) => {
      if (filter === 'with' && !meeting.hasSummary) return false;
      if (filter === 'without' && meeting.hasSummary) return false;
      if (filter === 'favorites' && !meeting.isFavorite) return false;
      const names = allSpeakerNames(meeting);
      if (!pinnedSpeakers.every((pin) => names.some((name) => sameName(name, pin)))) return false;
      if (speakerNeedle && !names.some((name) => name.toLowerCase().includes(speakerNeedle))) return false;
      return !needle || meeting.title.toLowerCase().includes(needle) || matchContexts.has(meeting.id);
    });
  }, [meetings, filter, trimmedQuery, matchContexts, pinnedSpeakers, speakerText]);

  const groups = useMemo(() => groupByDate(visibleMeetings, t), [visibleMeetings, t]);
  const timeFormat = useMemo(() => new Intl.DateTimeFormat(getIntlLocale(), { hour: '2-digit', minute: '2-digit' }), []);
  const dateFormat = useMemo(() => new Intl.DateTimeFormat(getIntlLocale(), { day: 'numeric', month: 'short', weekday: 'short' }), []);

  const toggleFavorite = useCallback(async (meeting: MeetingListItem) => {
    const favorite = !meeting.isFavorite;
    const apply = (value: boolean) => setMeetings((current) =>
      current.map((item) => (item.id === meeting.id ? { ...item, isFavorite: value } : item)));
    apply(favorite);
    try {
      await invoke('api_set_meeting_favorite', { meetingId: meeting.id, favorite });
    } catch (error) {
      apply(!favorite);
      toast.error(t('Could not update favorites'), { description: String(error) });
    }
  }, [setMeetings, t]);

  const openMeeting = useCallback((meeting: MeetingListItem) => {
    setCurrentMeeting({ id: meeting.id, title: meeting.title });
    // A transcript match opens the meeting searched for the same text.
    const search = matchContexts.has(meeting.id) ? `&q=${encodeURIComponent(trimmedQuery)}` : '';
    router.push(`/meeting-details?id=${meeting.id}${search}`);
  }, [router, setCurrentMeeting, matchContexts, trimmedQuery]);

  // Runs in the backend; the row shows its progress through `stages`.
  const processMeeting = async (meetingId: string) => {
    try {
      await invoke('process_meeting', { meetingId });
      void refetchMeetings();
    } catch (error) {
      toast.error(t('Could not process the recording'), { description: String(error) });
    }
  };

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
            onClick={() => void identifySpeakers()}
            disabled={startingMatch || matchingVoices}
            title={matchingVoices
              ? t('Speakers are being identified by the voice library')
              : t('Look for the voice library people among the speakers of all meetings')}
            className="inline-flex items-center gap-2 rounded-lg border border-slate-200 bg-white px-3.5 py-2 text-sm font-semibold text-slate-700 shadow-sm transition-colors hover:bg-slate-50 disabled:cursor-default disabled:opacity-70 disabled:hover:bg-white"
          >
            {startingMatch || matchingVoices
              ? <Loader2 className="h-4 w-4 motion-safe:animate-spin" />
              : <ScanSearch className="h-4 w-4" />}
            {t('Identify speakers')}
          </button>
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
        <SpeakerFilter
          pinned={pinnedSpeakers}
          onPin={pinSpeaker}
          onUnpin={unpinSpeaker}
          onClear={clearSpeakers}
          text={speakerText}
          onTextChange={setSpeakerText}
          knownNames={knownSpeakers}
          t={t}
        />
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
                  const activity = meetingActivity(meeting, tasks, t);
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
                        {activity ? (
                          <ActivityIcon activity={activity} t={t} />
                        ) : (
                          <div className={`grid h-9 w-9 shrink-0 place-items-center rounded-lg ${rowIconClass(meeting)}`}>
                            <AudioLines className="h-4 w-4" />
                          </div>
                        )}
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
                      <div className="min-w-0">
                        <SpeakersCell meeting={meeting} pinned={pinnedSpeakers} onPin={pinSpeaker} t={t} />
                      </div>
                      <div>
                        <StageIcons meeting={meeting} t={t} />
                      </div>
                      <div className="flex items-center justify-end gap-1">
                      <div className="flex justify-end gap-1 opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100">
                        {meeting.stages?.transcript?.state === 'done' && (
                          <button
                            type="button"
                            aria-label={t('Process recording')}
                            title={isProcessing(meeting) ? t('The recording is being processed') : t('Process recording: run the steps it is missing')}
                            disabled={isProcessing(meeting)}
                            onClick={(event) => {
                              event.stopPropagation();
                              void processMeeting(meeting.id);
                            }}
                            className="grid h-8 w-8 place-items-center rounded-md border border-slate-200 bg-white text-slate-500 shadow-sm transition-colors hover:border-indigo-200 hover:text-indigo-600 disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:border-slate-200 disabled:hover:text-slate-500"
                          >
                            <Sparkles className="h-3.5 w-3.5" />
                          </button>
                        )}
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
                        <button
                          type="button"
                          aria-pressed={meeting.isFavorite}
                          aria-label={meeting.isFavorite ? t('Remove from favorites') : t('Add to favorites')}
                          title={meeting.isFavorite ? t('Remove from favorites') : t('Add to favorites')}
                          // A mouse click must not leave focus in the row: the
                          // row's hover actions also show on focus-within.
                          onMouseDown={(event) => event.preventDefault()}
                          onClick={(event) => {
                            event.stopPropagation();
                            void toggleFavorite(meeting);
                          }}
                          className={`grid h-8 w-8 shrink-0 place-items-center rounded-md transition-colors hover:bg-amber-50 ${
                            meeting.isFavorite ? 'text-amber-500' : 'text-slate-300 hover:text-amber-500'
                          }`}
                        >
                          <Star className={`h-4 w-4 ${meeting.isFavorite ? 'fill-amber-400' : ''}`} />
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
