import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Download, Loader2, Trash2 } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from './ui/button';
import type { RawModelInfo } from '@/hooks/useTranscriptionModels';

const MODEL = 'gigaam-v3-e2e-ctc';

interface Props {
  selectedModel?: string;
  onModelSelect?: (modelName: string) => void;
  autoSave?: boolean;
}

export function GigaAMModelManager({ selectedModel, onModelSelect, autoSave = false }: Props) {
  const [model, setModel] = useState<RawModelInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [progress, setProgress] = useState<number | null>(null);

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

  const available = model?.status === 'Available';
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
          <div className="font-semibold text-gray-900">🇷🇺 GigaAM v3</div>
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
