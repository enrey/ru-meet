'use client';

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { ChevronDown, Loader2 } from 'lucide-react';
import { AudioLevelMeter } from './AudioLevelMeter';
import type { AudioDevice, AudioLevelData, AudioLevelUpdate } from './DeviceSelection';
import { useConfig } from '@/contexts/ConfigContext';

type Devices = { micDevice: string | null; systemDevice: string | null };
type ActiveDevices = { microphone: string | null; system: string | null };
const unsuffix = (value: string | null | undefined) =>
  value?.replace(/ \((input|output)\)$/i, '') || null;

/** Shows capture health, not browser volume: both values come from active WASAPI/CPAL inputs. */
export function LiveAudioStatus({ recording, devices }: { recording: boolean; devices?: Devices }) {
  const { setSelectedDevices } = useConfig();
  const [levels, setLevels] = useState<Map<string, AudioLevelData>>(new Map());
  const [activeDevices, setActiveDevices] = useState<ActiveDevices | null>(null);
  const [availableDevices, setAvailableDevices] = useState<AudioDevice[]>([]);
  const [expandedDevice, setExpandedDevice] = useState<'microphone' | 'system' | null>(null);
  const [switching, setSwitching] = useState<'microphone' | 'system' | null>(null);
  const [switchError, setSwitchError] = useState<string | null>(null);
  const mic = activeDevices?.microphone || unsuffix(devices?.micDevice);
  const system = activeDevices?.system || unsuffix(devices?.systemDevice);

  useEffect(() => {
    if (!recording) { setLevels(new Map()); return; }
    let unlisten: (() => void) | undefined;
    (async () => {
      const active = await invoke<ActiveDevices>('get_active_recording_devices');
      setActiveDevices(active);
      const names = [active.microphone, active.system].filter((item): item is string => Boolean(item));
      unlisten = await listen<AudioLevelUpdate>('audio-levels', ({ payload }) => {
        setLevels(new Map(payload.levels.map(level => [level.device_name, level])));
      });
      if (names.length) await invoke('start_audio_level_monitoring', { deviceNames: names });
    })().catch(console.error);
    return () => {
      unlisten?.();
      invoke('stop_audio_level_monitoring').catch(console.error);
    };
  }, [recording]);

  useEffect(() => {
    if (!recording || !expandedDevice || availableDevices.length) return;
    invoke<AudioDevice[]>('get_audio_devices')
      .then(setAvailableDevices)
      .catch((error) => setSwitchError(`Could not load devices: ${String(error)}`));
  }, [recording, expandedDevice, availableDevices.length]);

  const restartMeter = async (next: ActiveDevices) => {
    const names = [next.microphone, next.system].filter((item): item is string => Boolean(item));
    await invoke('stop_audio_level_monitoring');
    if (names.length) await invoke('start_audio_level_monitoring', { deviceNames: names });
  };

  const switchDevice = async (kind: 'microphone' | 'system', nextName: string) => {
    setExpandedDevice(null);
    setSwitching(kind);
    setSwitchError(null);
    try {
      if (kind === 'microphone') {
        await invoke('switch_recording_microphone', { micDeviceName: nextName });
      } else {
        await invoke('switch_recording_system_audio', { systemDeviceName: nextName });
      }
      // The backend may fall back to a default endpoint if Windows loses a
      // device mid-switch. Reflect what is actually capturing, not the value
      // the dropdown requested.
      const next = await invoke<ActiveDevices>('get_active_recording_devices');
      setActiveDevices(next);
      setSelectedDevices({
        micDevice: next.microphone ? `${next.microphone} (input)` : null,
        systemDevice: next.system ? `${next.system} (output)` : null,
      });
      const preferences = await invoke<Record<string, unknown>>('get_recording_preferences');
      await invoke('set_recording_preferences', {
        preferences: {
          ...preferences,
          preferred_mic_device: next.microphone,
          preferred_system_device: next.system,
        },
      });
      await restartMeter(next);
    } catch (error) {
      setSwitchError(`Could not switch ${kind === 'microphone' ? 'microphone' : 'system audio'}: ${String(error)}`);
    } finally {
      setSwitching(null);
    }
  };

  if (!recording) return null;
  const row = (kind: 'microphone' | 'system', label: string, name: string | null, options: AudioDevice[]) => {
    const level = name ? levels.get(name) : undefined;
    const isExpanded = expandedDevice === kind;
    return <div className="space-y-1" key={label}>
      <span className="text-xs font-medium">{label}</span>
      <button
        type="button"
        onClick={() => setExpandedDevice(isExpanded ? null : kind)}
        className="flex w-full items-center justify-between gap-2 rounded px-1 py-0.5 text-left text-xs text-gray-600 hover:bg-gray-200"
        aria-expanded={isExpanded}
      >
        <span className="truncate">{name || 'Default device'}</span>
        <ChevronDown className={`h-3.5 w-3.5 shrink-0 transition-transform ${isExpanded ? 'rotate-180' : ''}`} />
      </button>
      {isExpanded && <div className="border-t border-gray-200 pt-2">
        <div className="max-h-40 overflow-y-auto rounded-md border border-gray-200 bg-white p-1">
          {options.map((device) => <button
            key={device.name}
            type="button"
            disabled={switching !== null}
            onClick={() => switchDevice(kind, device.name)}
            className={`block w-full rounded px-2 py-1.5 text-left text-xs hover:bg-gray-100 disabled:opacity-50 ${device.name === name ? 'bg-gray-100 font-medium text-gray-900' : 'text-gray-700'}`}
          >{device.name}</button>)}
        </div>
        {switching === kind && <p className="mt-1 flex items-center gap-1 text-xs text-gray-500"><Loader2 className="h-3.5 w-3.5 animate-spin" /> Switching…</p>}
        {switchError && <p className="mt-1 text-xs text-red-600">{switchError}</p>}
      </div>}
      <AudioLevelMeter rmsLevel={level?.rms_level || 0} peakLevel={level?.peak_level || 0} isActive={level?.is_active || false} deviceName={name || label} size="small" />
    </div>;
  };
  const inputs = availableDevices.filter((device) => device.device_type === 'Input');
  const outputs = availableDevices.filter((device) => device.device_type === 'Output');
  return <div className="mt-3 space-y-2 rounded-md border border-gray-200 bg-gray-50 p-3" aria-live="polite">
    <p className="text-xs font-medium text-gray-700">Live capture signal</p>
    {row('microphone', 'Microphone', mic, inputs)}
    {row('system', 'System audio', system, outputs)}
  </div>;
}
