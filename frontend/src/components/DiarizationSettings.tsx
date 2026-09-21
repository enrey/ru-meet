'use client';

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { CheckCircle2, Download, Loader2, RadioTower, UsersRound } from 'lucide-react';
import { useEffect, useState } from 'react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Switch } from '@/components/ui/switch';
import { DIARIZATION_MODELS, type DiarizationEngineId } from '@/lib/diarization-models';

interface DiarizationSettingsState {
  enabled: boolean;
  engine: string;
}

interface DiarizationModelStatus {
  engine: string;
  ready: boolean;
}

export function DiarizationSettings() {
  const [settings, setSettings] = useState<DiarizationSettingsState>({
    enabled: false,
    engine: 'pyannote-wespeaker',
  });
  const [modelStatuses, setModelStatuses] = useState<Record<string, boolean>>({});
  const [isLoading, setIsLoading] = useState(true);
  const [isDownloading, setIsDownloading] = useState(false);
  const [downloadProgress, setDownloadProgress] = useState(0);
  const [downloadingEngine, setDownloadingEngine] = useState<string | null>(null);

  const refreshModelStatuses = async () => {
    const statuses = await invoke<DiarizationModelStatus[]>('get_diarization_model_statuses');
    setModelStatuses(Object.fromEntries(statuses.map(({ engine, ready }) => [engine, ready])));
  };

  useEffect(() => {
    let disposed = false;
    const unlisten = listen<{ engine: string; progress: number }>(
      'diarization-download-progress',
      ({ payload }) => {
        if (disposed) return;
        setDownloadingEngine(payload.engine);
        setDownloadProgress(payload.progress);
      },
    );

    void Promise.all([
      invoke<DiarizationSettingsState>('get_diarization_settings').then((value) => {
        if (!disposed) setSettings(value);
      }),
      refreshModelStatuses(),
    ])
      .catch((error) => console.warn('Failed to load diarization settings:', error))
      .finally(() => {
        if (!disposed) setIsLoading(false);
      });

    return () => {
      disposed = true;
      void unlisten.then((stop) => stop());
    };
  }, []);

  const save = (next: DiarizationSettingsState) => {
    setSettings(next);
    void invoke('set_diarization_settings', { settings: next }).catch((error) => {
      console.error('Failed to save diarization settings:', error);
      toast.error('Could not save speaker diarization settings');
    });
  };

  const downloadModels = async (engine: DiarizationEngineId) => {
    setIsDownloading(true);
    setDownloadingEngine(engine);
    setDownloadProgress(0);
    try {
      await invoke('download_diarization_models', { engine });
      await refreshModelStatuses();
      toast.success('Speaker diarization models are ready');
    } catch (error) {
      console.error('Failed to download diarization models:', error);
      toast.error('Could not download speaker diarization models');
    } finally {
      setIsDownloading(false);
      setDownloadingEngine(null);
      setDownloadProgress(0);
    }
  };

  return (
    <section className="mt-6 w-full rounded-xl border border-gray-200 bg-white p-8 shadow-sm">
      <div className="flex items-start justify-between gap-6">
        <div>
          <h2 className="text-lg font-semibold text-gray-900">Speaker diarization</h2>
          <p className="mt-1 text-sm text-gray-600">
            Identify and label speakers after a recording has been saved.
          </p>
        </div>
        <Switch
          checked={settings.enabled}
          onCheckedChange={(enabled) => save({ ...settings, enabled })}
          aria-label="Enable speaker diarization"
        />
      </div>

      <div className="mt-6 border-t border-gray-100 pt-6">
        <div className="mb-4">
          <h3 className="text-sm font-medium text-gray-900">Diarization model</h3>
          <p className="mt-1 text-sm text-gray-500">Choose which local model identifies speakers after recording.</p>
        </div>

        {isLoading ? (
          <div className="grid gap-4">
            <div className="h-40 animate-pulse rounded-xl bg-gray-100" />
            <div className="h-40 animate-pulse rounded-xl bg-gray-100" />
          </div>
        ) : (
          <div className="grid gap-4">
            {(Object.entries(DIARIZATION_MODELS) as Array<[DiarizationEngineId, (typeof DIARIZATION_MODELS)[DiarizationEngineId]]>).map(([id, model]) => {
              const selected = settings.engine === id;
              const ready = modelStatuses[id] ?? false;
              const downloading = isDownloading && downloadingEngine === id;
              const Icon = id === 'pyannote-wespeaker' ? UsersRound : RadioTower;

              return (
                <div
                  key={id}
                  role="button"
                  tabIndex={0}
                  onClick={() => save({ ...settings, engine: id })}
                  onKeyDown={(event) => {
                    if (event.target === event.currentTarget && (event.key === 'Enter' || event.key === ' ')) {
                      event.preventDefault();
                      save({ ...settings, engine: id });
                    }
                  }}
                  className={`group flex min-h-40 cursor-pointer flex-col rounded-xl border-2 p-5 text-left transition-all focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2 ${
                    selected
                      ? 'border-blue-500 bg-blue-50 shadow-sm'
                      : 'border-gray-200 bg-white hover:border-gray-300 hover:shadow-sm'
                  }`}
                  aria-pressed={selected}
                >
                  <div className="flex w-full items-start justify-between gap-4">
                    <div className={`flex h-10 w-10 shrink-0 items-center justify-center rounded-lg ${selected ? 'bg-blue-600 text-white' : 'bg-gray-100 text-gray-600'}`}>
                      <Icon className="h-5 w-5" />
                    </div>
                    {ready ? (
                      <span className="flex items-center gap-1.5 rounded-full bg-green-100 px-2.5 py-1 text-xs font-medium text-green-700">
                        <CheckCircle2 className="h-3.5 w-3.5" /> Ready
                      </span>
                    ) : downloading ? (
                      <span className="flex items-center gap-1.5 text-sm font-medium text-blue-600">
                        <Loader2 className="h-4 w-4 animate-spin" /> {downloadProgress}%
                      </span>
                    ) : (
                      <Button
                        type="button"
                        size="sm"
                        onClick={(event) => {
                          event.stopPropagation();
                          void downloadModels(id);
                        }}
                      >
                        <Download className="mr-2 h-4 w-4" /> Download
                      </Button>
                    )}
                  </div>

                  <div className="mt-4">
                    <div className="flex items-center gap-2">
                      <span className="font-semibold text-gray-900">{model.name}</span>
                      <span className="text-xs font-medium text-gray-500">{model.size}</span>
                    </div>
                    <p className="mt-1.5 text-sm leading-5 text-gray-600">{model.description}</p>
                  </div>

                  {downloading && (
                    <div className="mt-auto w-full pt-4">
                      <div className="h-2 overflow-hidden rounded-full bg-blue-100">
                        <div className="h-full bg-blue-600 transition-all" style={{ width: `${downloadProgress}%` }} />
                      </div>
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>
    </section>
  );
}
