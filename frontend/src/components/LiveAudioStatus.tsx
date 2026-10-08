'use client';

import { useEffect, useRef, useState, type HTMLAttributes } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { ChevronDown, ChevronUp, Cpu, GripHorizontal, Headphones, Loader2, Mic, Zap } from 'lucide-react';
import { AudioLevelMeter } from './AudioLevelMeter';
import type { AudioDevice, AudioLevelData, AudioLevelUpdate } from './DeviceSelection';
import { useConfig } from '@/contexts/ConfigContext';
import { configService } from '@/services/configService';
import { normalizeAudioDevicePreferences, stripAudioDeviceSuffix } from '@/lib/audioDevicePreferences';
import { useI18n } from '@/lib/i18n';
import { notifyRecordingSourceMutesChanged } from '@/hooks/useRecordingSourceMutes';

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
export function LiveAudioStatus({ recording, devices, dragHandleProps, alwaysVisible = false, disabled = false }: {
  recording: boolean;
  devices?: Devices;
  dragHandleProps?: HTMLAttributes<HTMLDivElement>;
  alwaysVisible?: boolean;
  disabled?: boolean;
}) {
  const { setSelectedDevices, transcriptModelConfig } = useConfig();
  const { t } = useI18n();
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
  const monitoringQueue = useRef<Promise<void>>(Promise.resolve());
  const mic = activeDevices?.microphone || stripAudioDeviceSuffix(devices?.micDevice);
  const system = activeDevices?.system || stripAudioDeviceSuffix(devices?.systemDevice);

  useEffect(() => {
    setLevels(new Map());
    if (!recording) {
      setActiveDevices(null);
    }
    if (disabled || (!recording && !alwaysVisible)) return;
    let cancelled = false;
    let started = false;
    let unlisten: (() => void) | undefined;
    // Serialize cleanup and setup so a late preview cannot stop the live meter.
    monitoringQueue.current = monitoringQueue.current.then(async () => {
      if (cancelled) return;
      let active: ActiveDevices;
      let mutes: SourceMutes = { microphone: false, system: false };
      if (recording) {
        [active, mutes] = await Promise.all([
          invoke<ActiveDevices>('get_active_recording_devices'),
          invoke<SourceMutes>('get_recording_source_mutes'),
        ]);
      } else {
        const [available, savedMutes] = await Promise.all([
          invoke<AudioDevice[]>('get_audio_devices'),
          invoke<SourceMutes>('get_recording_source_mutes'),
        ]);
        mutes = savedMutes;
        if (cancelled) return;
        setAvailableDevices(available);
        active = {
          microphone: stripAudioDeviceSuffix(devices?.micDevice) || available.find(device => device.device_type === 'Input')?.name || null,
          system: stripAudioDeviceSuffix(devices?.systemDevice) || available.find(device => device.device_type === 'Output')?.name || null,
        };
      }
      if (cancelled) return;
      setActiveDevices(active);
      setSourceMutes(mutes);
      notifyRecordingSourceMutesChanged(mutes);
      const names = [active.microphone, active.system].filter((item): item is string => Boolean(item));
      unlisten = await listen<AudioLevelUpdate>('audio-levels', ({ payload }) => {
        if (!cancelled) setLevels(new Map(payload.levels.map(level => [level.device_name, level])));
      });
      if (cancelled) { unlisten(); unlisten = undefined; return; }
      if (names.length) {
        await invoke('start_audio_level_monitoring', { deviceNames: names });
        started = true;
      }
    }).catch(console.error);
    return () => {
      cancelled = true;
      unlisten?.();
      monitoringQueue.current = monitoringQueue.current.then(async () => {
        unlisten?.();
        if (started) await invoke('stop_audio_level_monitoring');
      }).catch(console.error);
    };
  }, [recording, alwaysVisible, disabled, devices?.micDevice, devices?.systemDevice]);

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
    if (!expandedDevice || availableDevices.length) return;
    invoke<AudioDevice[]>('get_audio_devices')
      .then(setAvailableDevices)
      .catch((error) => setSwitchError(t('Could not load devices: {error}', { error: String(error) })));
  }, [recording, expandedDevice, availableDevices.length]);

  const switchDevice = async (kind: 'microphone' | 'system', nextName: string) => {
    setExpandedDevice(null);
    setSwitching(kind);
    setSwitchError(null);
    try {
      if (!recording) {
        const selected = normalizeAudioDevicePreferences({
          micDevice: kind === 'microphone' ? nextName : devices?.micDevice ?? null,
          systemDevice: kind === 'system' ? nextName : devices?.systemDevice ?? null,
        });
        await configService.saveRecordingDevicePreferences(selected);
        setSelectedDevices(selected);
        return;
      }
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
    } catch (error) {
      setSwitchError(t(kind === 'microphone' ? 'Could not switch microphone: {error}' : 'Could not switch system audio: {error}', { error: String(error) }));
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
      notifyRecordingSourceMutesChanged(mutes);
    } catch (error) {
      setSwitchError(t(
        kind === 'microphone'
          ? (sourceMutes[kind] ? 'Could not enable microphone: {error}' : 'Could not mute microphone: {error}')
          : (sourceMutes[kind] ? 'Could not enable system audio: {error}' : 'Could not mute system audio: {error}'),
        { error: String(error) },
      ));
    } finally {
      setTogglingSource(null);
    }
  };

  if (!recording && !alwaysVisible) return null;
  const row = (kind: 'microphone' | 'system', label: string, name: string | null, options: AudioDevice[]) => {
    const level = name ? levels.get(name) : undefined;
    const isExpanded = expandedDevice === kind;
    const isMuted = sourceMutes[kind];
    const SourceIcon = kind === 'microphone' ? Mic : Headphones;
    return <div className="space-y-1" key={label}>
      <div className="flex items-center gap-1.5 text-xs font-medium">
      <button
        type="button"
        onClick={() => toggleSource(kind)}
        disabled={disabled || togglingSource !== null}
        className="relative inline-flex h-6 w-6 shrink-0 items-center justify-center rounded border border-slate-300 bg-white text-slate-700 transition-colors enabled:cursor-pointer enabled:hover:bg-slate-100 enabled:active:bg-slate-200 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-indigo-500 disabled:cursor-default disabled:opacity-50"
        aria-pressed={!isMuted}
        aria-label={isMuted ? t('Enable {source}', { source: label }) : t('Mute {source}', { source: label })}
        title={isMuted ? t('Enable {source}', { source: label }) : t('Mute {source}', { source: label })}
      >
        <span className="relative inline-flex shrink-0">
          {togglingSource === kind ? <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden="true" /> : <SourceIcon className="h-3.5 w-3.5" aria-hidden="true" />}
          {isMuted && <span className="absolute left-[-1px] top-1/2 h-0.5 w-[18px] -rotate-45 rounded-full bg-red-500" aria-hidden="true" />}
        </span>
        {!isMuted && <span className="absolute -right-0.5 -top-0.5 h-1.5 w-1.5 rounded-full bg-green-500 ring-1 ring-white" aria-hidden="true" />}
      </button>
      <span className="text-slate-800">{label}</span>
      </div>
      <button
        type="button"
        onClick={() => setExpandedDevice(isExpanded ? null : kind)}
        disabled={disabled || switching !== null}
        className="flex w-full items-center justify-between gap-2 rounded px-1 py-0.5 text-left text-xs text-slate-600 hover:bg-slate-200"
        aria-expanded={isExpanded}
      >
        <span className="truncate">{name || t('Default device')}</span>
        <ChevronDown className={`h-3.5 w-3.5 shrink-0 transition-transform ${isExpanded ? 'rotate-180' : ''}`} />
      </button>
      {isExpanded && <div className="border-t border-slate-200 pt-2">
        <div className="max-h-40 overflow-y-auto rounded-md border border-slate-200 bg-white p-1">
          {options.map((device) => <button
            key={device.name}
            type="button"
            disabled={disabled || switching !== null}
            onClick={() => switchDevice(kind, device.name)}
            className={`block w-full rounded px-2 py-1.5 text-left text-xs hover:bg-slate-100 disabled:opacity-50 ${device.name === name ? 'bg-slate-100 font-medium text-slate-900' : 'text-slate-700'}`}
          >{device.name}</button>)}
        </div>
        {switching === kind && <p className="mt-1 flex items-center gap-1 text-xs text-slate-500"><Loader2 className="h-3.5 w-3.5 animate-spin" /> {t('Switching…')}</p>}
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
    .map(({ provider, node_count }) => t('{provider}: {count} operations', { provider, count: node_count }))
    .join('\n');
  const isAccelerated = providerLabel?.startsWith('GPU') || providerLabel?.startsWith('CoreML');
  return <div className={`${dragHandleProps ? '' : 'mt-3'} space-y-2 rounded-md border border-slate-200 bg-slate-50 p-3`} aria-live="polite">
    <div {...dragHandleProps} className={`flex items-center justify-between gap-2 ${dragHandleProps?.className ?? ''}`}>
      {dragHandleProps && <GripHorizontal className="h-4 w-4 shrink-0 text-slate-400" aria-hidden="true" />}
      {collapsed ? <button
        type="button"
        onClick={() => setCollapsed(false)}
        className="text-left text-xs font-medium text-slate-700 hover:text-slate-900"
        aria-expanded="false"
      >
        {t('Live capture signal')}
      </button> : <p className="text-xs font-medium text-slate-700">{t('Live capture signal')}</p>}
      <div className="flex items-center gap-2">
        {!collapsed && recording && transcriptModelConfig.provider === 'gigaam' && <span
          className={`flex items-center gap-1 rounded-full px-2 py-0.5 text-xs font-medium ${
            isAccelerated
              ? 'bg-purple-100 text-purple-700'
              : providerLabel
              ? 'bg-slate-100 text-slate-600'
              : 'bg-slate-100 text-slate-400'
          }`}
          title={
            !providerLabel
              ? t('Detecting the active GigaAM execution mode…')
              : [
                  providerLabel.includes('+ CPU')
                    ? t('ONNX Runtime split the graph between {provider} and CPU', { provider: providerLabel.replace(' + CPU', '') })
                    : isAccelerated
                    ? t('All reported graph operations are assigned to {provider}', { provider: providerLabel })
                    : providerStatus?.fallback_reason || t('All reported graph operations are assigned to {provider}', { provider: 'CPU' }),
                  providerAssignments,
                ].filter(Boolean).join('\n')
          }
        >
          {isAccelerated
            ? <Zap className="h-3 w-3" aria-hidden="true" />
            : <Cpu className="h-3 w-3" aria-hidden="true" />}
          {providerLabel ?? t('Detecting…')}
        </span>}
        <button
          type="button"
          onClick={() => {
            setExpandedDevice(null);
            setCollapsed(previous => !previous);
          }}
          className="rounded p-0.5 text-slate-500 hover:bg-slate-200 hover:text-slate-800"
          aria-label={collapsed ? t('Expand live capture signal') : t('Collapse live capture signal')}
          aria-expanded={!collapsed}
          title={collapsed ? t('Expand') : t('Collapse')}
        >
          {collapsed ? <ChevronUp className="h-3.5 w-3.5" aria-hidden="true" /> : <ChevronDown className="h-3.5 w-3.5" aria-hidden="true" />}
        </button>
      </div>
    </div>
    {!collapsed && <>
      {row('microphone', t('Microphone'), mic, inputs)}
      {row('system', t('System audio'), system, outputs)}
      {switchError && <p className="mt-1 text-xs text-red-600">{switchError}</p>}
    </>}
  </div>;
}
