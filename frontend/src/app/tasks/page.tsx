'use client';

import { useState } from 'react';
import Link from 'next/link';
import { invoke } from '@tauri-apps/api/core';
import { AlertCircle, AudioLines, CheckCircle2, Clock3, FileText, ListTodo, Loader2, Pause, RotateCcw, Upload, UsersRound, Volume2, XCircle, type LucideIcon } from 'lucide-react';
import { toast } from 'sonner';
import { useTasks, type BackgroundTask, type TaskStatus } from '@/contexts/TasksContext';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { Button } from '@/components/ui/button';
import { getIntlLocale, useI18n } from '@/lib/i18n';

const KINDS: Record<BackgroundTask['kind'], { label: string; icon: LucideIcon }> = {
  tts: { label: 'Summary speech', icon: Volume2 },
  diarization: { label: 'Speaker diarization', icon: UsersRound },
  summary: { label: 'Summary generation', icon: FileText },
  import: { label: 'Audio import', icon: Upload },
  retranscription: { label: 'Retranscription', icon: AudioLines },
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

export default function TasksPage() {
  const { t, locale } = useI18n();
  const { tasks, recordingPaused, loading, error, refresh } = useTasks();
  const { meetings } = useSidebar();
  const [retrying, setRetrying] = useState<number | null>(null);
  const queued = tasks.filter(pending);
  const history = tasks.filter(task => !pending(task));
  const scheduledOnly = queued.length > 0 && queued.every(task => task.status === 'scheduled');
  const failures = history.filter(task => task.status === 'failed').length;
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
      <li key={task.id} className={`flex flex-wrap items-start gap-4 px-5 py-5 ${task.status === 'running' ? 'bg-indigo-50/40' : ''}`}>
        <span className="mt-2 w-6 shrink-0 text-center text-sm tabular-nums text-slate-500" aria-hidden>{pending(task) ? index + 1 : '\u2014'}</span>
        <span className={`grid h-11 w-11 shrink-0 place-items-center rounded-xl ${task.status === 'running' ? 'bg-indigo-100 text-indigo-700' : 'bg-slate-100 text-slate-500'}`}><Icon className="h-5 w-5" aria-hidden /></span>
        <div className="min-w-0 flex-1 basis-40">
          <p className="font-semibold text-slate-900">{t(kind.label)}</p>
          {task.meetingId ? <Link href={`/meeting-details?id=${encodeURIComponent(task.meetingId)}`} className="mt-0.5 inline-block max-w-full break-words text-sm text-slate-600 underline-offset-4 hover:text-indigo-700 hover:underline focus-visible:outline focus-visible:outline-2 focus-visible:outline-indigo-500">{title}</Link> : <p className="mt-0.5 break-words text-sm text-slate-600">{title}</p>}
          {task.scheduledAt && <p className="mt-2 text-sm text-sky-800">{t('Automatic retry after {time}, when recording and other tasks finish.', { time: new Date(task.scheduledAt).toLocaleTimeString(getIntlLocale(locale), { hour: '2-digit', minute: '2-digit' }) })}</p>}
          {task.error && <p className="mt-3 whitespace-pre-wrap break-words rounded-lg border border-red-200 bg-red-50 p-3 text-sm text-red-800">{task.error}</p>}
          <p className="mt-2 text-xs text-slate-500"><time dateTime={time.toISOString()}>{time.toLocaleString(getIntlLocale(locale), { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' })}</time></p>
        </div>
        <div className="flex shrink-0 flex-wrap items-center justify-end gap-3">
          <span className={`inline-flex items-center gap-1.5 rounded-full px-3 py-1.5 text-xs font-medium ${status.style}`}><StatusIcon aria-hidden className={`h-3.5 w-3.5 ${task.status === 'running' ? 'motion-safe:animate-spin' : ''}`} />{t(status.label)}</span>
          {(task.status === 'failed' || task.status === 'scheduled') && <Button variant="outline" size="sm" className="min-h-10" disabled={!task.canRetry || retrying !== null} onClick={() => void retry(task)}><RotateCcw aria-hidden className={`mr-2 h-4 w-4 ${retrying === task.id ? 'motion-safe:animate-spin' : ''}`} />{task.status === 'scheduled' ? t('Run now') : t('Retry')}</Button>}
        </div>
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
          <div><p className="text-sm font-semibold">{recordingPaused ? t('Queue paused during recording') : scheduledOnly ? t('Waiting for delayed retries') : queued.length ? t('Queue is processing') : t('No pending tasks')}</p><p className="mt-1 text-sm opacity-80">{recordingPaused ? t('Tasks will resume automatically when recording finishes.') : t('Saved failures retry after 10 minutes from startup, one at a time. A fresh failure waits for manual retry.')}</p></div>
        </div>
        {error && <div role="alert" className="mb-6 rounded-xl border border-red-200 bg-red-50 p-4 text-sm text-red-800"><p>{t('Could not load tasks')}</p><p className="mt-1 break-words">{error}</p><Button variant="outline" className="mt-3" onClick={() => void refresh()}>{t('Refresh')}</Button></div>}
        <section aria-labelledby="queue-heading"><div className="mb-3 flex items-center gap-2"><h2 id="queue-heading" className="text-sm font-semibold text-slate-900">{t('Queue')}</h2><span className="text-sm tabular-nums text-slate-500">{queued.length}</span></div><ol className="overflow-hidden rounded-2xl border border-slate-200 bg-white divide-y divide-slate-100">{queued.map(row)}{queued.length === 0 && <li className="flex flex-col items-center px-6 py-12 text-center"><ListTodo aria-hidden className="mb-3 h-8 w-8 text-slate-400" /><p className="font-medium text-slate-700">{loading ? t('Loading tasks...') : t('Queue is empty')}</p><p className="mt-2 max-w-sm text-sm leading-6 text-slate-500">{t('Speech, speaker, summary and transcription jobs will appear here when requested.')}</p></li>}</ol></section>
        {history.length > 0 && <section aria-labelledby="history-heading" className="mt-8"><div className="mb-3 flex flex-wrap items-center gap-3"><h2 id="history-heading" className="text-sm font-semibold text-slate-900">{t('Recent tasks')}</h2>{failures > 0 && <span className="text-sm text-red-700">{t('Needs attention: {count}', { count: failures })}</span>}</div><ul className="overflow-hidden rounded-2xl border border-slate-200 bg-white divide-y divide-slate-100">{history.map(row)}</ul></section>}
        <p className="mt-5 text-xs text-slate-500">{t('Saved failures and recent results from this app session.')}</p>
      </div>
    </div>
  );
}
