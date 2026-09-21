export type AudioDevicePreferences = {
  micDevice: string | null;
  systemDevice: string | null;
};

export const stripAudioDeviceSuffix = (value: string | null | undefined) =>
  value?.replace(/ \((input|output)\)$/i, '') || null;

const withAudioDeviceSuffix = (
  value: string | null | undefined,
  kind: 'input' | 'output',
) => {
  const name = stripAudioDeviceSuffix(value);
  return name ? `${name} (${kind})` : null;
};

/** Canonical format accepted by the Rust recording start path and device picker. */
export const normalizeAudioDevicePreferences = (
  devices: AudioDevicePreferences,
): AudioDevicePreferences => ({
  micDevice: withAudioDeviceSuffix(devices.micDevice, 'input'),
  systemDevice: withAudioDeviceSuffix(devices.systemDevice, 'output'),
});
