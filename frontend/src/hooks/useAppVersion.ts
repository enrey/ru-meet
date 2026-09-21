import { useEffect, useState } from 'react';
import { getVersion } from '@tauri-apps/api/app';

export function useAppVersion() {
  const [version, setVersion] = useState<string | null>(null);

  useEffect(() => {
    getVersion().then(setVersion).catch(console.error);
  }, []);

  return version;
}
