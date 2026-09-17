import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Loader2 } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { OnboardingContainer } from '../OnboardingContainer';
import { useOnboarding } from '@/contexts/OnboardingContext';
import { DEFAULT_GIGAAM_MODEL, DEFAULT_PARAKEET_MODEL } from '@/constants/modelDefaults';
import { getSummaryModelSizeLabel } from '@/lib/onboarding-summary-model';

type Download = { status: 'waiting' | 'downloading' | 'completed' | 'error'; progress: number; error?: string };
const waiting: Download = { status: 'waiting', progress: 0 };

export function DownloadProgressStep() {
  const {
    goNext, goPrevious, transcriptionProvider, downloadTranscription, downloadSummary,
    downloadDiarization, diarizationEngine,
    selectedSummaryModel, recommendedSummaryModel, summaryModelDownloaded,
    startBackgroundDownloads, completeOnboarding,
  } = useOnboarding();
  const [transcription, setTranscription] = useState<Download>(waiting);
  const [summary, setSummary] = useState<Download>(waiting);
  const [diarization, setDiarization] = useState<Download>(waiting);
  const [isCompleting, setIsCompleting] = useState(false);
  const [isMac, setIsMac] = useState(false);
  const started = useRef(false);
  const summaryStarted = useRef(false);
  const diarizationStarted = useRef(false);
  const model = transcriptionProvider === 'gigaam' ? DEFAULT_GIGAAM_MODEL : DEFAULT_PARAKEET_MODEL;
  const label = transcriptionProvider === 'gigaam' ? 'GigaAM v3' : 'Parakeet TDT v3';
  const initCommand = transcriptionProvider === 'gigaam' ? 'gigaam_init' : 'parakeet_init';
  const readyCommand = transcriptionProvider === 'gigaam' ? 'gigaam_has_available_models' : 'parakeet_has_available_models';
  const eventPrefix = transcriptionProvider === 'gigaam' ? 'gigaam' : 'parakeet';

  useEffect(() => {
    import('@tauri-apps/plugin-os')
      .then(({ platform }) => setIsMac(typeof platform === 'function' ? platform() === 'macos' : navigator.userAgent.includes('Mac')))
      .catch(() => setIsMac(navigator.userAgent.includes('Mac')));
  }, []);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    Promise.all([
      listen<{ modelName: string; progress: number }>(`${eventPrefix}-model-download-progress`, ({ payload }) => {
        if (!disposed && payload.modelName === model) setTranscription({ status: 'downloading', progress: payload.progress });
      }),
      listen<{ modelName: string }>(`${eventPrefix}-model-download-complete`, ({ payload }) => {
        if (!disposed && payload.modelName === model) setTranscription({ status: 'completed', progress: 100 });
      }),
      listen<{ modelName: string; error: string }>(`${eventPrefix}-model-download-error`, ({ payload }) => {
        if (!disposed && payload.modelName === model) setTranscription({ status: 'error', progress: 0, error: payload.error });
      }),
    ]).then((items) => {
      if (disposed) items.forEach((unlisten) => unlisten());
      else unlisteners.push(...items);
    });

    const prepare = async () => {
      try {
        await invoke(initCommand);
        const ready = await invoke<boolean>(readyCommand);
        if (disposed) return;
        if (ready) {
          setTranscription({ status: 'completed', progress: 100 });
        } else if (downloadTranscription && !started.current) {
          started.current = true;
          setTranscription({ status: 'downloading', progress: 0 });
          await startBackgroundDownloads({ includeParakeet: transcriptionProvider === 'parakeet', includeGigaam: transcriptionProvider === 'gigaam', includeSummary: false });
        }
      } catch (error) {
        if (!disposed) setTranscription({ status: 'error', progress: 0, error: String(error) });
      }
    };
    void prepare();
    return () => { disposed = true; unlisteners.forEach((unlisten) => unlisten()); };
  }, [transcriptionProvider, downloadTranscription]);

  useEffect(() => {
    if (!downloadSummary || !selectedSummaryModel || summaryStarted.current) return;
    summaryStarted.current = true;
    if (summaryModelDownloaded) {
      setSummary({ status: 'completed', progress: 100 });
      return;
    }
    setSummary({ status: 'downloading', progress: 0 });
    void startBackgroundDownloads({ includeParakeet: false, includeSummary: true, summaryModel: selectedSummaryModel });
  }, [downloadSummary, selectedSummaryModel, summaryModelDownloaded]);

  useEffect(() => {
    const unlisten = listen<{ model: string; progress: number; status: string; error?: string }>('builtin-ai-download-progress', ({ payload }) => {
      if (payload.model !== selectedSummaryModel) return;
      setSummary({ status: payload.status === 'error' ? 'error' : payload.status === 'completed' ? 'completed' : 'downloading', progress: payload.progress, error: payload.error });
    });
    return () => { void unlisten.then((fn) => fn()); };
  }, [selectedSummaryModel]);

  useEffect(() => {
    if (!downloadDiarization) return;
    let disposed = false;
    const unlisten = listen<{ engine: string; progress: number }>('diarization-download-progress', ({ payload }) => {
      if (!disposed && payload.engine === diarizationEngine) {
        setDiarization({ status: payload.progress === 100 ? 'completed' : 'downloading', progress: payload.progress });
      }
    });
    if (!diarizationStarted.current) {
      diarizationStarted.current = true;
      setDiarization({ status: 'downloading', progress: 0 });
      void invoke('download_diarization_models', { engine: diarizationEngine })
        .then(() => { if (!disposed) setDiarization({ status: 'completed', progress: 100 }); })
        .catch((error) => { if (!disposed) setDiarization({ status: 'error', progress: 0, error: String(error) }); });
    }
    return () => { disposed = true; void unlisten.then((fn) => fn()); };
  }, [downloadDiarization, diarizationEngine]);

  const retryTranscription = async () => {
    setTranscription({ status: 'downloading', progress: 0 });
    try {
      if (transcriptionProvider === 'parakeet') await invoke('parakeet_retry_download', { modelName: model });
      else await invoke('gigaam_download_model', { modelName: model });
    } catch (error) {
      setTranscription({ status: 'error', progress: 0, error: String(error) });
    }
  };

  const retrySummary = async () => {
    if (!selectedSummaryModel) return;
    setSummary({ status: 'downloading', progress: 0 });
    try { await invoke('builtin_ai_download_model', { modelName: selectedSummaryModel }); }
    catch (error) { setSummary({ status: 'error', progress: 0, error: String(error) }); }
  };

  const retryDiarization = async () => {
    setDiarization({ status: 'downloading', progress: 0 });
    try {
      await invoke('download_diarization_models', { engine: diarizationEngine });
      setDiarization({ status: 'completed', progress: 100 });
    } catch (error) {
      setDiarization({ status: 'error', progress: 0, error: String(error) });
    }
  };

  const continueSetup = async () => {
    if (downloadTranscription && transcription.status !== 'completed') {
      toast.info('The transcription model is still downloading. You can continue and add it later.');
    }
    if (isMac) { goNext(); return; }
    setIsCompleting(true);
    try { await completeOnboarding(); window.location.reload(); }
    catch (error) { toast.error('Failed to complete setup', { description: String(error) }); setIsCompleting(false); }
  };

  const card = (title: string, detail: string, state: Download, retry: () => void) => (
    <div className="rounded-xl border border-gray-200 bg-white p-5">
      <div className="flex items-center justify-between gap-3"><div><h3 className="font-medium text-gray-900">{title}</h3><p className="text-sm text-gray-500">{detail}</p></div><span className="text-sm capitalize text-gray-600">{state.status}</span></div>
      {(state.status === 'downloading' || state.status === 'completed') && <div className="mt-4 h-2 overflow-hidden rounded-full bg-gray-200"><div className="h-full bg-gray-900" style={{ width: `${state.progress}%` }} /></div>}
      {state.status === 'error' && <div className="mt-3 text-sm text-red-600"><p>{state.error}</p><Button className="mt-2" variant="outline" onClick={retry}>Try again</Button></div>}
    </div>
  );

  return (
    <OnboardingContainer title="Getting things ready" description="Downloads may continue in the background. You can add models later in Settings." step={3} totalSteps={isMac ? 4 : 3}>
      <div className="mx-auto w-full max-w-lg space-y-4">
        {downloadTranscription && card(label, transcriptionProvider === 'gigaam' ? '~186 MB' : '~670 MB', transcription, () => { void retryTranscription(); })}
        {downloadSummary && card('Local summarization model', getSummaryModelSizeLabel(selectedSummaryModel || recommendedSummaryModel), summary, () => { void retrySummary(); })}
        {downloadDiarization && card('Speaker diarization', diarizationEngine === 'nvidia-sortformer-v2' ? 'NVIDIA Sortformer v2' : 'PyAnnote + WeSpeaker', diarization, () => { void retryDiarization(); })}
        {!downloadTranscription && !downloadSummary && !downloadDiarization && <p className="rounded-lg bg-gray-100 p-4 text-sm text-gray-700">No models selected for download. Recording will need a transcription model later.</p>}
        <div className="flex gap-3"><Button variant="outline" onClick={goPrevious}>Back</Button><Button className="flex-1 bg-gray-900 text-white hover:bg-gray-800" disabled={isCompleting} onClick={() => { void continueSetup(); }}>{isCompleting && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}Continue</Button></div>
      </div>
    </OnboardingContainer>
  );
}
