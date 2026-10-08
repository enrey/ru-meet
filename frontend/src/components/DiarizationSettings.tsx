'use client';

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { CheckCircle2, Download, Loader2, RadioTower, Sparkles, UsersRound } from 'lucide-react';
import { useEffect, useState } from 'react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Switch } from '@/components/ui/switch';
import { useI18n } from '@/lib/i18n';
import { DEFAULT_DIARIZATION_ENGINE, DIARIZATION_MODELS, type DiarizationEngineId } from '@/lib/diarization-models';

const ENGINE_ICONS: Record<DiarizationEngineId, typeof UsersRound> = {
  'pyannote-wespeaker': UsersRound,
  'speakrs-pyannote-wespeaker': Sparkles,
  'nvidia-sortformer-v2': RadioTower,
};

interface DiarizationSettingsState {
  enabled: boolean;
  engine: string;
  collapseMinorSpeakers: boolean;
}

interface DiarizationModelStatus {
  engine: string;
  ready: boolean;
}

export function DiarizationSettings() {
  const { t } = useI18n();
  const [settings, setSettings] = useState<DiarizationSettingsState>({
    enabled: false,
    engine: DEFAULT_DIARIZATION_ENGINE,
    collapseMinorSpeakers: true,
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
      toast.error(t('Could not save speaker diarization settings'));
    });
  };

  const downloadModels = async (engine: DiarizationEngineId) => {
    setIsDownloading(true);
    setDownloadingEngine(engine);
    setDownloadProgress(0);
    try {
      await invoke('download_diarization_models', { engine });
      await refreshModelStatuses();
      toast.success(t('Speaker diarization models are ready'));
    } catch (error) {
      console.error('Failed to download diarization models:', error);
      toast.error(t('Could not download speaker diarization models'));
    } finally {
      setIsDownloading(false);
      setDownloadingEngine(null);
      setDownloadProgress(0);
    }
  };

  return (
    <section className="mt-6 w-full rounded-xl border border-slate-200 bg-white p-8 shadow-sm">
      <div className="flex items-start justify-between gap-6">
        <div>
          <h2 className="text-lg font-semibold text-slate-900">{t('Speaker diarization')}</h2>
          <p className="mt-1 text-sm text-slate-600">
            {t('Identify and label speakers after a recording has been saved.')}
          </p>
        </div>
        <Switch
          checked={settings.enabled}
          onCheckedChange={(enabled) => save({ ...settings, enabled })}
          aria-label={t('Enable speaker diarization')}
        />
      </div>

      <div className="mt-6 flex items-start justify-between gap-6 border-t border-slate-100 pt-6">
        <div>
          <h3 className="text-sm font-medium text-slate-900">{t('Merge barely-heard speakers')}</h3>
          <p className="mt-1 text-sm text-slate-500">
            {t('Speakers with under 1% of the talking — and less than 30 seconds of it — are labelled “Others” instead of getting their own entry. Re-run diarization with this off to get them back.')}
          </p>
        </div>
        <Switch
          checked={settings.collapseMinorSpeakers}
          onCheckedChange={(collapseMinorSpeakers) => save({ ...settings, collapseMinorSpeakers })}
          aria-label={t('Merge barely-heard speakers into Others')}
        />
      </div>

      <div className="mt-6 border-t border-slate-100 pt-6">
        <div className="mb-4">
          <h3 className="text-sm font-medium text-slate-900">{t('Diarization model')}</h3>
          <p className="mt-1 text-sm text-slate-500">{t('Choose which local model identifies speakers after recording.')}</p>
        </div>

        {isLoading ? (
          <div className="grid gap-2">
            <div className="h-16 animate-pulse rounded-lg bg-slate-100" />
            <div className="h-16 animate-pulse rounded-lg bg-slate-100" />
            <div className="h-16 animate-pulse rounded-lg bg-slate-100" />
          </div>
        ) : (
          <div className="grid gap-2">
            {(Object.entries(DIARIZATION_MODELS) as Array<[DiarizationEngineId, (typeof DIARIZATION_MODELS)[DiarizationEngineId]]>).map(([id, model]) => {
              const selected = settings.engine === id;
              const ready = modelStatuses[id] ?? false;
              const downloading = isDownloading && downloadingEngine === id;
              const Icon = ENGINE_ICONS[id];

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
                  className={`group cursor-pointer rounded-lg border px-3 py-2.5 text-left transition-all focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:ring-offset-2 ${
                    selected
                      ? 'border-indigo-500 bg-indigo-50'
                      : 'border-slate-200 bg-white hover:border-slate-300'
                  }`}
                  aria-pressed={selected}
                >
                  <div className="flex items-center gap-3">
                    <div className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-md ${selected ? 'bg-indigo-600 text-white' : 'bg-slate-100 text-slate-600'}`}>
                      <Icon className="h-4 w-4" />
                    </div>
                    <div className="min-w-0 flex-1">
                      <div className="flex flex-wrap items-baseline gap-x-2">
                        <span className="text-sm font-semibold text-slate-900">{model.name}</span>
                        <span className="text-xs font-medium text-slate-500">{model.size}</span>
                      </div>
                      <p className="truncate text-xs leading-4 text-slate-600" title={t(model.description)}>
                        {t(model.description)}
                      </p>
                    </div>
                    {ready ? (
                      <span className="flex shrink-0 items-center gap-1 rounded-full bg-green-100 px-2 py-0.5 text-xs font-medium text-green-700">
                        <CheckCircle2 className="h-3.5 w-3.5" /> {t('Ready')}
                      </span>
                    ) : downloading ? (
                      <span className="flex shrink-0 items-center gap-1.5 text-xs font-medium text-indigo-600">
                        <Loader2 className="h-3.5 w-3.5 animate-spin" /> {downloadProgress}%
                      </span>
                    ) : (
                      <Button
                        type="button"
                        size="sm"
                        className="h-7 shrink-0 px-2.5 text-xs"
                        onClick={(event) => {
                          event.stopPropagation();
                          void downloadModels(id);
                        }}
                      >
                        <Download className="mr-1.5 h-3.5 w-3.5" /> {t('Download')}
                      </Button>
                    )}
                  </div>

                  {downloading && (
                    <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-indigo-100">
                      <div className="h-full bg-indigo-600 transition-all" style={{ width: `${downloadProgress}%` }} />
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
