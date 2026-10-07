'use client';

import React from 'react';
import { Home, Info as InfoIcon, Library, Loader2, Mic, PanelLeftClose, PanelLeftOpen, Settings, Square, Unplug, Upload } from 'lucide-react';
import { useRouter, usePathname } from 'next/navigation';
import { useSidebar } from './SidebarProvider';
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip';
import { Dialog, DialogContent, DialogTitle, DialogTrigger } from '@/components/ui/dialog';
import { VisuallyHidden } from '@/components/ui/visually-hidden';
import { RecordingStatus, useRecordingState } from '@/contexts/RecordingStateContext';
import { recordingService } from '@/services/recordingService';
import { toast } from 'sonner';
import { useImportDialog } from '@/contexts/ImportDialogContext';
import { useConfig } from '@/contexts/ConfigContext';
import { useTranscriptRecoveryDialog } from '@/contexts/TranscriptRecoveryContext';
import { useI18n } from '@/lib/i18n';
import { About } from '../About';
import Logo from '../Logo';

interface NavItemProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  icon: React.ReactNode;
  label: string;
  collapsed: boolean;
  active?: boolean;
  /** Second line under the label (expanded) and in the tooltip (collapsed). */
  detail?: React.ReactNode;
  /** Small trailing marker: a count, a live dot. */
  trailing?: React.ReactNode;
  /** Dot drawn on the icon corner so state stays visible when collapsed. */
  iconBadge?: React.ReactNode;
}

const NavItem = React.forwardRef<HTMLButtonElement, NavItemProps>(
  ({ icon, label, collapsed, active = false, detail, trailing, iconBadge, className = '', ...props }, ref) => {
    const button = (
      <button
        ref={ref}
        type="button"
        aria-label={collapsed ? label : undefined}
        aria-current={active ? 'page' : undefined}
        className={`flex h-11 shrink-0 items-center rounded-xl transition-colors duration-150 ${
          collapsed ? 'w-11 justify-center' : 'w-full gap-3 px-3 text-left'
        } ${active ? 'bg-indigo-50 text-indigo-700' : 'text-gray-600 hover:bg-gray-100 hover:text-gray-900'} ${className}`}
        {...props}
      >
        <span className="relative grid shrink-0 place-items-center">
          {icon}
          {iconBadge}
        </span>
        {!collapsed && (
          <>
            <span className="min-w-0 flex-1">
              <span className="block truncate text-sm font-medium">{label}</span>
              {detail && <span className="block truncate text-xs text-gray-500">{detail}</span>}
            </span>
            {trailing}
          </>
        )}
      </button>
    );

    if (!collapsed) return button;
    return (
      <Tooltip>
        <TooltipTrigger asChild>{button}</TooltipTrigger>
        <TooltipContent side="right">
          <p>{label}</p>
          {detail && <p className="text-xs opacity-80">{detail}</p>}
        </TooltipContent>
      </Tooltip>
    );
  },
);
NavItem.displayName = 'NavItem';

function formatClock(seconds: number | null): string {
  const total = Math.max(0, Math.floor(seconds ?? 0));
  const pad = (value: number) => value.toString().padStart(2, '0');
  const hours = Math.floor(total / 3600);
  const clock = `${pad(Math.floor((total % 3600) / 60))}:${pad(total % 60)}`;
  return hours > 0 ? `${hours}:${clock}` : clock;
}

/** Left navigation: icon rail when collapsed, icons with labels when expanded. */
const Sidebar: React.FC = () => {
  const router = useRouter();
  const pathname = usePathname();
  const { t } = useI18n();
  const { handleRecordingToggle, isCollapsed, toggleCollapse, meetings } = useSidebar();
  const recordingState = useRecordingState();
  const { isRecording, isPaused, activeDuration } = recordingState;
  const stopPending = React.useRef(false);
  const recordingBusy = recordingState.isStartingRecording || recordingState.isStopping || recordingState.isProcessing || recordingState.isSaving;
  const toggleRecording = async () => {
    if (recordingBusy || stopPending.current) return;
    if (!isRecording) { handleRecordingToggle(); return; }
    stopPending.current = true;
    recordingState.setStatus(RecordingStatus.STOPPING);
    try {
      const result = await recordingService.stopRecording();
      if (!result) recordingState.setStatus(RecordingStatus.IDLE);
    } catch (error) {
      recordingState.setStatus(RecordingStatus.ERROR, String(error));
      toast.error(t('Failed to stop recording'), { description: String(error) });
    } finally {
      stopPending.current = false;
    }
  };
  const { openImportDialog } = useImportDialog();
  const { betaFeatures } = useConfig();
  const { recoverableCount, openRecoveryDialog } = useTranscriptRecoveryDialog();

  const isLibraryPage = pathname === '/meetings' || pathname?.startsWith('/meeting-details');
  const recordingDetail = isRecording
    ? `${isPaused ? t('Paused') : t('Recording in progress')} · ${formatClock(activeDuration)}`
    : undefined;

  return (
    <TooltipProvider>
      <nav
        className={`fixed top-0 left-0 z-40 flex h-screen flex-col gap-1 border-r border-gray-200 bg-white py-4 transition-[width] duration-300 ${
          isCollapsed ? 'w-16 items-center' : 'w-64 px-3'
        }`}
      >
        <div className={isCollapsed ? '' : 'px-1'}>
          <Logo isCollapsed={isCollapsed} />
        </div>

        <NavItem
          collapsed={isCollapsed}
          icon={<Home className="h-5 w-5" />}
          label={t('Home')}
          active={pathname === '/'}
          detail={recordingDetail}
          onClick={() => router.push('/')}
          iconBadge={isRecording && (
            <span
              aria-hidden
              className={`absolute -right-1 -top-1 h-2.5 w-2.5 rounded-full ring-2 ring-white ${
                isPaused ? 'bg-amber-500' : 'bg-red-500 motion-safe:animate-pulse'
              }`}
            />
          )}
        />

        <NavItem
          collapsed={isCollapsed}
          icon={<Library className="h-5 w-5" />}
          label={t('Meetings')}
          active={isLibraryPage}
          onClick={() => router.push('/meetings')}
          trailing={meetings.length > 0 && <span className="text-xs text-gray-400">{meetings.length}</span>}
        />

        {betaFeatures.importAndRetranscribe && (
          <NavItem
            collapsed={isCollapsed}
            icon={<Upload className="h-5 w-5" />}
            label={t('Import Audio')}
            onClick={() => openImportDialog()}
          />
        )}

          <NavItem
            collapsed={isCollapsed}
            icon={recordingBusy ? <Loader2 className="h-5 w-5 motion-safe:animate-spin" /> : isRecording ? <Square className="h-4 w-4 fill-current" /> : <Mic className="h-5 w-5" />}
            label={recordingBusy ? (recordingState.isStartingRecording ? t('Starting recording...') : t('Processing recording...')) : isRecording ? t('Stop') : t('Start Recording')}
            onClick={() => void toggleRecording()}
            disabled={recordingBusy}
            className={`mt-2 !bg-red-500 !text-white shadow-sm enabled:hover:!bg-red-600 disabled:cursor-default disabled:opacity-60 ${isCollapsed ? '!rounded-full' : ''}`}
          />

        {recoverableCount > 0 && (
          <NavItem
            collapsed={isCollapsed}
            icon={<Unplug className="h-5 w-5" />}
            label={t('Interrupted Meetings')}
            detail={isCollapsed ? t('Can be recovered: {count}', { count: recoverableCount }) : undefined}
            onClick={openRecoveryDialog}
            className="mt-1 !text-amber-700 hover:!bg-amber-50"
            iconBadge={isCollapsed && (
              <span className="absolute -right-2 -top-2 grid h-4 min-w-4 place-items-center rounded-full bg-amber-500 px-1 text-[10px] font-bold leading-none text-white ring-2 ring-white">
                {recoverableCount}
              </span>
            )}
            trailing={
              <span className="rounded-full bg-amber-100 px-2 py-0.5 text-xs font-semibold text-amber-700">
                {recoverableCount}
              </span>
            }
          />
        )}

        <div className="flex-1" />

        <NavItem
          collapsed={isCollapsed}
          icon={<Settings className="h-5 w-5" />}
          label={t('Settings')}
          active={pathname === '/settings'}
          onClick={() => router.push('/settings')}
        />

        <Dialog aria-describedby={undefined}>
          <DialogTrigger asChild>
            <NavItem collapsed={isCollapsed} icon={<InfoIcon className="h-5 w-5" />} label={t('About')} />
          </DialogTrigger>
          <DialogContent>
            <VisuallyHidden>
              <DialogTitle>{t('About Ru-Meet')}</DialogTitle>
            </VisuallyHidden>
            <About />
          </DialogContent>
        </Dialog>

        <NavItem
          collapsed={isCollapsed}
          icon={isCollapsed ? <PanelLeftOpen className="h-5 w-5" /> : <PanelLeftClose className="h-5 w-5" />}
          label={isCollapsed ? t('Expand') : t('Collapse')}
          onClick={toggleCollapse}
        />
      </nav>
    </TooltipProvider>
  );
};

export default Sidebar;
