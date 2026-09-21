import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Cpu, Download, Loader2, PlayCircle, Trash2, Zap } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from './ui/button';
import type { RawModelInfo } from '@/hooks/useTranscriptionModels';

const MODEL = 'gigaam-v3-e2e-ctc';

interface Props {
  selectedModel?: string;
  onModelSelect?: (modelName: string) => void;
  autoSave?: boolean;
}

interface ActiveProviderStatus {
  label: string | null;
  fallback_reason: string | null;
  provider_assignments: Array<{ provider: string; node_count: number }>;
  is_live: boolean;
}

export function GigaAMModelManager({ selectedModel, onModelSelect, autoSave = false }: Props) {
  const [model, setModel] = useState<RawModelInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [progress, setProgress] = useState<number | null>(null);
  // Reflects which execution provider GigaAM actually ran on - not just
  // which model is downloaded/selected. `label` stays null until something
  // has loaded the model at least once this session (transcription, import);
  // a still-empty status right after picking GigaAM isn't a sign anything is
  // wrong. It also stays populated (with `is_live: false`) after the model
  // unloads - which happens right after every batch import finishes - so
  // this doesn't go blank the moment you check.
  const [providerStatus, setProviderStatus] = useState<ActiveProviderStatus | null>(null);
  const [testing, setTesting] = useState(false);

  const fetchProviderStatus = async () => {
    try {
      const status = await invoke<ActiveProviderStatus>('gigaam_get_active_provider');
      setProviderStatus(status);
    } catch {
      // Engine not initialized yet - leave status as-is, the poll below will retry.
    }
  };

  const refresh = async () => {
    const models = await invoke<RawModelInfo[]>('gigaam_get_available_models');
    setModel(models[0] ?? null);
  };

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];

    Promise.all([
      listen<{ modelName: string; progress: number }>('gigaam-model-download-progress', ({ payload }) => {
        if (!disposed && payload.modelName === MODEL) setProgress(payload.progress);
      }),
      listen<{ modelName: string }>('gigaam-model-download-complete', async ({ payload }) => {
        if (disposed || payload.modelName !== MODEL) return;
        setProgress(null);
        await refresh();
        toast.success('GigaAM v3 is ready');
        onModelSelect?.(MODEL);
      }),
      listen<{ modelName: string; error: string }>('gigaam-model-download-error', ({ payload }) => {
        if (disposed || payload.modelName !== MODEL) return;
        setProgress(null);
        toast.error('GigaAM v3 download failed', { description: payload.error });
      }),
    ]).then((items) => unlisteners.push(...items));

    invoke('gigaam_init')
      .then(refresh)
      .catch((error) => toast.error('Failed to initialize GigaAM', { description: String(error) }))
      .finally(() => { if (!disposed) setLoading(false); });

    return () => {
      disposed = true;
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, []);

  // Poll the active execution provider while this panel is open. There's no
  // "model loaded" event to listen for instead - loading happens as a side
  // effect of transcription/import, which can be triggered from elsewhere in
  // the app while this panel isn't even mounted.
  useEffect(() => {
    fetchProviderStatus();
    const interval = setInterval(fetchProviderStatus, 3000);
    return () => clearInterval(interval);
  }, []);

  const available = model?.status === 'Available';
  const assignmentDetails = providerStatus?.provider_assignments
    .map(({ provider, node_count }) => `${provider}: ${node_count} operations`)
    .join('\n');
  const isAccelerated = providerStatus?.label?.startsWith('GPU') || providerStatus?.label?.startsWith('CoreML');

  // "Test" button: loads the model (triggering the real DirectML-vs-CPU
  // decision and capturing it via GigaAMModel::active_provider()), reads the
  // now-live status, then unloads again - mirroring what a real
  // transcription does (unload_engine_after_batch), so trying this doesn't
  // leave the model sitting in memory just because someone clicked a button
  // in Settings.
  const testProvider = async () => {
    setTesting(true);
    try {
      await invoke('gigaam_load_model', { modelName: MODEL });
      await fetchProviderStatus();
    } catch (error) {
      toast.error('Failed to load GigaAM v3', { description: String(error) });
    } finally {
      try {
        await invoke('gigaam_unload_model');
      } catch {
        // Best-effort - a failed load already leaves nothing loaded to unload.
      }
      await fetchProviderStatus();
      setTesting(false);
    }
  };
  const select = async () => {
    onModelSelect?.(MODEL);
    if (autoSave) {
      await invoke('api_save_transcript_config', { provider: 'gigaam', model: MODEL, apiKey: null });
    }
  };

  const download = async () => {
    setProgress(0);
    try {
      await invoke('gigaam_download_model', { modelName: MODEL });
    } catch {
      setProgress(null); // The backend event contains and displays the detailed error.
    }
  };

  const remove = async () => {
    await invoke('gigaam_delete_model', { modelName: MODEL });
    await refresh();
    toast.success('GigaAM v3 deleted');
  };

  if (loading) {
    return <div className="h-24 rounded-lg bg-gray-100 animate-pulse" />;
  }

  return (
    <div
      className={`rounded-lg border-2 p-4 ${selectedModel === MODEL && available ? 'border-blue-500 bg-blue-50' : 'border-gray-200 bg-white'}`}
      onClick={() => { if (available) void select(); }}
    >
      <div className="flex items-center justify-between gap-4">
        <div>
          <div className="flex items-center gap-2">
            <span className="font-semibold text-gray-900">🇷🇺 GigaAM v3</span>
            {/* Always rendered, never conditionally hidden - so "it's blank"
                is never confusable with "the component didn't mount" or "the
                request hasn't come back yet". Defaults to a neutral "Unknown"
                badge until gigaam_get_active_provider has ever returned a
                real label (nothing has loaded this session yet). */}
            <span
              className={`flex items-center gap-1 rounded-full px-2 py-0.5 text-xs font-medium ${
                isAccelerated
                  ? 'bg-purple-100 text-purple-700'
                  : providerStatus?.label
                  ? 'bg-gray-100 text-gray-600'
                  : 'bg-gray-100 text-gray-400'
              }`}
              title={
                !providerStatus?.label
                  ? 'Not known yet - nothing has loaded GigaAM this session. Run an import or a recording with GigaAM selected, then check back here.'
                  : ([
                      providerStatus.label.includes('+ CPU')
                        ? `ONNX Runtime split the graph between ${providerStatus.label.replace(' + CPU', '')} and CPU`
                        : isAccelerated
                        ? `All reported graph operations were assigned to ${providerStatus.label}`
                        : providerStatus.fallback_reason || 'All reported graph operations were assigned to CPU',
                      assignmentDetails,
                    ].filter(Boolean).join('\n')) +
                    (providerStatus.is_live
                      ? '\nCurrently loaded'
                      : '\nFrom the last transcription; the model is not loaded now')
              }
            >
              {isAccelerated ? (
                <Zap className="h-3 w-3" />
              ) : (
                <Cpu className="h-3 w-3" />
              )}
              {providerStatus?.label ?? 'Unknown'}
              {providerStatus?.label && !providerStatus.is_live && (
                <span className="opacity-60">(last run)</span>
              )}
            </span>
            {available && (
              <Button
                variant="ghost"
                size="icon"
                className="h-5 w-5"
                title="Load the model, check which provider it actually uses, then unload it again"
                disabled={testing}
                onClick={(event) => { event.stopPropagation(); void testProvider(); }}
              >
                {testing ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <PlayCircle className="h-3.5 w-3.5" />
                )}
              </Button>
            )}
          </div>
          <p className="mt-1 text-sm text-gray-600">Fast and accurate Russian speech recognition · 186 MB</p>
        </div>
        {available ? (
          <div className="flex items-center gap-2">
            <span className="text-xs font-medium text-green-600">Ready</span>
            <Button variant="ghost" size="icon" title="Delete model" onClick={(event) => { event.stopPropagation(); void remove(); }}>
              <Trash2 className="h-4 w-4" />
            </Button>
          </div>
        ) : progress !== null ? (
          <div className="flex min-w-20 items-center gap-2 text-sm text-blue-600">
            <Loader2 className="h-4 w-4 animate-spin" /> {progress}%
          </div>
        ) : (
          <Button size="sm" onClick={(event) => { event.stopPropagation(); void download(); }}>
            <Download className="mr-2 h-4 w-4" /> Download
          </Button>
        )}
      </div>
      {progress !== null && (
        <div className="mt-3 h-2 overflow-hidden rounded-full bg-gray-200">
          <div className="h-full bg-blue-600 transition-all" style={{ width: `${progress}%` }} />
        </div>
      )}
    </div>
  );
}
