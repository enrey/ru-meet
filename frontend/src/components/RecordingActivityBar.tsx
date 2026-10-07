'use client';

import { useRef, useState } from 'react';
import { Loader2, Pause, Play, Square } from 'lucide-react';
import { toast } from 'sonner';
import { RecordingStatus, useRecordingState } from '@/contexts/RecordingStateContext';
import { useAutoRecording } from '@/hooks/useAutoRecording';
import { recordingService } from '@/services/recordingService';
import { useI18n } from '@/lib/i18n';
import { Button } from '@/components/ui/button';
import { Switch } from '@/components/ui/switch';

export function RecordingActivityBar() {
  const state = useRecordingState();
  const { preferences, saving, error, setEnabled } = useAutoRecording();
  const [actionPending, setActionPending] = useState(false);
  const actionRef = useRef(false);
  const { t } = useI18n();
  const finishing = state.isStopping || state.isProcessing || state.isSaving;
  const recording = state.isRecording && !finishing;
  const busy = finishing || state.isStartingRecording;
  const enabled = preferences?.autoRecordSupported && preferences.autoRecordMeetings;
  const seconds = Math.max(0, Math.floor(state.activeDuration ?? 0));
  const duration = `${Math.floor(seconds / 60).toString().padStart(2, '0')}:${(seconds % 60).toString().padStart(2, '0')}`;

  const stop = async () => {
    if (actionRef.current || busy) return;
    actionRef.current = true;
    setActionPending(true);
    state.setStatus(RecordingStatus.STOPPING);
    try {
      const result = await recordingService.stopRecording();
      // The global provider refreshes the saved meeting from recording-stopped.
      if (!result) state.setStatus(RecordingStatus.IDLE);
    } catch (error) {
      state.setStatus(RecordingStatus.ERROR, String(error));
      toast.error(t('Failed to stop recording'), { description: String(error) });
    } finally {
      actionRef.current = false;
      setActionPending(false);
    }
  };

  const togglePause = async () => {
    if (actionRef.current || busy) return;
    actionRef.current = true;
    setActionPending(true);
    try {
      if (state.isPaused) await recordingService.resumeRecording();
      else await recordingService.pauseRecording();
    } catch (error) {
      toast.error(t('Could not change recording state'), { description: String(error) });
    } finally {
      actionRef.current = false;
      setActionPending(false);
    }
  };

  const label = busy
    ? (state.isStartingRecording ? t('Starting recording...') : state.isSaving ? t('Saving transcript...') : t('Processing recording...'))
    : recording ? (state.isPaused ? t('Recording paused') : t('Recording in progress'))
    : t('Automatic recording');
  const autoDescription = error ? t('Could not load automatic recording status')
    : !preferences ? t('Loading...')
    : !preferences.autoRecordSupported ? t('Automatic recording is unavailable on this platform')
    : enabled ? t('Waiting for sound') : t('Off');

  return (
    <div className="shrink-0 px-4 pt-4 pb-2">
      <div className={`flex min-h-12 flex-wrap items-center justify-between gap-x-4 gap-y-2 rounded-2xl border px-4 py-2 ${
        recording ? (state.isPaused ? 'border-amber-100 bg-amber-50 text-amber-700' : 'border-red-100 bg-red-50 text-red-600')
        : busy ? 'border-blue-100 bg-blue-50 text-blue-700'
        : enabled ? 'border-blue-100 bg-blue-50/70 text-blue-700' : 'border-gray-200 bg-gray-50 text-gray-600'
      }`}>
        <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 text-sm">
          {busy ? <Loader2 aria-hidden className="h-4 w-4 motion-safe:animate-spin" />
            : recording ? <span aria-hidden className={`h-3 w-3 shrink-0 rounded-full ${state.isPaused ? 'bg-amber-500' : 'bg-red-500 motion-safe:animate-pulse'}`} />
            : <svg aria-hidden viewBox="0 0 32 32" className={`h-7 w-7 shrink-0 ${enabled ? 'text-blue-600' : 'text-gray-400'}`} fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round">
                <circle cx="16" cy="16" r="2.5" fill="currentColor" stroke="none" />
                <g className={enabled ? 'auto-record-echo auto-record-echo-inner' : ''}><path d="M10 11a7 7 0 0 0 0 10M22 11a7 7 0 0 1 0 10" /></g>
                <g className={enabled ? 'auto-record-echo auto-record-echo-outer' : ''}><path d="M6 7a13 13 0 0 0 0 18M26 7a13 13 0 0 1 0 18" /></g>
              </svg>}
          <span className="font-semibold" role="status">{label}</span>
          {recording && <><span aria-hidden className="text-gray-400">·</span><span className="tabular-nums text-gray-600">{duration}</span></>}
          {!recording && !busy && <span className="text-xs text-gray-500">{autoDescription}</span>}
        </div>
        {recording ? <div className="flex shrink-0 items-center gap-2">
          <Button variant="ghost" size="sm" onClick={togglePause} disabled={actionPending} aria-label={state.isPaused ? t('Resume recording') : t('Pause recording')} title={state.isPaused ? t('Resume recording') : t('Pause recording')}>
            {state.isPaused ? <Play className="h-4 w-4" /> : <Pause className="h-4 w-4" />}
          </Button>
          <Button variant="outline" size="sm" onClick={stop} disabled={actionPending} className="rounded-full border-red-200 bg-transparent text-red-600 hover:bg-red-100 hover:text-red-700">
            <Square aria-hidden className="h-3 w-3 fill-current" /> {t('Stop')}
          </Button>
        </div> : !busy && <label className="flex min-h-9 shrink-0 cursor-pointer items-center gap-3 text-sm">
          <span>{preferences ? (enabled ? t('On') : t('Off')) : '—'}</span>
          {saving && <Loader2 aria-hidden className="h-3 w-3 motion-safe:animate-spin" />}
          <Switch checked={!!enabled} disabled={!preferences?.autoRecordSupported || saving} onCheckedChange={setEnabled} aria-label={t('Automatic recording')} className="data-[state=checked]:bg-blue-600" />
        </label>}
      </div>
    </div>
  );
}
