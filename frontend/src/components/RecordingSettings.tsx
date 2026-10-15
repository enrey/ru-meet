import React, { useState, useEffect } from 'react';
import { Switch } from '@/components/ui/switch';
import { FolderOpen } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { DeviceSelection, SelectedDevices } from '@/components/DeviceSelection';
import { toast } from 'sonner';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { useConfig } from '@/contexts/ConfigContext';
import { normalizeAudioDevicePreferences, stripAudioDeviceSuffix } from '@/lib/audioDevicePreferences';
import { useI18n } from '@/lib/i18n';

export interface RecordingPreferences {
  save_folder: string;
  auto_save: boolean;
  file_format: string;
  preferred_mic_device: string | null;
  preferred_system_device: string | null;
  trim_silent_edges: boolean;
}

interface ManualSilenceStop {
  enabled: boolean;
  minutes: number;
}

interface RecordingSettingsProps {
  onSave?: (preferences: RecordingPreferences) => void;
}

export function RecordingSettings({ onSave }: RecordingSettingsProps) {
  const [preferences, setPreferences] = useState<RecordingPreferences>({
    save_folder: '',
    auto_save: true,
    file_format: 'mp4',
    preferred_mic_device: null,
    preferred_system_device: null,
    trim_silent_edges: true
  });
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [showRecordingNotification, setShowRecordingNotification] = useState(true);
  const [silenceStop, setSilenceStop] = useState<ManualSilenceStop | null>(null);
  const [silenceMinutes, setSilenceMinutes] = useState('');
  const { isRecording } = useRecordingState();
  const { setSelectedDevices } = useConfig();
  const { t } = useI18n();

  // Load recording preferences on component mount
  useEffect(() => {
    const loadPreferences = async () => {
      try {
        const prefs = await invoke<RecordingPreferences>('get_recording_preferences');
        const devices = normalizeAudioDevicePreferences({
          micDevice: prefs.preferred_mic_device,
          systemDevice: prefs.preferred_system_device,
        });
        setPreferences({
          ...prefs,
          preferred_mic_device: devices.micDevice,
          preferred_system_device: devices.systemDevice,
        });
        setSelectedDevices(devices);
      } catch (error) {
        console.error('Failed to load recording preferences:', error);
        // If loading fails, get default folder path
        try {
          const defaultPath = await invoke<string>('get_default_recordings_folder_path');
          setPreferences(prev => ({ ...prev, save_folder: defaultPath }));
        } catch (defaultError) {
          console.error('Failed to get default folder path:', defaultError);
        }
      } finally {
        setLoading(false);
      }
    };

    loadPreferences();
  }, []);

  // Load recording notification preference
  useEffect(() => {
    const loadNotificationPref = async () => {
      try {
        const { Store } = await import('@/lib/portableStore');
        const store = await Store.load('preferences.json');
        const show = await store.get<boolean>('show_recording_notification') ?? true;
        setShowRecordingNotification(show);
      } catch (error) {
        console.error('Failed to load notification preference:', error);
      }
    };
    loadNotificationPref();
  }, []);

  useEffect(() => {
    invoke<ManualSilenceStop>('get_manual_silence_stop')
      .then(value => { setSilenceStop(value); setSilenceMinutes(String(value.minutes)); })
      .catch(error => console.error('Failed to load silence stop preference:', error));
  }, []);

  const saveSilenceStop = async (enabled: boolean, minutesText: string) => {
    const minutes = Number(minutesText);
    if (!Number.isInteger(minutes) || minutes < 1 || minutes > 600) {
      toast.error(t('Enter from 1 to 600 minutes'));
      if (silenceStop) setSilenceMinutes(String(silenceStop.minutes));
      return;
    }
    try {
      const saved = await invoke<ManualSilenceStop>('set_manual_silence_stop', { enabled, minutes });
      setSilenceStop(saved);
      setSilenceMinutes(String(saved.minutes));
    } catch (error) {
      toast.error(t('Failed to save preference'), { description: String(error) });
      if (silenceStop) setSilenceMinutes(String(silenceStop.minutes));
    }
  };

  const handleAutoSaveToggle = async (enabled: boolean) => {
    const newPreferences = { ...preferences, auto_save: enabled };
    setPreferences(newPreferences);
    await savePreferences(newPreferences);
  };

  const handleTrimToggle = async (enabled: boolean) => {
    const newPreferences = { ...preferences, trim_silent_edges: enabled };
    setPreferences(newPreferences);
    await savePreferences(newPreferences);
  };

  const handleDeviceChange = async (devices: SelectedDevices) => {
    const newPreferences = {
      ...preferences,
      preferred_mic_device: devices.micDevice,
      preferred_system_device: devices.systemDevice
    };
    setPreferences(newPreferences);
    // Sync the in-memory selection ConfigContext exposes to the start path.
    // Without this, the picker only writes to disk (loaded into context once at
    // app mount), so a newly-chosen mic isn't honored until the next launch —
    // start keeps sending the stale launch-time device.
    setSelectedDevices(devices);
    await savePreferences(newPreferences);

    if (isRecording) {
      try {
        const microphoneChanged = preferences.preferred_mic_device !== devices.micDevice;
        const systemAudioChanged = preferences.preferred_system_device !== devices.systemDevice;
        if (microphoneChanged && systemAudioChanged) {
          await invoke('switch_recording_devices', {
            micDeviceName: devices.micDevice,
            systemDeviceName: devices.systemDevice,
          });
          toast.success(t('Microphone and system audio switched'));
        } else if (microphoneChanged) {
          await invoke('switch_recording_microphone', { micDeviceName: stripAudioDeviceSuffix(devices.micDevice) });
          toast.success(t('Microphone switched'));
        } else if (systemAudioChanged) {
          await invoke('switch_recording_system_audio', { systemDeviceName: stripAudioDeviceSuffix(devices.systemDevice) });
          toast.success(t('System audio switched'));
        }
      } catch (error) {
        toast.error(t('Could not switch recording devices'), { description: String(error) });
      }
    }
  };

  const handleOpenFolder = async () => {
    try {
      await invoke('open_recordings_folder');
    } catch (error) {
      console.error('Failed to open recordings folder:', error);
    }
  };

  const handleNotificationToggle = async (enabled: boolean) => {
    try {
      setShowRecordingNotification(enabled);
      const { Store } = await import('@/lib/portableStore');
      const store = await Store.load('preferences.json');
      await store.set('show_recording_notification', enabled);
      await store.save();
      toast.success(t('Preference saved'));
    } catch (error) {
      console.error('Failed to save notification preference:', error);
      toast.error(t('Failed to save preference'));
    }
  };

  const savePreferences = async (prefs: RecordingPreferences) => {
    setSaving(true);
    try {
      await invoke('set_recording_preferences', { preferences: prefs });
      onSave?.(prefs);
    } catch (error) {
      console.error('Failed to save recording preferences:', error);
      toast.error(t('Failed to save device preferences'), {
        description: error instanceof Error ? error.message : String(error)
      });
    } finally {
      setSaving(false);
    }
  };

  if (loading) {
    return (
      <div className="animate-pulse">
        <div className="h-4 bg-slate-200 rounded w-1/4 mb-4"></div>
        <div className="h-8 bg-slate-200 rounded mb-4"></div>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div>
        <h3 className="text-lg font-semibold mb-4">{t('Recording Settings')}</h3>
        <p className="text-sm text-slate-600 mb-6">
          {t('Configure how your audio recordings are saved during meetings.')}
        </p>
      </div>

      {/* Auto Save Toggle */}
      <div className="flex items-center justify-between p-4 border rounded-lg">
        <div className="flex-1">
          <div className="font-medium">{t('Save Audio Recordings')}</div>
          <div className="text-sm text-slate-600">
            {t('Automatically save audio files when recording stops')}
          </div>
        </div>
        <Switch
          checked={preferences.auto_save}
          onCheckedChange={handleAutoSaveToggle}
          disabled={saving}
        />
      </div>

      {/* Folder Location - Only shown when auto_save is enabled */}
      {preferences.auto_save && (
        <div className="space-y-4">
          <div className="p-4 border rounded-lg bg-slate-50">
            <div className="font-medium mb-2">{t('Save Location')}</div>
            <div className="text-sm text-slate-600 mb-3 break-all">
              {preferences.save_folder || t('Default folder')}
            </div>
            <button
              onClick={handleOpenFolder}
              className="flex items-center gap-2 px-3 py-2 text-sm border border-slate-300 rounded-md hover:bg-slate-50 transition-colors"
            >
              <FolderOpen className="w-4 h-4" />
              {t('Open Folder')}
            </button>
          </div>

          <div className="p-4 border rounded-lg bg-indigo-50">
            <div className="text-sm text-indigo-800">
              <strong>{t('File Format:')}</strong> {t('{format} files', { format: preferences.file_format.toUpperCase() })}
            </div>
            <div className="text-xs text-indigo-600 mt-1">
              {t('Recordings are saved with timestamp: {pattern}', { pattern: `recording_YYYYMMDD_HHMMSS.${preferences.file_format}` })}
            </div>
          </div>
        </div>
      )}

      {/* Info when auto_save is disabled */}
      {!preferences.auto_save && (
        <div className="p-4 border rounded-lg bg-yellow-50">
          <div className="text-sm text-yellow-800">
            {t('Audio recording is disabled. Enable "Save Audio Recordings" to automatically save your meeting audio.')}
          </div>
        </div>
      )}

      {/* Recording Notification Toggle */}
      <div className="flex items-center justify-between p-4 border rounded-lg">
        <div className="flex-1">
          <div className="font-medium">{t('Recording Start Notification')}</div>
          <div className="text-sm text-slate-600">
            {t('Show reminder to inform participants when recording starts')}
          </div>
        </div>
        <Switch
          checked={showRecordingNotification}
          onCheckedChange={handleNotificationToggle}
        />
      </div>

      {/* Silence stop for manual recordings */}
      <div className="flex items-center justify-between gap-4 p-4 border rounded-lg">
        <div className="flex-1">
          <label htmlFor="manual-silence-minutes" className="font-medium">{t('Stop a manual recording after silence longer than (minutes)')}</label>
          <div className="text-sm text-slate-600">
            {t('For when you forget to press Stop after a meeting. Automatic recordings end on their own.')}
          </div>
        </div>
        <input
          id="manual-silence-minutes"
          type="number"
          inputMode="numeric"
          min={1}
          max={600}
          value={silenceMinutes}
          disabled={!silenceStop}
          onChange={event => setSilenceMinutes(event.target.value)}
          onBlur={() => { if (silenceStop && silenceMinutes !== String(silenceStop.minutes)) void saveSilenceStop(silenceStop.enabled, silenceMinutes); }}
          onKeyDown={event => { if (event.key === 'Enter') event.currentTarget.blur(); }}
          className="w-20 rounded-md border border-slate-300 px-2 py-1 text-sm tabular-nums disabled:opacity-50"
        />
        <Switch
          checked={silenceStop?.enabled ?? false}
          disabled={!silenceStop}
          onCheckedChange={enabled => void saveSilenceStop(enabled, silenceMinutes)}
          aria-label={t('Stop a manual recording after silence')}
        />
      </div>

      {/* Silent edges */}
      <div className="flex items-center justify-between p-4 border rounded-lg">
        <div className="flex-1">
          <div className="font-medium">{t('Trim silence at the edges of a recording')}</div>
          <div className="text-sm text-slate-600">
            {t('When a recording is saved, the silence before the first phrase and after the last one is cut off, keeping 2 seconds around them.')}
          </div>
        </div>
        <Switch
          checked={preferences.trim_silent_edges}
          onCheckedChange={handleTrimToggle}
          disabled={saving}
          aria-label={t('Trim silence at the edges of a recording')}
        />
      </div>

      {/* Device Preferences */}
      <div className="space-y-4">
        <div className="border-t pt-6">
          <h4 className="text-base font-medium text-slate-900 mb-4">{t('Default Audio Devices')}</h4>
          <p className="text-sm text-slate-600 mb-4">
            {t('Set your preferred microphone and system audio devices for recording. These will be automatically selected when starting new recordings.')}
          </p>

          {isRecording && <p role="status" aria-live="polite" className="text-sm text-indigo-700 bg-indigo-50 border border-indigo-200 rounded-md p-2 mb-4">{t('Changes are applied to the active recording without stopping it.')}</p>}

          <div className="border rounded-lg p-4 bg-slate-50">
            <DeviceSelection
              selectedDevices={{
                micDevice: preferences.preferred_mic_device,
                systemDevice: preferences.preferred_system_device
              }}
              onDeviceChange={handleDeviceChange}
              disabled={saving}
            />
          </div>
        </div>
      </div>
    </div>
  );
}
