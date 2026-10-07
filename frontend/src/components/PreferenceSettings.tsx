"use client"

import { useEffect, useState } from "react"
import { Switch } from "./ui/switch"
import { FolderOpen } from "lucide-react"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { useConfig, NotificationSettings } from "@/contexts/ConfigContext"
import { toast } from "sonner"
import { Languages } from "lucide-react"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "./ui/select"
import { Locale, UI_LOCALES, useI18n } from "@/lib/i18n"

interface AutomationPreferences {
  autoRecordMeetings: boolean;
  launchAtLogin: boolean;
  autoRecordSupported: boolean;
  excludedApps: string[];
}

export function PreferenceSettings() {
  const {
    notificationSettings,
    storageLocations,
    isLoadingPreferences,
    loadPreferences,
    updateNotificationSettings
  } = useConfig();
  const { locale, setLocale, t } = useI18n();

  const [notificationsEnabled, setNotificationsEnabled] = useState<boolean | null>(null);
  const [isInitialLoad, setIsInitialLoad] = useState(true);
  const [previousNotificationsEnabled, setPreviousNotificationsEnabled] = useState<boolean | null>(null);
  const [automation, setAutomation] = useState<AutomationPreferences | null>(null);
  const [automationError, setAutomationError] = useState(false);
  const [savingAutomation, setSavingAutomation] = useState(false);
  const [excludedAppInput, setExcludedAppInput] = useState('');

  const saveExcludedApps = async (apps: string[]) => {
    if (!automation || savingAutomation) return;
    setSavingAutomation(true);
    try {
      const excludedApps = await invoke<string[]>('set_auto_record_excluded_apps', { apps });
      setAutomation(previous => previous && { ...previous, excludedApps });
      setExcludedAppInput('');
    } catch (error) {
      toast.error(t('Could not save exclusions'), { description: String(error) });
    } finally {
      setSavingAutomation(false);
    }
  };

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    let revision = 0;
    const load = async () => {
      const current = ++revision;
      try {
        const preferences = await invoke<AutomationPreferences>('get_automation_preferences');
        if (!cancelled && current === revision) { setAutomation(preferences); setAutomationError(false); }
      } catch (error) {
        console.error('Failed to load automation preferences:', error);
        if (!cancelled && current === revision) setAutomationError(true);
      }
    };
    listen<boolean>('auto-record-meetings-changed', () => { void load(); })
      .then(fn => { if (cancelled) fn(); else { unlisten = fn; void load(); } })
      .catch(() => { void load(); });
    return () => { cancelled = true; unlisten?.(); };
  }, []);

  const updateAutomation = async (key: 'autoRecordMeetings' | 'launchAtLogin', enabled: boolean) => {
    if (!automation || savingAutomation) return;
    setSavingAutomation(true);
    try {
      if (key === 'autoRecordMeetings') {
        await invoke('set_auto_record_meetings', { enabled });
        setAutomation(previous => previous && { ...previous, autoRecordMeetings: enabled });
      } else {
        const actual = await invoke<boolean>('set_launch_at_login', { enabled });
        setAutomation(previous => previous && { ...previous, launchAtLogin: actual });
      }
    } catch (error) {
      toast.error(t('Could not save preference'), { description: String(error) });
    } finally {
      setSavingAutomation(false);
    }
  };

  // Lazy load preferences on mount (only loads if not already cached)
  useEffect(() => {
    loadPreferences();
  }, [loadPreferences]);

  // Update notificationsEnabled when notificationSettings are loaded from global state
  useEffect(() => {
    if (notificationSettings) {
      // Notification enabled means both started and stopped notifications are enabled
      const enabled =
        notificationSettings.notification_preferences.show_recording_started &&
        notificationSettings.notification_preferences.show_recording_stopped;
      setNotificationsEnabled(enabled);
      if (isInitialLoad) {
        setPreviousNotificationsEnabled(enabled);
        setIsInitialLoad(false);
      }
    } else if (!isLoadingPreferences) {
      // If not loading and no settings, use default
      setNotificationsEnabled(true);
      if (isInitialLoad) {
        setPreviousNotificationsEnabled(true);
        setIsInitialLoad(false);
      }
    }
  }, [notificationSettings, isLoadingPreferences, isInitialLoad])

  useEffect(() => {
    // Skip update on initial load or if value hasn't actually changed
    if (isInitialLoad || notificationsEnabled === null || notificationsEnabled === previousNotificationsEnabled) return;
    if (!notificationSettings) return;

    const handleUpdateNotificationSettings = async () => {
      console.log("Updating notification settings to:", notificationsEnabled);

      try {
        // Update the notification preferences
        const updatedSettings: NotificationSettings = {
          ...notificationSettings,
          notification_preferences: {
            ...notificationSettings.notification_preferences,
            show_recording_started: notificationsEnabled,
            show_recording_stopped: notificationsEnabled,
          }
        };

        console.log("Calling updateNotificationSettings with:", updatedSettings);
        await updateNotificationSettings(updatedSettings);
        setPreviousNotificationsEnabled(notificationsEnabled);
        console.log("Successfully updated notification settings to:", notificationsEnabled);
      } catch (error) {
        console.error('Failed to update notification settings:', error);
      }
    };

    handleUpdateNotificationSettings();
  }, [notificationsEnabled, notificationSettings, isInitialLoad, previousNotificationsEnabled, updateNotificationSettings])

  const handleOpenFolder = async (folderType: 'database' | 'models' | 'recordings') => {
    try {
      switch (folderType) {
        case 'database':
          await invoke('open_database_folder');
          break;
        case 'models':
          await invoke('open_models_folder');
          break;
        case 'recordings':
          await invoke('open_recordings_folder');
          break;
      }
    } catch (error) {
      console.error(`Failed to open ${folderType} folder:`, error);
    }
  };

  // Show loading only if we're actually loading and don't have cached data
  if (isLoadingPreferences && !notificationSettings && !storageLocations) {
    return <div className="max-w-2xl mx-auto p-6">{t('Loading preferences...')}</div>
  }

  // Show loading if notificationsEnabled hasn't been determined yet
  if (notificationsEnabled === null && !isLoadingPreferences) {
    return <div className="max-w-2xl mx-auto p-6">{t('Loading preferences...')}</div>
  }

  // Ensure we have a boolean value for the Switch component
  const notificationsEnabledValue = notificationsEnabled ?? false;

  return (
    <div className="space-y-6">
      {/* Interface Language Section */}
      <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm">
        <div className="flex items-center justify-between gap-6">
          <div>
            <div className="flex items-center gap-2 mb-2">
              <Languages size={18} className="text-gray-500" aria-hidden="true" />
              <h3 id="ui-language-label" className="text-lg font-semibold text-gray-900">{t('Interface language')}</h3>
            </div>
            <p className="text-sm text-gray-600">{t('Language of menus, buttons and messages in the app')}</p>
          </div>
          <Select value={locale} onValueChange={value => setLocale(value as Locale)}>
            <SelectTrigger aria-labelledby="ui-language-label" className="w-44 shrink-0">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {UI_LOCALES.map(option => (
                <SelectItem key={option.value} value={option.value}>{option.label}</SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      </div>

      {/* Notifications Section */}
      <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm">
        <div className="flex items-center justify-between">
          <div>
            <h3 id="notifications-label" className="text-lg font-semibold text-gray-900 mb-2">{t('Notifications')}</h3>
            <p className="text-sm text-gray-600">{t('Enable or disable notifications of start and end of meeting')}</p>
          </div>
          <Switch aria-labelledby="notifications-label" checked={notificationsEnabledValue} onCheckedChange={setNotificationsEnabled} />
        </div>
      </div>

      <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm space-y-6">
        <div className="flex items-center justify-between gap-6">
          <div>
            <h3 id="auto-record-label" className="text-lg font-semibold text-gray-900 mb-2">{t('Record meetings automatically')}</h3>
            <p id="auto-record-description" className="text-sm text-gray-600">
              {t('Start recording automatically when an app other than the exclusions plays audio or uses a microphone, or when sound is detected on the selected microphone. Meetings shorter than 1 minute are ignored.')}
            </p>
            {automation && !automation.autoRecordSupported && (
              <p className="mt-2 text-sm text-gray-600">{t('Meeting detection is currently available only on Windows.')}</p>
            )}
          </div>
          <Switch
            aria-labelledby="auto-record-label"
            aria-describedby="auto-record-description"
            checked={automation?.autoRecordMeetings ?? true}
            disabled={!automation || !automation.autoRecordSupported || savingAutomation}
            onCheckedChange={enabled => void updateAutomation('autoRecordMeetings', enabled)}
            className="shrink-0"
          />
        </div>
        <div className="space-y-3">
          <label htmlFor="auto-record-exclusion" className="block text-sm font-medium text-gray-900">{t('Excluded apps')}</label>
          <p id="auto-record-exclusion-help" className="text-sm text-gray-600">
            {t('Audio from these apps never starts a recording. Enter the .exe name, for example Spotify.exe. Meetily audio is always excluded. Exclusions do not remove audio from a recording already in progress.')}
          </p>
          <form className="flex gap-2" onSubmit={event => {
            event.preventDefault();
            if (automation && excludedAppInput.trim()) void saveExcludedApps([...automation.excludedApps, excludedAppInput]);
          }}>
            <input
              id="auto-record-exclusion"
              aria-describedby="auto-record-exclusion-help"
              value={excludedAppInput}
              onChange={event => setExcludedAppInput(event.target.value)}
              placeholder="Spotify.exe"
              disabled={!automation?.autoRecordSupported || savingAutomation}
              className="min-w-0 flex-1 rounded-md border border-gray-300 px-3 py-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-600 disabled:opacity-50"
            />
            <button type="submit" disabled={!automation?.autoRecordSupported || savingAutomation || !excludedAppInput.trim()} className="rounded-md border border-gray-300 px-3 py-2 text-sm hover:bg-gray-50 disabled:opacity-50 focus-visible:ring-2 focus-visible:ring-blue-600">{t('Add')}</button>
          </form>
          {!!automation?.excludedApps.length && (
            <ul className="space-y-2">
              {automation.excludedApps.map(name => (
                <li key={name} className="flex items-center justify-between gap-3 rounded-md bg-gray-50 px-3 py-2">
                  <span className="text-sm break-all">{name}</span>
                  <button type="button" aria-label={t('Remove {name} from exclusions', { name })} disabled={savingAutomation} onClick={() => void saveExcludedApps(automation.excludedApps.filter(app => app !== name))} className="text-sm text-gray-600 hover:text-gray-900 disabled:opacity-50 focus-visible:ring-2 focus-visible:ring-blue-600">{t('Remove')}</button>
                </li>
              ))}
            </ul>
          )}
        </div>
        <div className="flex items-center justify-between gap-6 border-t border-gray-200 pt-6">
          <div>
            <h3 id="launch-at-login-label" className="text-lg font-semibold text-gray-900 mb-2">{t('Launch at startup')}</h3>
            <p className="text-sm text-gray-600">{t('Start Meetily automatically when you sign in.')}</p>
          </div>
          <Switch
            aria-labelledby="launch-at-login-label"
            checked={automation?.launchAtLogin ?? false}
            disabled={!automation || savingAutomation}
            onCheckedChange={enabled => void updateAutomation('launchAtLogin', enabled)}
            className="shrink-0"
          />
        </div>
        {automationError && <p role="alert" className="text-sm text-red-600">{t('Could not load automation settings.')}</p>}
      </div>

      {/* Data Storage Locations Section */}
      <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm">
        <h3 className="text-lg font-semibold text-gray-900 mb-4">{t('Data Storage Locations')}</h3>
        <p className="text-sm text-gray-600 mb-6">
          {t('View and access where Meetily stores your data')}
        </p>

        <div className="space-y-4">
          {/* Database Location */}
          {/* <div className="p-4 border rounded-lg bg-gray-50">
            <div className="font-medium mb-2">Database</div>
            <div className="text-sm text-gray-600 mb-3 break-all font-mono text-xs">
              {storageLocations?.database || 'Loading...'}
            </div>
            <button
              onClick={() => handleOpenFolder('database')}
              className="flex items-center gap-2 px-3 py-2 text-sm border border-gray-300 rounded-md hover:bg-gray-100 transition-colors"
            >
              <FolderOpen className="w-4 h-4" />
              Open Folder
            </button>
          </div> */}

          {/* Models Location */}
          {/* <div className="p-4 border rounded-lg bg-gray-50">
            <div className="font-medium mb-2">Whisper Models</div>
            <div className="text-sm text-gray-600 mb-3 break-all font-mono text-xs">
              {storageLocations?.models || 'Loading...'}
            </div>
            <button
              onClick={() => handleOpenFolder('models')}
              className="flex items-center gap-2 px-3 py-2 text-sm border border-gray-300 rounded-md hover:bg-gray-100 transition-colors"
            >
              <FolderOpen className="w-4 h-4" />
              Open Folder
            </button>
          </div> */}

          {/* Recordings Location */}
          <div className="p-4 border rounded-lg bg-gray-50">
            <div className="font-medium mb-2">{t('Meeting Recordings')}</div>
            <div className="text-sm text-gray-600 mb-3 break-all font-mono text-xs">
              {storageLocations?.recordings || t('Loading...')}
            </div>
            <button
              onClick={() => handleOpenFolder('recordings')}
              className="flex items-center gap-2 px-3 py-2 text-sm border border-gray-300 rounded-md hover:bg-gray-100 transition-colors"
            >
              <FolderOpen className="w-4 h-4" />
              {t('Open Folder')}
            </button>
          </div>
        </div>

        <div className="mt-4 p-3 bg-blue-50 rounded-md">
          <p className="text-xs text-blue-800">
            <strong>{t('Note:')}</strong> {t('Database and models are stored together in your application data directory for unified management.')}
          </p>
        </div>
      </div>
    </div>
  )
}
