'use client';

import { createContext, useCallback, useContext, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

export type TaskStatus = 'queued' | 'scheduled' | 'running' | 'paused' | 'failed' | 'done' | 'cancelled';
export interface BackgroundTask {
  id: number;
  kind: 'tts' | 'diarization' | 'summary' | 'import' | 'retranscription';
  meetingId: string | null;
  title: string | null;
  status: TaskStatus;
  error: string | null;
  createdAt: string;
  finishedAt: string | null;
  canRetry: boolean;
  scheduledAt: string | null;
}
interface Snapshot {
  tasks: BackgroundTask[];
  recordingPaused: boolean;
}
interface TasksState extends Snapshot {
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
}
const TasksContext = createContext<TasksState | null>(null);

/** One shared poll for navigation and the queue, with no overlapping requests. */
export function TasksProvider({ children }: { children: React.ReactNode }) {
  const [snapshot, setSnapshot] = useState<Snapshot>({ tasks: [], recordingPaused: false });
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const mounted = useRef(false);
  const inFlight = useRef<Promise<void> | null>(null);
  const refresh = useCallback((): Promise<void> => {
    if (inFlight.current) return inFlight.current;
    const request = invoke<Snapshot>('get_tasks')
      .then(next => { if (mounted.current) { setSnapshot(next); setError(null); } })
      .catch(reason => { if (mounted.current) setError(String(reason)); })
      .finally(() => { inFlight.current = null; if (mounted.current) setLoading(false); });
    inFlight.current = request;
    return request;
  }, []);
  useEffect(() => {
    mounted.current = true;
    void refresh();
    const timer = setInterval(() => { if (!document.hidden) void refresh(); }, 1500);
    const onFocus = () => void refresh();
    window.addEventListener('focus', onFocus);
    return () => { mounted.current = false; clearInterval(timer); window.removeEventListener('focus', onFocus); };
  }, [refresh]);
  return <TasksContext.Provider value={{ ...snapshot, loading, error, refresh }}>{children}</TasksContext.Provider>;
}

export function useTasks() {
  const context = useContext(TasksContext);
  if (!context) throw new Error('useTasks requires TasksProvider');
  return context;
}
