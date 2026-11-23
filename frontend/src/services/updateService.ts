/**
 * Update Service
 *
 * Handles automatic software updates using Tauri updater plugin.
 * Provides update checking, downloading, and installation functionality.
 */

import { check, Update } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import { getVersion } from '@tauri-apps/api/app';
import { invoke } from '@tauri-apps/api/core';

/** Notes of one published release, as listed in `latest.json` `releases`. */
export interface ReleaseNotesEntry {
  version: string;
  pub_date?: string | null;
  notes: string;
}

export interface UpdateInfo {
  available: boolean;
  currentVersion: string;
  version?: string;
  date?: string;
  body?: string;
  downloadUrl?: string;
  /** Notes of every release newer than the current version, newest first. */
  releases?: ReleaseNotesEntry[];
}

function parseVersion(version: string): number[] {
  return version
    .replace(/^v/, '')
    .split(/[.+-]/)
    .slice(0, 3)
    .map((part) => Number.parseInt(part, 10) || 0);
}

export function compareVersions(a: string, b: string): number {
  const left = parseVersion(a);
  const right = parseVersion(b);
  for (let i = 0; i < 3; i++) {
    const diff = (left[i] ?? 0) - (right[i] ?? 0);
    if (diff !== 0) return diff;
  }
  return 0;
}

function isReleaseNotesEntry(entry: unknown): entry is ReleaseNotesEntry {
  const candidate = entry as Partial<ReleaseNotesEntry> | null;
  return typeof candidate?.version === 'string' && typeof candidate?.notes === 'string';
}

/** Keeps the `latest.json` history for the About dialog, so it needs no network of its own. */
function cacheReleaseHistory(history: unknown) {
  if (!Array.isArray(history)) return;
  const releases = history.filter(isReleaseNotesEntry);
  if (releases.length === 0) return;
  invoke('save_release_history', { releases }).catch((error) =>
    console.warn('Failed to cache release notes:', error),
  );
}

/**
 * Releases the user skipped: newer than `currentVersion`, up to `latestVersion`.
 * `history` is the `releases` array that the release-notes workflow writes into
 * `latest.json`; without it only the latest release's own notes are shown.
 */
export function pendingReleaseNotes(
  history: unknown,
  currentVersion: string,
  latest: ReleaseNotesEntry,
): ReleaseNotesEntry[] {
  const entries = Array.isArray(history) ? history.filter(isReleaseNotesEntry) : [];
  const pending = entries
    .filter(
      (entry) =>
        compareVersions(entry.version, currentVersion) > 0 &&
        compareVersions(entry.version, latest.version) <= 0,
    )
    .sort((a, b) => compareVersions(b.version, a.version));
  if (!pending.some((entry) => compareVersions(entry.version, latest.version) === 0)) {
    pending.unshift(latest);
  }
  return pending.filter((entry) => entry.notes.trim().length > 0);
}

export interface UpdateProgress {
  downloaded: number;
  total: number;
  percentage: number;
}

/**
 * Update Service
 * Singleton service for managing app updates
 */
export class UpdateService {
  private updateCheckInProgress = false;
  private lastCheckTime: number | null = null;
  private readonly CHECK_INTERVAL_MS = 24 * 60 * 60 * 1000; // 24 hours

  /**
   * Check for available updates
   * @param force Force check even if recently checked
   * @returns Promise with update information
   */
  async checkForUpdates(force = false): Promise<UpdateInfo> {
    // Prevent concurrent update checks
    if (this.updateCheckInProgress) {
      throw new Error('Update check already in progress');
    }

    // Skip if checked recently (unless forced)
    if (!force && this.lastCheckTime) {
      const timeSinceLastCheck = Date.now() - this.lastCheckTime;
      if (timeSinceLastCheck < this.CHECK_INTERVAL_MS) {
        console.log('Skipping update check - checked recently');
        return {
          available: false,
          currentVersion: await getVersion(),
        };
      }
    }

    this.updateCheckInProgress = true;
    this.lastCheckTime = Date.now();

    try {
      const currentVersion = await getVersion();
      const update = await check();

      if (update?.available) {
        cacheReleaseHistory(update.rawJson?.releases);
        return {
          available: true,
          currentVersion,
          version: update.version,
          date: update.date,
          body: update.body,
          releases: pendingReleaseNotes(update.rawJson?.releases, currentVersion, {
            version: update.version,
            pub_date: update.date,
            notes: update.body ?? '',
          }),
        };
      }

      return {
        available: false,
        currentVersion,
      };
    } catch (error) {
      console.error('Failed to check for updates:', error);
      throw error;
    } finally {
      this.updateCheckInProgress = false;
    }
  }

  /**
   * Download and install the available update
   * @param update The update object from checkForUpdates
   * @param onProgress Optional progress callback
   * @returns Promise that resolves when download completes
   */
  async downloadAndInstall(
    update: Update,
    onProgress?: (progress: UpdateProgress) => void
  ): Promise<void> {
    try {
      // Download the update
      await update.download();

      // Notify progress if callback provided
      if (onProgress) {
        onProgress({ downloaded: 100, total: 100, percentage: 100 });
      }

      // Install and relaunch
      await update.install();
      await relaunch();
    } catch (error) {
      console.error('Failed to download/install update:', error);
      throw error;
    }
  }

  /**
   * Get the current app version
   * @returns Promise with version string
   */
  async getCurrentVersion(): Promise<string> {
    return getVersion();
  }

  /**
   * Check if an update check was performed recently
   * @returns true if checked within the interval
   */
  wasCheckedRecently(): boolean {
    if (!this.lastCheckTime) return false;
    const timeSinceLastCheck = Date.now() - this.lastCheckTime;
    return timeSinceLastCheck < this.CHECK_INTERVAL_MS;
  }
}

// Export singleton instance
export const updateService = new UpdateService();
