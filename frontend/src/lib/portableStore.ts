import { invoke } from '@tauri-apps/api/core';
import { join } from '@tauri-apps/api/path';
import { load as tauriLoad } from '@tauri-apps/plugin-store';

// The Rust command returns the app's portable data directory when portable.json
// is present, and the normal Tauri app data directory for installed builds.
export async function load(
  name: string,
  options?: Parameters<typeof tauriLoad>[1],
): ReturnType<typeof tauriLoad> {
  const dataDir = await invoke<string>('get_database_directory');
  return tauriLoad(await join(dataDir, name), options);
}

export const Store = { load };
