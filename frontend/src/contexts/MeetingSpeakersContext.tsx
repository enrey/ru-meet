'use client';

import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { toast } from 'sonner';
import type { SpeakerTurn } from '@/services/storageService';
import { useI18n } from '@/lib/i18n';

/** One colour per speaker, shared by the timeline and the transcript. */
export const SPEAKER_COLORS = ['#6366f1', '#06b6d4', '#f59e0b', '#ec4899', '#10b981', '#8b5cf6'];

/** What a rename request turned out to need. */
export type SpeakerRenameResult =
  | { kind: 'unchanged' }
  | { kind: 'invalid' }
  /** The name belongs to another speaker: renaming merges the two, confirm first. */
  | { kind: 'needs-merge'; target: string }
  | { kind: 'done' };

interface MeetingSpeakersValue {
  turns: SpeakerTurn[];
  /** Diarization is re-running; the old labels are about to change. */
  inProgress: boolean;
  /** Stored speaker labels in order of first appearance. */
  speakers: string[];
  colorFor: (speaker: string) => string | undefined;
  /** "Speaker 3" → "Спикер 3"; names the user typed are shown as is. */
  displayName: (speaker: string) => string;
  /** Validate a typed name; renames unless it would merge two speakers. */
  requestRename: (from: string, typed: string) => Promise<SpeakerRenameResult>;
  /** Rename (or merge) without further checks, after the user confirmed. */
  commitRename: (from: string, to: string) => Promise<boolean>;
}

const MeetingSpeakersContext = createContext<MeetingSpeakersValue | null>(null);

/** Null outside a meeting page (e.g. the live recording transcript). */
export function useMeetingSpeakers(): MeetingSpeakersValue | null {
  return useContext(MeetingSpeakersContext);
}

const DEFAULT_LABEL = /^Speaker (\d+)$/;
/** Matches `OTHERS_SPEAKER` in `src-tauri/src/audio/diarization.rs`. */
const OTHERS_LABEL = 'Others';

export function MeetingSpeakersProvider({
  meetingId,
  onSpeakerRenamed,
  children,
}: {
  meetingId: string;
  /** Lets the transcript relabel its already loaded lines. */
  onSpeakerRenamed?: (oldName: string, newName: string) => void;
  children: ReactNode;
}) {
  const { t } = useI18n();
  const [turns, setTurns] = useState<SpeakerTurn[]>([]);
  const [inProgress, setInProgress] = useState(false);

  useEffect(() => {
    let active = true;
    const load = () => invoke<SpeakerTurn[]>('get_meeting_speaker_turns', { meetingId })
      .then((value) => { if (active) setTurns(value); })
      .catch((error) => console.warn('Failed to load speaker timeline:', error));

    setTurns([]);
    void load();
    void invoke<{ inProgress: boolean; meetingId?: string }>('get_diarization_status', { meetingId })
      .then((status) => { if (active) setInProgress(status.inProgress && status.meetingId === meetingId); })
      .catch((error) => console.warn('Failed to load diarization status:', error));
    const subscriptions = Promise.all([
      listen<{ meetingId?: string }>('diarization-progress', ({ payload }) => {
        if (payload.meetingId === meetingId) setInProgress(true);
      }),
      listen<{ meetingId?: string }>('diarization-labels-saved', ({ payload }) => {
        if (!payload.meetingId || payload.meetingId === meetingId) {
          setInProgress(false);
          void load();
        }
      }),
      listen<{ meetingId?: string }>('diarization-rerun-error', ({ payload }) => {
        if (payload.meetingId === meetingId) setInProgress(false);
      }),
      listen<{ meetingId?: string }>('diarization-cancelled', ({ payload }) => {
        if (payload.meetingId === meetingId) setInProgress(false);
      }),
    ]);
    return () => {
      active = false;
      void subscriptions.then((listeners) => listeners.forEach((unlisten) => unlisten()));
    };
  }, [meetingId]);

  const speakers = useMemo(
    () => Array.from(new Set(turns.filter((turn) => turn.speaker.trim()).map((turn) => turn.speaker))),
    [turns],
  );

  const colorFor = useCallback((speaker: string) => {
    const index = speakers.indexOf(speaker);
    return index < 0 ? undefined : SPEAKER_COLORS[index % SPEAKER_COLORS.length];
  }, [speakers]);

  const displayName = useCallback((speaker: string) => {
    if (speaker === OTHERS_LABEL) return t('Others');
    const match = DEFAULT_LABEL.exec(speaker);
    return match ? t('Speaker {number}', { number: match[1] }) : speaker;
  }, [t]);

  const commitRename = useCallback(async (from: string, to: string) => {
    try {
      const merged = await invoke<boolean>('rename_meeting_speaker', { meetingId, oldName: from, newName: to });
      setTurns((current) => current.map((turn) => (turn.speaker === from ? { ...turn, speaker: to } : turn)));
      onSpeakerRenamed?.(from, to);
      if (merged) toast.success(t('Merged “{from}” into “{to}”', { from: displayName(from), to: displayName(to) }));
      return true;
    } catch (error) {
      toast.error(t('Could not rename speaker'), { description: String(error) });
      return false;
    }
  }, [meetingId, onSpeakerRenamed, t, displayName]);

  const requestRename = useCallback(async (from: string, typed: string): Promise<SpeakerRenameResult> => {
    const name = typed.trim();
    if (!name || name.length > 80) {
      toast.error(t('Speaker name must be between 1 and 80 characters'));
      return { kind: 'invalid' };
    }
    if (name === from || name === displayName(from)) return { kind: 'unchanged' };
    // Taking another speaker's name (stored or as displayed) folds the two
    // together. The only way back is re-running diarization, so ask first.
    const target = speakers.find((speaker) => speaker !== from && (speaker === name || displayName(speaker) === name));
    if (target) return { kind: 'needs-merge', target };
    return (await commitRename(from, name)) ? { kind: 'done' } : { kind: 'invalid' };
  }, [speakers, displayName, commitRename, t]);

  const value = useMemo<MeetingSpeakersValue>(
    () => ({ turns, inProgress, speakers, colorFor, displayName, requestRename, commitRename }),
    [turns, inProgress, speakers, colorFor, displayName, requestRename, commitRename],
  );

  return <MeetingSpeakersContext.Provider value={value}>{children}</MeetingSpeakersContext.Provider>;
}
