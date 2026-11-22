'use client';

import { useEffect, useMemo, useState } from 'react';
import { AlertCircle, Loader2, Play, Square } from 'lucide-react';
import { toast } from 'sonner';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { useMeetingSpeakers } from '@/contexts/MeetingSpeakersContext';
import { useMeetingPlayer } from '@/contexts/MeetingPlaybackContext';
import { useI18n } from '@/lib/i18n';
import {
  formatClipTime,
  useClipPlayer,
  voiceprints,
  type EnrollmentCandidates,
  type VoiceClip,
  type VoiceProfile,
} from '@/services/voiceprintService';

const DEFAULT_LABEL = /^Speaker \d+$/;
const clipKey = (clip: VoiceClip) => `${clip.start}-${clip.end}`;

/**
 * "Remember voice…": pick single-speaker fragments of one meeting speaker,
 * listen to them, drop mislabelled ones and save them to a new or existing
 * voice profile.
 */
export function RememberVoiceDialog({
  speaker,
  open,
  onOpenChange,
  profileId: preselected,
}: {
  speaker: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Add to this profile instead of offering a new one. */
  profileId?: string;
}) {
  const { t } = useI18n();
  const speakers = useMeetingSpeakers();
  const player = useMeetingPlayer();
  const clipPlayer = useClipPlayer(player?.pause);
  const [candidates, setCandidates] = useState<EnrollmentCandidates | null>(null);
  const [profiles, setProfiles] = useState<VoiceProfile[]>([]);
  const [excluded, setExcluded] = useState<Set<string>>(new Set());
  const [mode, setMode] = useState<'new' | 'existing'>('new');
  const [name, setName] = useState('');
  const [profileId, setProfileId] = useState('');
  const [saving, setSaving] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const meetingId = speakers?.meetingId;
  const displayName = speakers?.displayName(speaker) ?? speaker;

  useEffect(() => {
    if (!open || !meetingId) return;
    let active = true;
    setCandidates(null);
    setLoadError(null);
    setExcluded(new Set());
    setName(DEFAULT_LABEL.test(speaker) ? '' : speaker);
    setMode(preselected ? 'existing' : 'new');
    setProfileId(preselected ?? '');
    void Promise.all([voiceprints.candidates(meetingId, speaker), voiceprints.profiles()])
      .then(([found, library]) => {
        if (!active) return;
        setCandidates(found);
        setProfiles(library);
        // Remembering a speaker already named like a profile most likely adds to it.
        const same = library.find((profile) => profile.name === speaker);
        if (!preselected && same) {
          setMode('existing');
          setProfileId(same.id);
        }
      })
      .catch((error) => active && setLoadError(t(String(error))));
    return () => {
      active = false;
      clipPlayer.stop();
    };
    // clipPlayer.stop is stable; reload only when the dialog opens for a speaker.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, meetingId, speaker, preselected, t]);

  const selected = useMemo(
    () => (candidates?.clips ?? []).filter((clip) => !excluded.has(clipKey(clip))),
    [candidates, excluded],
  );
  const selectedSeconds = selected.reduce((sum, clip) => sum + clip.end - clip.start, 0);
  const enough = Boolean(candidates && selected.length >= candidates.minClips && selectedSeconds >= candidates.minSeconds);
  const target = mode === 'new' ? name.trim() : profileId;
  const canSave = enough && !candidates?.problem && Boolean(target) && !saving;

  const toggle = (clip: VoiceClip) => setExcluded((current) => {
    const next = new Set(current);
    const key = clipKey(clip);
    if (next.has(key)) next.delete(key); else next.add(key);
    return next;
  });

  const listen = (clip: VoiceClip) => {
    if (!meetingId) return;
    const key = clipKey(clip);
    if (clipPlayer.playing === key) {
      clipPlayer.stop();
      return;
    }
    clipPlayer.play(key, meetingId, clip).catch((error) => {
      clipPlayer.stop();
      toast.error(t('Could not play the fragment'), { description: t(String(error?.message ?? error)) });
    });
  };

  const save = async () => {
    if (!canSave || !meetingId || !speakers) return;
    setSaving(true);
    clipPlayer.stop();
    try {
      const result = await voiceprints.remember({
        meetingId,
        speaker,
        clips: selected,
        ...(mode === 'new' ? { name: name.trim() } : { profileId }),
      });
      speakers.applyRename(speaker, result.speaker);
      speakers.reloadVoiceLinks();
      toast.success(t('Voice of “{name}” remembered', { name: result.name }), {
        description: result.discarded > 0
          ? t('Fragments that did not match the rest were skipped: {count}', { count: result.discarded })
          : result.speaker !== result.name
            ? t('Another speaker of this meeting is already named “{name}”, so the label was kept.', { name: result.name })
            : undefined,
      });
      onOpenChange(false);
    } catch (error) {
      toast.error(t('Could not remember the voice'), { description: t(String(error)) });
    } finally {
      setSaving(false);
    }
  };

  const problemText = (problem: NonNullable<EnrollmentCandidates['problem']>, found: EnrollmentCandidates) => {
    if (problem === 'others') return t('“Others” combines several voices, so it cannot be remembered.');
    if (problem === 'no-audio') return t('This meeting has no saved recording.');
    return t('Too little clean speech of this speaker: at least {clips} fragments of 2 s or longer without overlapping voices, {seconds} s in total, are needed. Try another recording of this person.', {
      clips: found.minClips,
      seconds: found.minSeconds,
    });
  };

  return (
    <Dialog open={open} onOpenChange={(next) => !saving && onOpenChange(next)}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-[520px]">
        <DialogHeader>
          <DialogTitle>{t('Remember voice: {speaker}', { speaker: displayName })}</DialogTitle>
          <DialogDescription>
            {t('Listen to the fragments where only this speaker is heard and untick any that belong to someone else. The voice is stored on this computer only.')}
          </DialogDescription>
        </DialogHeader>

        {loadError ? (
          <p role="alert" className="rounded-lg border border-red-200 bg-red-50 p-3 text-sm text-red-800">{loadError}</p>
        ) : !candidates ? (
          <div className="flex items-center gap-2 py-6 text-sm text-slate-500"><Loader2 className="h-4 w-4 motion-safe:animate-spin" />{t('Finding fragments…')}</div>
        ) : (
          <div className="space-y-4">
            {candidates.problem && (
              <div role="alert" className="flex gap-2 rounded-lg border border-amber-200 bg-amber-50 p-3 text-sm text-amber-900">
                <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
                <p>{problemText(candidates.problem, candidates)}</p>
              </div>
            )}

            {candidates.clips.length > 0 && (
              <div>
                <ul className="divide-y divide-slate-100 rounded-lg border border-slate-200">
                  {candidates.clips.map((clip, index) => {
                    const key = clipKey(clip);
                    const included = !excluded.has(key);
                    const playing = clipPlayer.playing === key;
                    return (
                      <li key={key} className="flex items-center gap-3 px-3 py-2">
                        <input
                          type="checkbox"
                          id={`voice-clip-${index}`}
                          checked={included}
                          disabled={saving}
                          onChange={() => toggle(clip)}
                          className="h-4 w-4 accent-indigo-600"
                        />
                        <button
                          type="button"
                          onClick={() => listen(clip)}
                          aria-label={playing ? t('Stop') : t('Listen')}
                          title={playing ? t('Stop') : t('Listen')}
                          className="grid h-7 w-7 shrink-0 place-items-center rounded-full bg-slate-100 text-slate-700 hover:bg-indigo-100 hover:text-indigo-700"
                        >
                          {playing ? <Square className="h-3 w-3 fill-current" /> : <Play className="h-3.5 w-3.5 fill-current" />}
                        </button>
                        <label htmlFor={`voice-clip-${index}`} className={`flex-1 text-sm tabular-nums ${included ? 'text-slate-800' : 'text-slate-400 line-through'}`}>
                          {formatClipTime(clip.start)}–{formatClipTime(clip.end)}
                          <span className="ml-2 text-slate-400">{t('{seconds} s', { seconds: Math.round(clip.end - clip.start) })}</span>
                        </label>
                      </li>
                    );
                  })}
                </ul>
                <p className={`mt-1.5 text-xs ${enough ? 'text-slate-500' : 'text-amber-700'}`}>
                  {t('Selected: {count} fragments, {seconds} s', { count: selected.length, seconds: Math.round(selectedSeconds) })}
                  {!enough && !candidates.problem && ` · ${t('At least {clips} fragments and {seconds} s are needed', { clips: candidates.minClips, seconds: candidates.minSeconds })}`}
                </p>
              </div>
            )}

            {!candidates.problem && (
              <fieldset className="space-y-2" disabled={saving}>
                <legend className="mb-1 text-xs font-medium text-slate-500">{t('Whose voice is it?')}</legend>
                <label className="flex items-center gap-2 text-sm text-slate-700">
                  <input type="radio" name="voice-profile-mode" checked={mode === 'new'} onChange={() => setMode('new')} className="accent-indigo-600" />
                  {t('New person')}
                </label>
                {mode === 'new' && (
                  <input
                    autoFocus
                    value={name}
                    maxLength={80}
                    placeholder={t('Name, e.g. Anna Smirnova')}
                    onChange={(event) => setName(event.target.value)}
                    onKeyDown={(event) => event.key === 'Enter' && void save()}
                    className="ml-6 w-[calc(100%-1.5rem)] rounded-md border border-slate-300 px-2.5 py-1.5 text-sm outline-none focus:border-indigo-400 focus:ring-2 focus:ring-indigo-100"
                  />
                )}
                {profiles.length > 0 && (
                  <>
                    <label className="flex items-center gap-2 text-sm text-slate-700">
                      <input type="radio" name="voice-profile-mode" checked={mode === 'existing'} onChange={() => setMode('existing')} className="accent-indigo-600" />
                      {t('Add to an existing profile')}
                    </label>
                    {mode === 'existing' && (
                      <select
                        value={profileId}
                        onChange={(event) => setProfileId(event.target.value)}
                        className="ml-6 w-[calc(100%-1.5rem)] rounded-md border border-slate-300 bg-white px-2 py-1.5 text-sm outline-none focus:border-indigo-400 focus:ring-2 focus:ring-indigo-100"
                      >
                        <option value="" disabled>{t('Choose a profile')}</option>
                        {profiles.map((profile) => (
                          <option key={profile.id} value={profile.id}>
                            {profile.name} · {t('samples: {count}', { count: profile.samples.length })}
                          </option>
                        ))}
                      </select>
                    )}
                  </>
                )}
              </fieldset>
            )}
          </div>
        )}

        <DialogFooter>
          <button type="button" disabled={saving} onClick={() => onOpenChange(false)}
            className="rounded-md px-4 py-2 text-sm text-slate-600 hover:bg-slate-100 disabled:opacity-50">
            {t('Cancel')}
          </button>
          <button type="button" disabled={!canSave} onClick={() => void save()}
            className="inline-flex items-center gap-2 rounded-md bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-700 disabled:opacity-50">
            {saving && <Loader2 className="h-4 w-4 motion-safe:animate-spin" />}
            {saving ? t('Analysing voice…') : t('Remember')}
          </button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
