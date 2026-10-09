'use client';

import { AudioLines, X } from 'lucide-react';
import { useRouter } from 'next/navigation';
import { useMeetingPlayer, useSpaceToggle } from '@/contexts/MeetingPlaybackContext';
import { PlayerControls } from '@/components/MeetingDetails/MeetingPlayerBar';
import { useI18n } from '@/lib/i18n';

/**
 * The meeting player after its page was left: keeps playing under the
 * recording bar on every other page until closed with the cross.
 */
export function DockedMeetingPlayer() {
  const { t } = useI18n();
  const router = useRouter();
  const player = useMeetingPlayer();
  const visible = Boolean(player?.session && player.started && player.session.meetingId !== player.visibleMeetingId);

  useSpaceToggle(visible, player?.toggle);

  if (!player?.session || !visible) return null;
  const { meetingId, title } = player.session;

  return (
    <div className="@container relative shrink-0 border-b border-slate-200">
      <PlayerControls
        playback={player}
        className="py-2 pl-8 pr-12"
        label={(
          <button
            type="button"
            onClick={() => router.push(`/meeting-details?id=${encodeURIComponent(meetingId)}`)}
            title={t('Open meeting')}
            className="flex min-w-0 max-w-[16rem] shrink items-center gap-1.5 rounded-md px-1.5 py-1 text-left text-sm font-medium text-slate-700 transition-colors hover:bg-slate-100 hover:text-slate-950"
          >
            <AudioLines className="h-4 w-4 shrink-0 text-indigo-500" />
            <span className="truncate">{title}</span>
          </button>
        )}
      />
      <button
        type="button"
        onClick={player.close}
        title={t('Stop and close player')}
        aria-label={t('Stop and close player')}
        className="absolute right-2 top-1.5 grid h-5 w-5 place-items-center rounded text-slate-400 transition-colors hover:bg-slate-100 hover:text-slate-600"
      >
        <X className="h-3.5 w-3.5" />
      </button>
    </div>
  );
}
