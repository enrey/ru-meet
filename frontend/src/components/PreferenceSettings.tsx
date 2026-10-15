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
  callApps: string[];
  recordAnyApp: boolean;
  recordInPerson: boolean;
  micHolders: { name: string; call: boolean }[];
}

type AppListKind = 'excludedApps' | 'callApps';
type AutomationToggle = 'autoRecordMeetings' | 'launchAtLogin' | 'recordAnyApp' | 'recordInPerson';

const IS_MAC = typeof navigator !== 'undefined' && navigator.userAgent.includes('Mac');

function AppListEditor({ id, label, help, placeholder, apps, disabled, onSave }: {
  id: string;
  label: string;
  help: string;
  placeholder: string;
  apps: string[];
  disabled: boolean;
  onSave: (apps: string[]) => Promise<boolean>;
}) {
  const { t } = useI18n();
  const [input, setInput] = useState('');
  return (
    <div className="space-y-3">
      <label htmlFor={id} className="block text-sm font-medium text-slate-900">{label}</label>
      <p id={`${id}-help`} className="text-sm text-slate-600">{help}</p>
      <form className="flex gap-2" onSubmit={async event => {
        event.preventDefault();
        if (input.trim() && await onSave([...apps, input])) setInput('');
      }}>
        <input
          id={id}
          aria-describedby={`${id}-help`}
          value={input}
          onChange={event => setInput(event.target.value)}
          placeholder={placeholder}
          disabled={disabled}
          className="min-w-0 flex-1 rounded-md border border-slate-300 px-3 py-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-indigo-600 disabled:opacity-50"
        />
        <button type="submit" disabled={disabled || !input.trim()} className="rounded-md border border-slate-300 px-3 py-2 text-sm hover:bg-slate-50 disabled:opacity-50 focus-visible:ring-2 focus-visible:ring-indigo-600">{t('Add')}</button>
      </form>
      {!!apps.length && (
        <ul className="space-y-2">
          {apps.map(name => (
            <li key={name} className="flex items-center justify-between gap-3 rounded-md bg-slate-50 px-3 py-2">
              <span className="text-sm break-all">{name}</span>
              <button type="button" aria-label={t('Remove {name} from the list', { name })} disabled={disabled} onClick={() => void onSave(apps.filter(app => app !== name))} className="text-sm text-slate-600 hover:text-slate-900 disabled:opacity-50 focus-visible:ring-2 focus-visible:ring-indigo-600">{t('Remove')}</button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
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
  const saveAppList = async (kind: AppListKind, apps: string[]) => {
    if (!automation || savingAutomation) return false;
    setSavingAutomation(true);
    try {
      const command = kind === 'excludedApps' ? 'set_auto_record_excluded_apps' : 'set_auto_record_call_apps';
      const saved = await invoke<string[]>(command, { apps });
      setAutomation(previous => previous && { ...previous, [kind]: saved });
      return true;
    } catch (error) {
      toast.error(t('Could not save the app list'), { description: String(error) });
      return false;
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

  const updateAutomation = async (key: AutomationToggle, enabled: boolean) => {
    if (!automation || savingAutomation) return;
    setSavingAutomation(true);
    try {
      if (key === 'autoRecordMeetings') {
        await invoke('set_auto_record_meetings', { enabled });
        setAutomation(previous => previous && { ...previous, autoRecordMeetings: enabled });
      } else if (key === 'recordAnyApp' || key === 'recordInPerson') {
        await invoke('set_auto_record_option', { option: key === 'recordAnyApp' ? 'anyApp' : 'inPerson', enabled });
        setAutomation(previous => previous && { ...previous, [key]: enabled });
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
      <div className="bg-white rounded-lg border border-slate-200 p-6 shadow-sm">
        <div className="flex items-center justify-between gap-6">
          <div>
            <div className="flex items-center gap-2 mb-2">
              <Languages size={18} className="text-slate-500" aria-hidden="true" />
              <h3 id="ui-language-label" className="text-lg font-semibold text-slate-900">{t('Interface language')}</h3>
            </div>
            <p className="text-sm text-slate-600">{t('Language of menus, buttons and messages in the app')}</p>
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
      <div className="bg-white rounded-lg border border-slate-200 p-6 shadow-sm">
        <div className="flex items-center justify-between">
          <div>
            <h3 id="notifications-label" className="text-lg font-semibold text-slate-900 mb-2">{t('Notifications')}</h3>
            <p className="text-sm text-slate-600">{t('Enable or disable notifications of start and end of meeting')}</p>
          </div>
          <Switch aria-labelledby="notifications-label" checked={notificationsEnabledValue} onCheckedChange={setNotificationsEnabled} />
        </div>
      </div>

      <div className="bg-white rounded-lg border border-slate-200 p-6 shadow-sm space-y-6">
        <div className="flex items-center justify-between gap-6">
          <div>
            <h3 id="auto-record-label" className="text-lg font-semibold text-slate-900 mb-2">{t('Record meetings automatically')}</h3>
            <p id="auto-record-description" className="text-sm text-slate-600">
              {t('Recording starts when a call app (Zoom, Teams, Telegram, a browser with Meet or Telemost, and others) starts using the microphone, and ends a little after it releases the microphone. Being muted in the call does not stop the recording. Ru-Meet does not listen to the microphone before the call. Recordings shorter than 1 minute or with almost no speech are not saved.')}
            </p>
            {automation && !automation.autoRecordSupported && (
              <p className="mt-2 text-sm text-slate-600">{IS_MAC ? t('Meeting detection requires macOS 14.4 or later.') : t('Meeting detection is not available on this platform.')}</p>
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
        {automation?.autoRecordSupported && (
          <p className="text-sm text-slate-600" role="status">
            {automation.micHolders.length
              ? t('Using the microphone now: {apps}', {
                  apps: automation.micHolders.map(holder => holder.call ? `${holder.name} (${t('call')})` : holder.name).join(', '),
                })
              : t('No app is using the microphone now.')}
          </p>
        )}
        <AppListEditor
          id="auto-record-call-apps"
          label={t('Additional call apps')}
          help={IS_MAC
            ? t('If your call app is not recognised, add its bundle id, for example com.example.calls. The apps using the microphone are listed above.')
            : t('If your call app is not recognised, add its .exe name, for example Calls.exe. The apps using the microphone are listed above.')}
          placeholder={IS_MAC ? 'com.example.calls' : 'Calls.exe'}
          apps={automation?.callApps ?? []}
          disabled={!automation?.autoRecordSupported || savingAutomation}
          onSave={apps => saveAppList('callApps', apps)}
        />
        <AppListEditor
          id="auto-record-exclusion"
          label={t('Excluded apps')}
          help={IS_MAC
            ? t('These apps never start a recording. Enter the bundle id, for example com.spotify.client. Exclusions do not remove audio from a recording already in progress.')
            : t('These apps never start a recording. Enter the .exe name, for example Spotify.exe. Exclusions do not remove audio from a recording already in progress.')}
          placeholder={IS_MAC ? 'com.spotify.client' : 'Spotify.exe'}
          apps={automation?.excludedApps ?? []}
          disabled={!automation?.autoRecordSupported || savingAutomation}
          onSave={apps => saveAppList('excludedApps', apps)}
        />
        <div className="flex items-center justify-between gap-6">
          <div>
            <h4 id="auto-record-any-app-label" className="text-sm font-medium text-slate-900">{t('Record any app that uses the microphone')}</h4>
            <p className="text-sm text-slate-600">{t('Count every app as a call app, not only the known ones. Dictation, audio utilities and excluded apps still do not count. Expect recordings you did not intend.')}</p>
          </div>
          <Switch
            aria-labelledby="auto-record-any-app-label"
            checked={automation?.recordAnyApp ?? false}
            disabled={!automation?.autoRecordSupported || savingAutomation}
            onCheckedChange={enabled => void updateAutomation('recordAnyApp', enabled)}
            className="shrink-0"
          />
        </div>
        <div className="flex items-center justify-between gap-6">
          <div>
            <h4 id="auto-record-in-person-label" className="text-sm font-medium text-slate-900">{IS_MAC ? t('Activate on microphone sound (legacy method)') : t('Activate on any sound in the system (legacy method)')}</h4>
            <p className="text-sm text-slate-600">
              {IS_MAC
                ? t('Start recording on confirmed speech in the microphone even without a call. The microphone stays open all the time while this is on.')
                : t('Start recording on confirmed speech in the microphone or in audio played by an app, even without a call. The microphone stays open all the time while this is on.')}
            </p>
          </div>
          <Switch
            aria-labelledby="auto-record-in-person-label"
            checked={automation?.recordInPerson ?? false}
            disabled={!automation?.autoRecordSupported || savingAutomation}
            onCheckedChange={enabled => void updateAutomation('recordInPerson', enabled)}
            className="shrink-0"
          />
        </div>
        <div className="flex items-center justify-between gap-6 border-t border-slate-200 pt-6">
          <div>
            <h3 id="launch-at-login-label" className="text-lg font-semibold text-slate-900 mb-2">{t('Launch at startup')}</h3>
            <p className="text-sm text-slate-600">{t('Start Ru-Meet automatically when you sign in.')}</p>
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
      <div className="bg-white rounded-lg border border-slate-200 p-6 shadow-sm">
        <h3 className="text-lg font-semibold text-slate-900 mb-4">{t('Data Storage Locations')}</h3>
        <p className="text-sm text-slate-600 mb-6">
          {t('View and access where Ru-Meet stores your data')}
        </p>

        <div className="space-y-4">
          {/* Database Location */}
          {/* <div className="p-4 border rounded-lg bg-slate-50">
            <div className="font-medium mb-2">Database</div>
            <div className="text-sm text-slate-600 mb-3 break-all font-mono text-xs">
              {storageLocations?.database || 'Loading...'}
            </div>
            <button
              onClick={() => handleOpenFolder('database')}
              className="flex items-center gap-2 px-3 py-2 text-sm border border-slate-300 rounded-md hover:bg-slate-100 transition-colors"
            >
              <FolderOpen className="w-4 h-4" />
              Open Folder
            </button>
          </div> */}

          {/* Models Location */}
          {/* <div className="p-4 border rounded-lg bg-slate-50">
            <div className="font-medium mb-2">Whisper Models</div>
            <div className="text-sm text-slate-600 mb-3 break-all font-mono text-xs">
              {storageLocations?.models || 'Loading...'}
            </div>
            <button
              onClick={() => handleOpenFolder('models')}
              className="flex items-center gap-2 px-3 py-2 text-sm border border-slate-300 rounded-md hover:bg-slate-100 transition-colors"
            >
              <FolderOpen className="w-4 h-4" />
              Open Folder
            </button>
          </div> */}

          {/* Recordings Location */}
          <div className="p-4 border rounded-lg bg-slate-50">
            <div className="font-medium mb-2">{t('Meeting Recordings')}</div>
            <div className="text-sm text-slate-600 mb-3 break-all font-mono text-xs">
              {storageLocations?.recordings || t('Loading...')}
            </div>
            <button
              onClick={() => handleOpenFolder('recordings')}
              className="flex items-center gap-2 px-3 py-2 text-sm border border-slate-300 rounded-md hover:bg-slate-100 transition-colors"
            >
              <FolderOpen className="w-4 h-4" />
              {t('Open Folder')}
            </button>
          </div>
        </div>

        <div className="mt-4 p-3 bg-indigo-50 rounded-md">
          <p className="text-xs text-indigo-800">
            <strong>{t('Note:')}</strong> {t('Database and models are stored together in your application data directory for unified management.')}
          </p>
        </div>
      </div>
    </div>
  )
}
