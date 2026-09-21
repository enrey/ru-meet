'use client';

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { ChevronDown, Cpu, Headphones, Loader2, Mic, X, Zap } from 'lucide-react';
import { AudioLevelMeter } from './AudioLevelMeter';
import type { AudioDevice, AudioLevelData, AudioLevelUpdate } from './DeviceSelection';
import { useConfig } from '@/contexts/ConfigContext';
import { configService } from '@/services/configService';
import { normalizeAudioDevicePreferences, stripAudioDeviceSuffix } from '@/lib/audioDevicePreferences';

type Devices = { micDevice: string | null; systemDevice: string | null };
type ActiveDevices = { microphone: string | null; system: string | null };
type SourceMutes = { microphone: boolean; system: boolean };
type ActiveProviderStatus = {
  label: string | null;
  fallback_reason: string | null;
  provider_assignments: Array<{ provider: string; node_count: number }>;
  is_live: boolean;
};

/** Shows capture health, not browser volume: both values come from active WASAPI/CPAL inputs. */
export function LiveAudioStatus({ recording, devices }: { recording: boolean; devices?: Devices }) {
  const { setSelectedDevices, transcriptModelConfig } = useConfig();
  const [levels, setLevels] = useState<Map<string, AudioLevelData>>(new Map());
  const [activeDevices, setActiveDevices] = useState<ActiveDevices | null>(null);
  const [availableDevices, setAvailableDevices] = useState<AudioDevice[]>([]);
  const [expandedDevice, setExpandedDevice] = useState<'microphone' | 'system' | null>(null);
  const [switching, setSwitching] = useState<'microphone' | 'system' | null>(null);
  const [switchError, setSwitchError] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState(false);
  const [sourceMutes, setSourceMutes] = useState<SourceMutes>({ microphone: false, system: false });
  const [togglingSource, setTogglingSource] = useState<'microphone' | 'system' | null>(null);
  const [providerStatus, setProviderStatus] = useState<ActiveProviderStatus | null>(null);
  const mic = activeDevices?.microphone || stripAudioDeviceSuffix(devices?.micDevice);
  const system = activeDevices?.system || stripAudioDeviceSuffix(devices?.systemDevice);

  useEffect(() => {
    if (!recording) {
      setLevels(new Map());
      setCollapsed(false);
      setSourceMutes({ microphone: false, system: false });
      return;
    }
    let unlisten: (() => void) | undefined;
    (async () => {
      const [active, mutes] = await Promise.all([
        invoke<ActiveDevices>('get_active_recording_devices'),
        invoke<SourceMutes>('get_recording_source_mutes'),
      ]);
      setActiveDevices(active);
      setSourceMutes(mutes);
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
    if (!recording || transcriptModelConfig.provider !== 'gigaam') {
      setProviderStatus(null);
      return;
    }

    let disposed = false;
    const refreshProviderStatus = async () => {
      try {
        const status = await invoke<ActiveProviderStatus>('gigaam_get_active_provider');
        if (!disposed) setProviderStatus(status);
      } catch {
        // Model startup and provider detection happen asynchronously; retry below.
      }
    };

    void refreshProviderStatus();
    const interval = window.setInterval(refreshProviderStatus, 3000);
    return () => {
      disposed = true;
      window.clearInterval(interval);
    };
  }, [recording, transcriptModelConfig.provider]);

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
      const selected = normalizeAudioDevicePreferences({
        micDevice: next.microphone,
        systemDevice: next.system,
      });
      setSelectedDevices(selected);
      await configService.saveRecordingDevicePreferences(selected);
      await restartMeter(next);
    } catch (error) {
      setSwitchError(`Could not switch ${kind === 'microphone' ? 'microphone' : 'system audio'}: ${String(error)}`);
    } finally {
      setSwitching(null);
    }
  };

  const toggleSource = async (kind: 'microphone' | 'system') => {
    setExpandedDevice(null);
    setTogglingSource(kind);
    setSwitchError(null);
    try {
      const mutes = await invoke<SourceMutes>('set_recording_source_muted', {
        source: kind,
        muted: !sourceMutes[kind],
      });
      setSourceMutes(mutes);
    } catch (error) {
      setSwitchError(`Could not ${sourceMutes[kind] ? 'enable' : 'mute'} ${kind === 'microphone' ? 'microphone' : 'system audio'}: ${String(error)}`);
    } finally {
      setTogglingSource(null);
    }
  };

  if (!recording) return null;
  const row = (kind: 'microphone' | 'system', label: string, name: string | null, options: AudioDevice[]) => {
    const level = name ? levels.get(name) : undefined;
    const isExpanded = expandedDevice === kind;
    const isMuted = sourceMutes[kind];
    const SourceIcon = kind === 'microphone' ? Mic : Headphones;
    return <div className="space-y-1" key={label}>
      <button
        type="button"
        onClick={() => toggleSource(kind)}
        disabled={togglingSource !== null}
        className={`flex items-center gap-1.5 text-xs font-medium transition-colors disabled:opacity-50 ${isMuted ? 'text-gray-400' : 'text-gray-800 hover:text-gray-600'}`}
        aria-pressed={isMuted}
        title={isMuted ? `Enable ${label}` : `Mute ${label}`}
      >
        <span className="relative inline-flex">
          <SourceIcon className="h-3.5 w-3.5" aria-hidden="true" />
          {isMuted && <span className="absolute left-[-1px] top-1/2 h-px w-[18px] -rotate-45 bg-current" aria-hidden="true" />}
        </span>
        <span className={isMuted ? 'line-through' : ''}>{label}</span>
        {togglingSource === kind && <Loader2 className="h-3 w-3 animate-spin" aria-hidden="true" />}
      </button>
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
      <AudioLevelMeter
        rmsLevel={isMuted ? 0 : level?.rms_level || 0}
        peakLevel={isMuted ? 0 : level?.peak_level || 0}
        isActive={!isMuted && (level?.is_active || false)}
        deviceName={name || label}
        size="small"
      />
    </div>;
  };
  const inputs = availableDevices.filter((device) => device.device_type === 'Input');
  const outputs = availableDevices.filter((device) => device.device_type === 'Output');
  // The settings badge can intentionally show the last run. Here the user
  // asked for the current recording mode, so do not present a stale provider
  // while GigaAM is still starting up.
  const providerLabel = providerStatus?.is_live ? providerStatus.label : null;
  const providerAssignments = providerStatus?.provider_assignments
    .map(({ provider, node_count }) => `${provider}: ${node_count} operations`)
    .join('\n');
  const isAccelerated = providerLabel?.startsWith('GPU') || providerLabel?.startsWith('CoreML');
  return <div className="mt-3 space-y-2 rounded-md border border-gray-200 bg-gray-50 p-3" aria-live="polite">
    <div className="flex items-center justify-between gap-2">
      {collapsed ? <button
        type="button"
        onClick={() => setCollapsed(false)}
        className="text-left text-xs font-medium text-gray-700 hover:text-gray-900"
        aria-expanded="false"
      >
        Live capture signal
      </button> : <p className="text-xs font-medium text-gray-700">Live capture signal</p>}
      {!collapsed && <div className="flex items-center gap-2">
        {transcriptModelConfig.provider === 'gigaam' && <span
          className={`flex items-center gap-1 rounded-full px-2 py-0.5 text-xs font-medium ${
            isAccelerated
              ? 'bg-purple-100 text-purple-700'
              : providerLabel
              ? 'bg-gray-100 text-gray-600'
              : 'bg-gray-100 text-gray-400'
          }`}
          title={
            !providerLabel
              ? 'Detecting the active GigaAM execution mode…'
              : [
                  providerLabel.includes('+ CPU')
                    ? `ONNX Runtime split the graph between ${providerLabel.replace(' + CPU', '')} and CPU`
                    : isAccelerated
                    ? `All reported graph operations are assigned to ${providerLabel}`
                    : providerStatus?.fallback_reason || 'All reported graph operations are assigned to CPU',
                  providerAssignments,
                ].filter(Boolean).join('\n')
          }
        >
          {isAccelerated
            ? <Zap className="h-3 w-3" aria-hidden="true" />
            : <Cpu className="h-3 w-3" aria-hidden="true" />}
          {providerLabel ?? 'Detecting…'}
        </span>}
        <button
          type="button"
          onClick={() => {
            setExpandedDevice(null);
            setCollapsed(true);
          }}
          className="rounded p-0.5 text-gray-500 hover:bg-gray-200 hover:text-gray-800"
          aria-label="Collapse live capture signal"
          title="Collapse"
        >
          <X className="h-3.5 w-3.5" aria-hidden="true" />
        </button>
      </div>}
    </div>
    {!collapsed && <>
      {row('microphone', 'Microphone', mic, inputs)}
      {row('system', 'System audio', system, outputs)}
    </>}
  </div>;
}
