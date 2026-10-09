'use client';

import { ArrowRightLeft, AudioLines } from 'lucide-react';
import { formatPlaybackTime, useMeetingPlayback, type PlaybackControls } from '@/contexts/MeetingPlaybackContext';
import { useI18n } from '@/lib/i18n';

/**
 * Shown on a tab whose own audio is not the one playing: the player keeps the
 * other track going instead of cutting it off, and this offers the switch.
 * `tab` is the tab it sits on; `summary` is the summary reading's track, null
 * while it is not prepared.
 */
export function PlaybackSourceNotice({
  tab,
  summary,
}: {
  tab: 'transcript' | 'summary';
  summary: PlaybackControls | null;
}) {
  const { t } = useI18n();
  const recording = useMeetingPlayback();

  if (tab === 'summary') {
    if (!summary?.isAvailable || !recording?.isPlaying) return null;
    return (
      <Notice
        text={t('Meeting recording is playing ({time})', { time: formatPlaybackTime(recording.currentTime) })}
        action={t('Switch to the summary reading')}
        onSwitch={() => summary.play()}
      />
    );
  }

  if (!summary?.isPlaying || !recording?.isAvailable) return null;
  return (
    <Notice
      text={t('The summary is being read aloud ({time})', { time: formatPlaybackTime(summary.currentTime) })}
      action={t('Switch to the recording')}
      onSwitch={() => recording.play()}
    />
  );
}

function Notice({ text, action, onSwitch }: { text: string; action: string; onSwitch: () => void }) {
  return (
    <div className="mx-8 mt-3 flex shrink-0 items-center gap-2 rounded-lg border border-violet-100 bg-violet-50 px-3 py-2 text-sm text-violet-900">
      <AudioLines className="h-4 w-4 shrink-0 text-violet-500" />
      <span className="min-w-0 truncate">{text}</span>
      <span className="text-violet-300">·</span>
      <button
        type="button"
        onClick={onSwitch}
        className="flex shrink-0 items-center gap-1 font-medium text-violet-700 hover:text-violet-900 hover:underline"
      >
        <ArrowRightLeft className="h-3.5 w-3.5" />
        {action}
      </button>
    </div>
  );
}
