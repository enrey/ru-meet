import { useCallback, useEffect, useRef, useState } from 'react';
import { convertFileSrc, invoke } from '@tauri-apps/api/core';

/** Interval of a meeting recording, in seconds. */
export interface VoiceClip {
  start: number;
  end: number;
}

export interface VoiceSample {
  id: string;
  meetingId: string | null;
  meetingTitle: string | null;
  start: number;
  end: number;
  /** Made by the current extractor; older samples no longer match. */
  current: boolean;
  createdAt: string;
}

export interface VoiceProfile {
  id: string;
  name: string;
  createdAt: string;
  updatedAt: string;
  samples: VoiceSample[];
  confirmedMeetings: number;
}

export type VoiceLinkStatus = 'suggested' | 'confirmed' | 'rejected';

/** A meeting speaker's relation to a library profile. */
export interface VoiceLink {
  speaker: string;
  profileId: string;
  profileName: string;
  status: VoiceLinkStatus;
  similarity: number | null;
}

export type EnrollmentProblem = 'others' | 'no-audio' | 'too-little-speech';

export interface EnrollmentCandidates {
  clips: VoiceClip[];
  totalSeconds: number;
  minClips: number;
  minSeconds: number;
  problem: EnrollmentProblem | null;
}

export interface RememberVoiceResult {
  profileId: string;
  name: string;
  kept: number;
  discarded: number;
  /** The speaker's label afterwards. */
  speaker: string;
}

export const voiceprints = {
  profiles: () => invoke<VoiceProfile[]>('list_voice_profiles'),
  renameProfile: (profileId: string, name: string) => invoke<void>('rename_voice_profile', { profileId, name }),
  deleteProfile: (profileId: string) => invoke<void>('delete_voice_profile', { profileId }),
  deleteSample: (sampleId: string) => invoke<void>('delete_voice_sample', { sampleId }),
  links: (meetingId: string) => invoke<VoiceLink[]>('get_meeting_voice_links', { meetingId }),
  confirm: (meetingId: string, speaker: string) => invoke<string>('confirm_voice_suggestion', { meetingId, speaker }),
  reject: (meetingId: string, speaker: string) => invoke<void>('reject_voice_suggestion', { meetingId, speaker }),
  refresh: (meetingId: string) => invoke<number>('refresh_voice_suggestions', { meetingId }),
  /** Queues a matching task per meeting worth checking; returns how many. */
  matchAll: () => invoke<number>('match_voice_library'),
  candidates: (meetingId: string, speaker: string) =>
    invoke<EnrollmentCandidates>('get_voice_enrollment_candidates', { meetingId, speaker }),
  remember: (args: { meetingId: string; speaker: string; clips: VoiceClip[]; profileId?: string; name?: string }) =>
    invoke<RememberVoiceResult>('remember_voice', args),
};

/**
 * Plays one interval of a meeting recording in its own audio element, so
 * listening to a fragment neither moves nor depends on the meeting player.
 */
export function useClipPlayer(onStart?: () => void) {
  const audio = useRef<HTMLAudioElement | null>(null);
  const sources = useRef(new Map<string, string>());
  const until = useRef(0);
  const [playing, setPlaying] = useState<string | null>(null);

  useEffect(() => () => {
    audio.current?.pause();
    audio.current?.removeAttribute('src');
  }, []);

  const stop = useCallback(() => {
    audio.current?.pause();
    setPlaying(null);
  }, []);

  const play = useCallback(async (key: string, meetingId: string, clip: VoiceClip) => {
    if (!audio.current) {
      const element = new Audio();
      element.addEventListener('timeupdate', () => {
        if (element.currentTime >= until.current) {
          element.pause();
          setPlaying(null);
        }
      });
      element.addEventListener('ended', () => setPlaying(null));
      audio.current = element;
    }
    const element = audio.current;
    let source = sources.current.get(meetingId);
    if (!source) {
      // The command also grants the webview read access to the file.
      const path = await invoke<string | null>('get_meeting_audio_path', { meetingId });
      if (!path) throw new Error('This meeting has no saved recording');
      source = convertFileSrc(path);
      sources.current.set(meetingId, source);
    }
    onStart?.();
    if (element.src !== source) {
      element.src = source;
      await new Promise<void>((resolve, reject) => {
        element.addEventListener('loadedmetadata', () => resolve(), { once: true });
        element.addEventListener('error', () => reject(new Error('Could not load the recording')), { once: true });
      });
    }
    until.current = clip.end;
    element.currentTime = clip.start;
    setPlaying(key);
    await element.play();
  }, [onStart]);

  return { playing, play, stop };
}

export function formatClipTime(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const rest = String(total % 60).padStart(2, '0');
  return hours ? `${hours}:${String(minutes).padStart(2, '0')}:${rest}` : `${minutes}:${rest}`;
}
