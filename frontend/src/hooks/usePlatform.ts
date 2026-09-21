import { useState, useEffect } from 'react';

export type Platform = 'macos' | 'windows' | 'linux' | 'unknown';

// Extend Window type to include Tauri internals. __TAURI_OS_PLUGIN_INTERNALS__
// (used below) is already declared by @tauri-apps/plugin-os itself - redeclaring
// it here conflicts with that (non-optional, concretely typed) declaration.
declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
  }
}

/**
 * Detect platform from user agent (fallback method)
 */
function detectPlatformFromUserAgent(): Platform {
  if (typeof navigator === 'undefined') return 'unknown';

  const userAgent = navigator.userAgent.toLowerCase();
  if (userAgent.includes('mac')) {
    return 'macos';
  } else if (userAgent.includes('win')) {
    return 'windows';
  } else if (userAgent.includes('linux')) {
    return 'linux';
  }
  return 'unknown';
}

/**
 * Hook to detect the current platform
 * Uses Tauri's OS plugin if available, falls back to user agent detection
 * @returns The current platform
 */
export function usePlatform(): Platform {
  const [currentPlatform, setCurrentPlatform] = useState<Platform>(() => detectPlatformFromUserAgent());

  useEffect(() => {
    async function detectPlatform() {
      // Check if Tauri is available
      if (typeof window === 'undefined' || !window.__TAURI_INTERNALS__) {
        // Not in Tauri environment, use user agent
        setCurrentPlatform(detectPlatformFromUserAgent());
        return;
      }

      try {
        // __TAURI_INTERNALS__ (checked above) is the core IPC bridge and is
        // injected first; each plugin injects its own bridge separately a
        // moment later - the OS plugin's platform() reads
        // window.__TAURI_OS_PLUGIN_INTERNALS__.platform directly (see
        // @tauri-apps/plugin-os's dist-js/index.js), which throws
        // "Cannot read properties of undefined" if called in that gap.
        // Poll briefly for it instead of racing, so losing that (usually
        // sub-frame) race doesn't produce a scary-looking but harmless
        // TypeError on every normal startup.
        for (let attempt = 0; attempt < 10 && !window.__TAURI_OS_PLUGIN_INTERNALS__; attempt++) {
          await new Promise((resolve) => setTimeout(resolve, 20));
        }
        if (!window.__TAURI_OS_PLUGIN_INTERNALS__) {
          // Genuinely never showed up (not just lost the race) - fall back
          // quietly; this isn't actionable for the user and isn't a "real"
          // failure the way an actual plugin error would be.
          setCurrentPlatform(detectPlatformFromUserAgent());
          return;
        }

        // Dynamically import to avoid SSR issues
        const { platform } = await import('@tauri-apps/plugin-os');
        const platformName = await platform();

        // Map Tauri's platform names to our simplified types
        switch (platformName) {
          case 'macos':
          case 'ios':
            setCurrentPlatform('macos');
            break;
          case 'windows':
            setCurrentPlatform('windows');
            break;
          case 'linux':
          case 'android':
            setCurrentPlatform('linux');
            break;
          default:
            setCurrentPlatform('unknown');
        }
      } catch (error) {
        console.warn('[usePlatform] Tauri platform detection failed, using user agent:', error);
        setCurrentPlatform(detectPlatformFromUserAgent());
      }
    }

    detectPlatform();
  }, []);

  return currentPlatform;
}

/**
 * Simple helper to check if the current platform is Linux
 * @returns true if running on Linux
 */
export function useIsLinux(): boolean {
  const currentPlatform = usePlatform();
  return currentPlatform === 'linux';
}
