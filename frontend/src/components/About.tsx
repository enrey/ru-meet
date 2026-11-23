import React, { useEffect, useState } from "react";
import Image from 'next/image';
import { UpdateDialog } from "./UpdateDialog";
import { updateService, UpdateInfo, ReleaseNotesEntry } from '@/services/updateService';
import { Button } from './ui/button';
import { Loader2, CheckCircle2, Check, RefreshCw } from 'lucide-react';
import { fetchReleaseHistory, getCachedReleaseHistory, ReleaseNotesList } from './ReleaseNotes';
import { toast } from 'sonner';
import { useAppVersion } from '@/hooks/useAppVersion';
import { useI18n } from '@/lib/i18n';

const RECENT_RELEASES = 5;

const ADVANTAGES = [
    { title: 'Privacy-first', detail: 'Data and AI processing stay on your computer' },
    { title: 'Use Any Model', detail: 'Local models or any external API' },
    { title: 'Cost-Smart', detail: 'No per-minute billing' },
    { title: 'Works everywhere', detail: 'Google Meet, Zoom, Teams-online or offline.' },
];

export function About() {
    const currentVersion = useAppVersion();
    const { t } = useI18n();
    const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
    const [isChecking, setIsChecking] = useState(false);
    const [showUpdateDialog, setShowUpdateDialog] = useState(false);
    const [releases, setReleases] = useState<ReleaseNotesEntry[] | null>(null);
    const [isFetchingReleases, setIsFetchingReleases] = useState(false);

    // Only the local cache on open: the app goes online for release notes on request.
    useEffect(() => {
        let cancelled = false;
        getCachedReleaseHistory()
            .then((history) => {
                if (!cancelled) setReleases(history.slice(0, RECENT_RELEASES));
            })
            .catch((error) => {
                console.warn('Failed to read cached release notes:', error);
                if (!cancelled) setReleases([]);
            });
        return () => {
            cancelled = true;
        };
    }, []);

    const handleFetchReleases = async () => {
        setIsFetchingReleases(true);
        try {
            const history = await fetchReleaseHistory();
            setReleases(history.slice(0, RECENT_RELEASES));
        } catch (error) {
            console.warn('Failed to load release notes:', error);
            toast.error(t('Failed to load release notes: {error}', { error: String(error) }));
        } finally {
            setIsFetchingReleases(false);
        }
    };

    const handleCheckForUpdates = async () => {
        setIsChecking(true);
        try {
            const info = await updateService.checkForUpdates(true);
            setUpdateInfo(info);
            if (info.available) {
                setShowUpdateDialog(true);
            } else {
                toast.success(t('You are running the latest version'));
            }
        } catch (error: any) {
            console.error('Failed to check for updates:', error);
            toast.error(t('Failed to check for updates: {error}', { error: error.message || t('Unknown error') }));
        } finally {
            setIsChecking(false);
        }
    };

    return (
        <div className="p-4 space-y-4 h-[80vh] overflow-y-auto">
            {/* Compact Header */}
            <div className="text-center">
                <div className="mb-3">
                    <Image
                        src="icon_128x128.png"
                        alt="Ru-Meet Logo"
                        width={64}
                        height={64}
                        className="mx-auto"
                    />
                </div>
                <h1 className="text-xl font-bold text-slate-900">
                    Ru-Meet
                    {currentVersion && (
                        <span className="ml-2 text-sm font-normal text-slate-500">v{currentVersion}</span>
                    )}
                </h1>
                <p className="text-medium text-slate-600 mt-1">
                    {t('Real-time notes and summaries that never leave your machine.')}
                </p>
                <div className="mt-3">
                    <Button
                        onClick={handleCheckForUpdates}
                        disabled={isChecking}
                        variant="outline"
                        size="sm"
                        className="text-xs"
                    >
                        {isChecking ? (
                            <>
                                <Loader2 className="h-3 w-3 mr-2 animate-spin" />
                                {t('Checking...')}
                            </>
                        ) : (
                            <>
                                <CheckCircle2 className="h-3 w-3 mr-2" />
                                {t('Check for Updates')}
                            </>
                        )}
                    </Button>
                    {updateInfo?.available && (
                        <div className="mt-2 text-xs text-indigo-600">
                            {t('Update available: v{version}', { version: updateInfo.version ?? '' })}
                        </div>
                    )}
                </div>
            </div>

            {/* Advantages */}
            <div className="space-y-2">
                <h2 className="text-sm font-semibold text-slate-800">{t('Ru-Meet advantages')}</h2>
                <ul className="space-y-1.5">
                    {ADVANTAGES.map(({ title, detail }) => (
                        <li key={title} className="flex items-start gap-2 text-xs">
                            <span className="mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded-full bg-emerald-100 text-emerald-600">
                                <Check className="h-3 w-3" strokeWidth={3} />
                            </span>
                            <span className="leading-snug">
                                <span className="font-semibold text-slate-900">{t(title)}</span>
                                <span className="block text-slate-500">{t(detail)}</span>
                            </span>
                        </li>
                    ))}
                </ul>
            </div>

            {/* Release notes */}
            <div className="space-y-2">
                <div className="flex items-center justify-between gap-2">
                    <h2 className="text-sm font-semibold text-slate-800">{t("What's new")}</h2>
                    {releases && (
                        <Button
                            onClick={handleFetchReleases}
                            disabled={isFetchingReleases}
                            variant="ghost"
                            size="sm"
                            className="h-7 text-xs text-slate-500"
                        >
                            {isFetchingReleases ? (
                                <Loader2 className="h-3 w-3 mr-1.5 animate-spin" />
                            ) : (
                                <RefreshCw className="h-3 w-3 mr-1.5" />
                            )}
                            {t('Load from GitHub')}
                        </Button>
                    )}
                </div>
                {releases && releases.length === 0 && (
                    <p className="text-xs text-slate-500">
                        {t('Release notes are saved when the app checks for updates. You can also load them from GitHub.')}
                    </p>
                )}
                {releases && releases.length > 0 && (
                    <div className="rounded-lg border border-slate-200 bg-slate-50 p-4">
                        <ReleaseNotesList releases={releases} installedVersion={currentVersion} />
                    </div>
                )}
            </div>

            {/* Repository Link */}
            <div className="text-center">
                <a
                    href="https://github.com/enrey/ru-meet"
                    target="_blank"
                    rel="noopener noreferrer"
                    className="text-xs text-indigo-600 hover:underline"
                >
                    github.com/enrey/ru-meet
                </a>
            </div>

            {/* Update Dialog */}
            <UpdateDialog
                open={showUpdateDialog}
                onOpenChange={setShowUpdateDialog}
                updateInfo={updateInfo}
            />
        </div>

    )
}
