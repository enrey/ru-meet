'use client';

import { useRef, useState } from 'react';
import { Loader2, Pause, Play, Square } from 'lucide-react';
import { toast } from 'sonner';
import { RecordingStatus, useRecordingState } from '@/contexts/RecordingStateContext';
import { useAutoRecording } from '@/hooks/useAutoRecording';
import { useRecordingSourceMutes } from '@/hooks/useRecordingSourceMutes';
import { useTranscriptionModelLoading } from '@/hooks/useTranscriptionModelLoading';
import { recordingService } from '@/services/recordingService';
import { useI18n } from '@/lib/i18n';
import { Button } from '@/components/ui/button';
import { Switch } from '@/components/ui/switch';

export function RecordingActivityBar() {
  const state = useRecordingState();
  const { preferences, saving, error, setEnabled } = useAutoRecording();
  const sourceMutes = useRecordingSourceMutes();
  const [actionPending, setActionPending] = useState(false);
  const actionRef = useRef(false);
  const { t } = useI18n();
  const finishing = state.isStopping || state.isProcessing || state.isSaving;
  const recording = state.isRecording && !finishing;
  const busy = finishing || state.isStartingRecording;
  const modelLoading = useTranscriptionModelLoading(recording);
  const enabled = preferences?.autoRecordSupported && preferences.autoRecordMeetings;
  const noSources = sourceMutes?.microphone === true && sourceMutes.system === true;
  const listening = enabled && !noSources;
  const autoLabel = !sourceMutes ? t('Automatic recording')
    : noSources ? t('Automatic recording (disabled, no sources)')
    : !sourceMutes.microphone && !sourceMutes.system ? t('Automatic recording (microphone + system)')
    : !sourceMutes.microphone ? t('Automatic recording (microphone)')
    : t('Automatic recording (system)');
  const recordingLabel = !sourceMutes ? t('Recording in progress')
    : noSources ? t('Recording in progress (no sources)')
    : !sourceMutes.microphone && !sourceMutes.system ? t('Recording in progress (microphone + system)')
    : !sourceMutes.microphone ? t('Recording in progress (microphone)')
    : t('Recording in progress (system)');
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
    : recording ? (state.isPaused ? t('Recording paused') : recordingLabel)
    : autoLabel;
  const autoDescription = error ? t('Could not load automatic recording status')
    : !preferences ? t('Loading...')
    : !preferences.autoRecordSupported ? t('Automatic recording is unavailable on this platform')
    : listening ? (preferences.pausedAfterManualStop ? t('Paused after manual stop (the sound detector restarts only after 5 seconds of silence)') : t('Waiting for sound'))
    : t('Off');

  const tone = recording ? (state.isPaused ? 'bg-amber-50 text-amber-700' : 'bg-red-50 text-red-600')
    : busy ? 'bg-indigo-50 text-indigo-700'
    : noSources ? 'bg-red-50 text-red-600'
    : enabled ? 'bg-indigo-50/80 text-indigo-600' : 'bg-slate-100 text-slate-600';

  return (
    <header className="shrink-0 border-b border-slate-200 bg-white px-8 py-2">
      <div className={`flex min-h-10 flex-wrap items-center justify-between gap-x-4 gap-y-1 rounded-lg py-1 pl-2.5 pr-3 ${tone}`}>
      <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 text-sm">
        <span className="inline-flex min-w-0 items-center gap-1.5 font-semibold">
          {busy ? <Loader2 aria-hidden className="h-4 w-4 shrink-0 motion-safe:animate-spin" />
            : recording ? <span aria-hidden className={`mx-0.5 h-2.5 w-2.5 shrink-0 rounded-full ${state.isPaused ? 'bg-amber-500' : 'bg-red-500 motion-safe:animate-pulse'}`} />
            : <svg aria-hidden viewBox="0 0 32 32" className={`h-5 w-5 shrink-0 ${noSources ? 'text-red-500' : listening ? 'text-indigo-600' : 'text-slate-400'}`} fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round">
                <circle cx="16" cy="16" r="2.5" fill="currentColor" stroke="none" />
                <g className={listening ? 'auto-record-echo auto-record-echo-inner' : ''}><path d="M10 11a7 7 0 0 0 0 10M22 11a7 7 0 0 1 0 10" /></g>
                <g className={listening ? 'auto-record-echo auto-record-echo-outer' : ''}><path d="M6 7a13 13 0 0 0 0 18M26 7a13 13 0 0 1 0 18" /></g>
              </svg>}
          <span className="truncate" role="status">{label}</span>
          {recording && <span className="font-medium tabular-nums opacity-80">{duration}</span>}
        </span>
        {modelLoading && (
          <span className="inline-flex items-center gap-1.5 text-xs text-slate-500" role="status">
            <Loader2 aria-hidden className="h-3 w-3 motion-safe:animate-spin" />
            {t('Loading speech recognition model… audio is already being recorded')}
          </span>
        )}
        {!recording && !busy && <span className="translate-y-[1.5px] text-xs text-slate-500">{autoDescription}</span>}
      </div>
      {recording ? <div className="flex shrink-0 items-center gap-2">
        <Button variant="ghost" size="sm" onClick={togglePause} disabled={actionPending} aria-label={state.isPaused ? t('Resume recording') : t('Pause recording')} title={state.isPaused ? t('Resume recording') : t('Pause recording')} className="h-7 w-7 p-0 hover:bg-white/60">
          {state.isPaused ? <Play className="h-4 w-4" /> : <Pause className="h-4 w-4" />}
        </Button>
        <Button variant="outline" size="sm" onClick={stop} disabled={actionPending} className="h-7 rounded-md border-red-200 bg-white text-red-600 hover:bg-red-100 hover:text-red-700">
          <Square aria-hidden className="h-3 w-3 fill-current" /> {t('Stop')}
        </Button>
      </div> : !busy && <label className="flex shrink-0 cursor-pointer items-center gap-3">
        <span className="text-xs font-medium text-slate-500">{preferences ? (enabled ? t('On') : t('Off')) : '—'}</span>
        {saving && <Loader2 aria-hidden className="h-3 w-3 text-slate-400 motion-safe:animate-spin" />}
        <Switch checked={!!enabled} disabled={!preferences?.autoRecordSupported || saving} onCheckedChange={setEnabled} aria-label={t('Automatic recording')} />
      </label>}
      </div>
    </header>
  );
}
