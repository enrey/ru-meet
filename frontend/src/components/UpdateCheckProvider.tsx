'use client'

import React, { createContext, useContext, useState, useCallback, useEffect } from 'react';
import { useUpdateCheck } from '@/hooks/useUpdateCheck';
import { toast } from 'sonner';
import { compareVersions, UpdateInfo } from '@/services/updateService';
import { fetchReleaseHistory } from './ReleaseNotes';
import { translate } from '@/lib/i18n';
import { UpdateDialog } from './UpdateDialog';
import { setUpdateDialogCallback, showUpdateNotification } from './UpdateNotification';

interface UpdateCheckContextType {
  updateInfo: UpdateInfo | null;
  isChecking: boolean;
  checkForUpdates: (force?: boolean) => Promise<void>;
  showUpdateDialog: () => void;
  /** Opens the update dialog with real notes of recent releases, as if the user skipped some. */
  showUpdatePreview: () => Promise<void>;
}

const UpdateCheckContext = createContext<UpdateCheckContextType | undefined>(undefined);

export function UpdateCheckProvider({ children }: { children: React.ReactNode }) {
  const [showDialog, setShowDialog] = useState(false);
  const [previewInfo, setPreviewInfo] = useState<UpdateInfo | null>(null);

  const handleShowDialog = useCallback(() => {
    setShowDialog(true);
  }, []);

  const { updateInfo, isChecking, checkForUpdates } = useUpdateCheck({
    checkOnMount: true,
    showNotification: true,
    onUpdateAvailable: (info) => {
      // Show notification, dialog will be shown when user clicks notification
      showUpdateNotification(info, handleShowDialog);
    },
  });

  const showUpdatePreview = useCallback(async () => {
    try {
      const history = await fetchReleaseHistory();
      if (history.length === 0) {
        toast.error(translate('No published releases found'));
        return;
      }
      const sorted = [...history].sort((a, b) => compareVersions(b.version, a.version));
      const latest = sorted[0];
      // Pretend the user is two releases behind so the dialog shows accumulated notes.
      const current = sorted[Math.min(2, sorted.length - 1)];
      setPreviewInfo({
        available: true,
        currentVersion: current.version === latest.version ? '0.0.0' : current.version,
        version: latest.version,
        date: latest.pub_date ?? undefined,
        body: latest.notes,
        releases: sorted.filter((entry) => compareVersions(entry.version, current.version) > 0),
      });
    } catch (error) {
      toast.error(translate('Failed to load release notes: {error}', { error: String(error) }));
    }
  }, []);

  useEffect(() => {
    // Register the callback so UpdateNotification can trigger the dialog
    setUpdateDialogCallback(handleShowDialog);
    return () => {
      setUpdateDialogCallback(() => {});
    };
  }, [handleShowDialog]);

  // Listen for tray menu events
  useEffect(() => {
    const handleTrayCheck = () => {
      checkForUpdates(true); // Force check from tray
      setShowDialog(true);
    };

    window.addEventListener('check-updates-from-tray', handleTrayCheck);
    return () => window.removeEventListener('check-updates-from-tray', handleTrayCheck);
  }, [checkForUpdates]);

  return (
    <UpdateCheckContext.Provider
      value={{
        updateInfo,
        isChecking,
        checkForUpdates,
        showUpdateDialog: handleShowDialog,
        showUpdatePreview,
      }}
    >
      {children}
      <UpdateDialog
        open={showDialog}
        onOpenChange={setShowDialog}
        updateInfo={updateInfo}
      />
      <UpdateDialog
        open={previewInfo !== null}
        onOpenChange={(open) => {
          if (!open) setPreviewInfo(null);
        }}
        updateInfo={previewInfo}
        preview
      />
    </UpdateCheckContext.Provider>
  );
}

export function useUpdateCheckContext() {
  const context = useContext(UpdateCheckContext);
  if (context === undefined) {
    throw new Error('useUpdateCheckContext must be used within UpdateCheckProvider');
  }
  return context;
}
