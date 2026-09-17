'use client';

import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';
import { toast } from 'sonner';
import { Switch } from '@/components/ui/switch';

interface DiarizationSettingsState {
  enabled: boolean;
  engine: string;
}

export function DiarizationSettings() {
  const [settings, setSettings] = useState<DiarizationSettingsState>({
    enabled: false,
    engine: 'pyannote-wespeaker',
  });
  const [isDownloading, setIsDownloading] = useState(false);

  useEffect(() => {
    void invoke<DiarizationSettingsState>('get_diarization_settings')
      .then(setSettings)
      .catch((error) => console.warn('Failed to load diarization settings:', error));
  }, []);

  const save = (next: DiarizationSettingsState) => {
    setSettings(next);
    void invoke('set_diarization_settings', { settings: next }).catch((error) => {
      console.error('Failed to save diarization settings:', error);
      toast.error('Could not save speaker diarization settings');
    });
  };

  const downloadModels = async () => {
    setIsDownloading(true);
    try {
      await invoke('download_diarization_models', { engine: settings.engine });
      toast.success('Speaker diarization models are ready');
    } catch (error) {
      console.error('Failed to download diarization models:', error);
      toast.error('Could not download speaker diarization models');
    } finally {
      setIsDownloading(false);
    }
  };

  return (
    <section className="mt-6 max-w-3xl rounded-lg border border-gray-200 bg-white p-6 shadow-sm">
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
        <label htmlFor="diarization-engine" className="block text-sm font-medium text-gray-700">
          Engine
        </label>
        <select
          id="diarization-engine"
          value={settings.engine}
          onChange={(event) => save({ ...settings, engine: event.target.value })}
          className="mt-2 w-full max-w-sm rounded-md border border-gray-300 bg-white px-3 py-2 text-sm shadow-sm focus:border-blue-500 focus:outline-none focus:ring-1 focus:ring-blue-500"
        >
          <option value="pyannote-wespeaker">PyAnnote + WeSpeaker</option>
          <option value="nvidia-sortformer-v2">NVIDIA Sortformer v2</option>
        </select>

        <div className="mt-5 flex items-center gap-3">
          <button
            type="button"
            onClick={downloadModels}
            disabled={isDownloading}
            className="rounded-md bg-blue-600 px-3 py-2 text-sm font-medium text-white hover:bg-blue-700 disabled:cursor-not-allowed disabled:bg-gray-400"
          >
            {isDownloading ? 'Downloading…' : 'Download models'}
          </button>
          <span className="text-xs text-gray-500">Required before the first diarized recording.</span>
        </div>
      </div>
    </section>
  );
}
