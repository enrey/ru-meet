'use client';

import { useState } from 'react';
import Link from 'next/link';
import { invoke } from '@tauri-apps/api/core';
import { AlertCircle, AudioLines, CheckCircle2, Clock3, FileText, ListTodo, Loader2, Pause, RotateCcw, ScanSearch, Upload, UsersRound, Volume2, XCircle, type LucideIcon } from 'lucide-react';
import { toast } from 'sonner';
import { TASK_KIND_LABELS, useTasks, type BackgroundTask, type TaskStatus } from '@/contexts/TasksContext';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { Button } from '@/components/ui/button';
import { getIntlLocale, useI18n, type TranslateFn } from '@/lib/i18n';

const KINDS: Record<BackgroundTask['kind'], { label: string; icon: LucideIcon }> = {
  tts: { label: TASK_KIND_LABELS.tts, icon: Volume2 },
  diarization: { label: TASK_KIND_LABELS.diarization, icon: UsersRound },
  summary: { label: TASK_KIND_LABELS.summary, icon: FileText },
  import: { label: TASK_KIND_LABELS.import, icon: Upload },
  retranscription: { label: TASK_KIND_LABELS.retranscription, icon: AudioLines },
  'voice-match': { label: TASK_KIND_LABELS['voice-match'], icon: ScanSearch },
};
const STATUSES: Record<TaskStatus, { label: string; icon: LucideIcon; style: string }> = {
  scheduled: { label: 'Scheduled retry', icon: Clock3, style: 'bg-sky-50 text-sky-700' },
  queued: { label: 'Queued', icon: Clock3, style: 'bg-slate-100 text-slate-600' },
  running: { label: 'Running', icon: Loader2, style: 'bg-indigo-50 text-indigo-700' },
  paused: { label: 'Paused for recording', icon: Pause, style: 'bg-amber-50 text-amber-800' },
  failed: { label: 'Failed', icon: AlertCircle, style: 'bg-red-50 text-red-700' },
  done: { label: 'Done', icon: CheckCircle2, style: 'bg-emerald-50 text-emerald-700' },
  cancelled: { label: 'Cancelled', icon: XCircle, style: 'bg-slate-100 text-slate-600' },
};
const pending = (task: BackgroundTask) => !task.finishedAt;
const active = (task: BackgroundTask) => pending(task) && task.status !== 'scheduled';

/** How long finished tasks took in this session, on average by kind. */
function averageDurations(tasks: BackgroundTask[]) {
  const sums = new Map<string, { total: number; count: number }>();
  let total = 0;
  let count = 0;
  for (const task of tasks) {
    if (task.status !== 'done' || !task.startedAt || !task.finishedAt) continue;
    const seconds = (Date.parse(task.finishedAt) - Date.parse(task.startedAt)) / 1000;
    if (!Number.isFinite(seconds) || seconds < 0) continue;
    const sum = sums.get(task.kind) ?? { total: 0, count: 0 };
    sums.set(task.kind, { total: sum.total + seconds, count: sum.count + 1 });
    total += seconds;
    count += 1;
  }
  return (kind: string) => {
    const sum = sums.get(kind);
    if (sum) return sum.total / sum.count;
    return count ? total / count : null;
  };
}

/**
 * Rough time left: the average duration of each waiting task's kind, minus
 * what the running one has spent already. `null` until something finished.
 */
function estimateSeconds(tasks: BackgroundTask[], now: number): number | null {
  const averageOf = averageDurations(tasks);
  let seconds = 0;
  for (const task of tasks.filter(active)) {
    const average = averageOf(task.kind);
    if (average === null) return null;
    const spent = task.status !== 'queued' && task.startedAt ? (now - Date.parse(task.startedAt)) / 1000 : 0;
    // A task running longer than usual still needs a moment.
    seconds += Math.max(average - spent, Math.min(average, 5));
  }
  return seconds;
}

function formatLeft(seconds: number, t: TranslateFn): string {
  const minutes = Math.round(seconds / 60);
  if (minutes < 1) return t('Less than a minute');
  const hours = Math.floor(minutes / 60);
  return hours ? t('{hours} h {minutes} min', { hours, minutes: minutes % 60 }) : t('{minutes} min', { minutes });
}

export default function TasksPage() {
  const { t, locale } = useI18n();
  const { tasks, recordingPaused, loading, error, refresh } = useTasks();
  const { meetings } = useSidebar();
  const [retrying, setRetrying] = useState<number | null>(null);
  const queued = tasks.filter(pending);
  const history = tasks.filter(task => !pending(task));
  const scheduledOnly = queued.length > 0 && queued.every(task => task.status === 'scheduled');
  const failures = history.filter(task => task.status === 'failed').length;
  const now = Date.now();
  const remaining = tasks.filter(active);
  const inWork = remaining.filter(task => task.status === 'running' || task.status === 'paused').length;
  const left = remaining.length ? estimateSeconds(tasks, now) : null;
  const progress = remaining.length > 0 && (
    <p className="mt-1 text-sm font-medium">
      {t('Tasks left: {count}', { count: remaining.length })}
      {inWork > 0 && ` · ${t('running: {count}', { count: inWork })}`}
      {left !== null && !recordingPaused && ` · ${t('about {time} left, until ~{clock}', {
        time: formatLeft(left, t),
        clock: new Date(now + left * 1000).toLocaleTimeString(getIntlLocale(locale), { hour: '2-digit', minute: '2-digit' }),
      })}`}
    </p>
  );
  const meetingTitles = new Map(meetings.map(meeting => [meeting.id, meeting.title]));
  const retry = async (task: BackgroundTask) => {
    if (retrying !== null) return;
    setRetrying(task.id);
    try {
      await invoke('retry_task', { taskId: task.id });
      await refresh();
      toast.success(t('Task added to the end of the queue'));
    } catch (reason) {
      toast.error(t('Could not retry task'), { description: String(reason) });
    } finally { setRetrying(null); }
  };
  const row = (task: BackgroundTask, index: number) => {
    const kind = KINDS[task.kind];
    const status = STATUSES[task.status];
    const Icon = kind.icon;
    const StatusIcon = status.icon;
    const title = task.title ?? (task.meetingId ? meetingTitles.get(task.meetingId) ?? t('Meeting') : t('Audio file'));
    const time = new Date(task.finishedAt ?? task.createdAt);
    return (
      <li key={task.id} className={`flex flex-wrap items-center gap-x-3 gap-y-1.5 px-4 py-2 ${task.status === 'running' ? 'bg-indigo-50/40' : ''}`}>
        <span className="w-6 shrink-0 text-center text-xs tabular-nums text-slate-400" aria-hidden>{pending(task) ? index + 1 : '\u2014'}</span>
        <span className={`grid h-7 w-7 shrink-0 place-items-center rounded-lg ${task.status === 'running' ? 'bg-indigo-100 text-indigo-700' : 'bg-slate-100 text-slate-500'}`}><Icon className="h-3.5 w-3.5" aria-hidden /></span>
        <div className="flex min-w-0 flex-1 basis-60 items-baseline gap-2">
          <span className="shrink-0 text-sm font-medium text-slate-900">{t(kind.label)}</span>
          {task.meetingId ? <Link href={`/meeting-details?id=${encodeURIComponent(task.meetingId)}`} title={title} className="min-w-0 truncate text-sm text-slate-500 underline-offset-4 hover:text-indigo-700 hover:underline focus-visible:outline focus-visible:outline-2 focus-visible:outline-indigo-500">{title}</Link> : <span title={title} className="min-w-0 truncate text-sm text-slate-500">{title}</span>}
        </div>
        <time dateTime={time.toISOString()} className="shrink-0 text-xs tabular-nums text-slate-400">{time.toLocaleString(getIntlLocale(locale), { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' })}</time>
        <span className={`inline-flex shrink-0 items-center gap-1 rounded-full px-2 py-0.5 text-xs font-medium ${status.style}`}><StatusIcon aria-hidden className={`h-3 w-3 ${task.status === 'running' ? 'motion-safe:animate-spin' : ''}`} />{t(status.label)}</span>
        {(task.status === 'failed' || task.status === 'scheduled') && <Button variant="outline" size="sm" className="h-7 px-2.5 text-xs" disabled={!task.canRetry || retrying !== null} onClick={() => void retry(task)}><RotateCcw aria-hidden className={`mr-1.5 h-3.5 w-3.5 ${retrying === task.id ? 'motion-safe:animate-spin' : ''}`} />{task.status === 'scheduled' ? t('Run now') : t('Retry')}</Button>}
        {(task.scheduledAt || task.error) && (
          <div className="basis-full pl-[4.75rem]">
            {task.scheduledAt && <p className="text-xs text-sky-800">{t('Automatic retry after {time}, when recording and other tasks finish.', { time: new Date(task.scheduledAt).toLocaleTimeString(getIntlLocale(locale), { hour: '2-digit', minute: '2-digit' }) })}</p>}
            {task.error && <p className="mt-1 whitespace-pre-wrap break-words rounded-md border border-red-200 bg-red-50 px-2.5 py-1.5 text-xs text-red-800">{task.error}</p>}
          </div>
        )}
      </li>
    );
  };
  return (
    <div className="h-full overflow-y-auto pr-8 pb-12">
      <div className="mx-auto max-w-5xl py-8">
        <header className="mb-7 flex flex-wrap items-start justify-between gap-4">
          <div><h1 className="text-3xl font-semibold tracking-tight text-slate-900">{t('Tasks')}</h1><p className="mt-2 max-w-2xl text-sm leading-6 text-slate-500">{t('Tasks run from top to bottom, one at a time. Recording pauses background work.')}</p></div>
          <span className="rounded-full border border-slate-200 bg-white px-3 py-1.5 text-sm text-slate-600">{t('Concurrency: 1')}</span>
        </header>
        <div role="status" className={`mb-6 flex items-start gap-3 rounded-xl border p-4 ${recordingPaused ? 'border-amber-200 bg-amber-50 text-amber-900' : 'border-slate-200 bg-white text-slate-700'}`}>
          {recordingPaused ? <Pause aria-hidden className="mt-0.5 h-5 w-5 shrink-0" /> : <ListTodo aria-hidden className="mt-0.5 h-5 w-5 shrink-0" />}
          <div><p className="text-sm font-semibold">{recordingPaused ? t('Queue paused during recording') : scheduledOnly ? t('Waiting for delayed retries') : queued.length ? t('Queue is processing') : t('No pending tasks')}</p>{progress}<p className="mt-1 text-sm opacity-80">{recordingPaused ? t('Tasks will resume automatically when recording finishes.') : t('Saved failures retry after 10 minutes from startup, one at a time. A fresh failure waits for manual retry.')}</p></div>
        </div>
        {error && <div role="alert" className="mb-6 rounded-xl border border-red-200 bg-red-50 p-4 text-sm text-red-800"><p>{t('Could not load tasks')}</p><p className="mt-1 break-words">{error}</p><Button variant="outline" className="mt-3" onClick={() => void refresh()}>{t('Refresh')}</Button></div>}
        <section aria-labelledby="queue-heading"><div className="mb-3 flex items-center gap-2"><h2 id="queue-heading" className="text-sm font-semibold text-slate-900">{t('Queue')}</h2><span className="text-sm tabular-nums text-slate-500">{queued.length}</span></div><ol className="overflow-hidden rounded-2xl border border-slate-200 bg-white divide-y divide-slate-100">{queued.map(row)}{queued.length === 0 && <li className="flex flex-col items-center px-6 py-12 text-center"><ListTodo aria-hidden className="mb-3 h-8 w-8 text-slate-400" /><p className="font-medium text-slate-700">{loading ? t('Loading tasks...') : t('Queue is empty')}</p><p className="mt-2 max-w-sm text-sm leading-6 text-slate-500">{t('Speech, speaker, summary and transcription jobs will appear here when requested.')}</p></li>}</ol></section>
        {history.length > 0 && <section aria-labelledby="history-heading" className="mt-8"><div className="mb-3 flex flex-wrap items-center gap-3"><h2 id="history-heading" className="text-sm font-semibold text-slate-900">{t('Recent tasks')}</h2>{failures > 0 && <span className="text-sm text-red-700">{t('Needs attention: {count}', { count: failures })}</span>}</div><ul className="overflow-hidden rounded-2xl border border-slate-200 bg-white divide-y divide-slate-100">{history.map(row)}</ul></section>}
        <p className="mt-5 text-xs text-slate-500">{t('Saved failures and recent results from this app session.')}</p>
      </div>
    </div>
  );
}
