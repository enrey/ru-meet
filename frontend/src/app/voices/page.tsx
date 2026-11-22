'use client';

import { useCallback, useEffect, useState } from 'react';
import Link from 'next/link';
import { listen } from '@tauri-apps/api/event';
import { AlertCircle, ChevronDown, ChevronRight, Loader2, Pencil, Play, Square, Trash2, UserRound, X } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { useMeetingPlayer } from '@/contexts/MeetingPlaybackContext';
import { getIntlLocale, useI18n } from '@/lib/i18n';
import { formatClipTime, useClipPlayer, voiceprints, type VoiceProfile } from '@/services/voiceprintService';

const sampleSeconds = (profile: VoiceProfile) =>
  profile.samples.filter((sample) => sample.current).reduce((sum, sample) => sum + sample.end - sample.start, 0);

export default function VoicesPage() {
  const { t, locale } = useI18n();
  const player = useMeetingPlayer();
  const clipPlayer = useClipPlayer(player?.pause);
  const [profiles, setProfiles] = useState<VoiceProfile[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [renaming, setRenaming] = useState<{ id: string; name: string } | null>(null);
  const [forgetting, setForgetting] = useState<VoiceProfile | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(() => {
    voiceprints.profiles()
      .then((value) => { setProfiles(value); setError(null); })
      .catch((reason) => setError(String(reason)));
  }, []);

  useEffect(() => {
    load();
    const unlisten = listen('voice-library-updated', load);
    return () => { void unlisten.then((stop) => stop()); };
  }, [load]);

  const toggle = (id: string) => setExpanded((current) => {
    const next = new Set(current);
    if (next.has(id)) next.delete(id); else next.add(id);
    return next;
  });

  const run = async (action: () => Promise<unknown>, failure: string) => {
    if (busy) return false;
    setBusy(true);
    try {
      await action();
      load();
      return true;
    } catch (reason) {
      toast.error(t(failure), { description: t(String(reason)) });
      return false;
    } finally {
      setBusy(false);
    }
  };

  const saveName = async () => {
    if (!renaming) return;
    const name = renaming.name.trim();
    if (!name || name.length > 80) {
      toast.error(t('Speaker name must be between 1 and 80 characters'));
      return;
    }
    if (await run(() => voiceprints.renameProfile(renaming.id, name), 'Could not rename the profile')) setRenaming(null);
  };

  const forget = async () => {
    if (!forgetting) return;
    const name = forgetting.name;
    if (await run(() => voiceprints.deleteProfile(forgetting.id), 'Could not forget the voice')) {
      setForgetting(null);
      toast.success(t('Voice of “{name}” forgotten', { name }));
    }
  };

  const listenTo = (key: string, meetingId: string, start: number, end: number) => {
    if (clipPlayer.playing === key) {
      clipPlayer.stop();
      return;
    }
    clipPlayer.play(key, meetingId, { start, end }).catch((reason) => {
      clipPlayer.stop();
      toast.error(t('Could not play the fragment'), { description: t(String(reason?.message ?? reason)) });
    });
  };

  return (
    <div className="h-full overflow-y-auto pr-8 pb-12">
      <div className="mx-auto max-w-4xl py-8">
        <header className="mb-7">
          <h1 className="text-3xl font-semibold tracking-tight text-slate-900">{t('Voice library')}</h1>
          <p className="mt-2 max-w-2xl text-sm leading-6 text-slate-500">
            {t('Voices you remembered from meetings. After diarization of a new meeting ru-meet suggests the names of familiar participants; a name reaches the transcript and exports only once you confirm it. Voices are stored on this computer only.')}
          </p>
        </header>

        {error && (
          <div role="alert" className="mb-6 rounded-xl border border-red-200 bg-red-50 p-4 text-sm text-red-800">
            <p>{t('Could not load the voice library')}</p>
            <p className="mt-1 break-words">{error}</p>
            <Button variant="outline" className="mt-3" onClick={load}>{t('Refresh')}</Button>
          </div>
        )}

        {profiles === null && !error ? (
          <div className="flex items-center gap-2 text-sm text-slate-500"><Loader2 className="h-4 w-4 motion-safe:animate-spin" />{t('Loading...')}</div>
        ) : profiles?.length === 0 ? (
          <div className="flex flex-col items-center rounded-2xl border border-slate-200 bg-white px-6 py-14 text-center">
            <UserRound aria-hidden className="mb-3 h-9 w-9 text-slate-400" />
            <p className="font-medium text-slate-700">{t('No voices remembered yet')}</p>
            <p className="mt-2 max-w-md text-sm leading-6 text-slate-500">
              {t('Open a finished meeting, hover over a speaker in the Speakers panel and choose “Remember voice…”.')}
            </p>
          </div>
        ) : (
          <ul className="divide-y divide-slate-100 overflow-hidden rounded-2xl border border-slate-200 bg-white">
            {profiles?.map((profile) => {
              const open = expanded.has(profile.id);
              const usable = profile.samples.filter((sample) => sample.current).length;
              return (
                <li key={profile.id} className="px-5 py-4">
                  <div className="flex items-center gap-3">
                    <button
                      type="button"
                      onClick={() => toggle(profile.id)}
                      aria-expanded={open}
                      aria-label={open ? t('Hide samples') : t('Show samples')}
                      className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-slate-500 hover:bg-slate-100"
                    >
                      {open ? <ChevronDown className="h-4 w-4" /> : <ChevronRight className="h-4 w-4" />}
                    </button>
                    <span className="grid h-10 w-10 shrink-0 place-items-center rounded-full bg-indigo-50 text-indigo-600">
                      <UserRound className="h-5 w-5" aria-hidden />
                    </span>
                    <div className="min-w-0 flex-1">
                      {renaming?.id === profile.id ? (
                        <form className="flex items-center gap-2" onSubmit={(event) => { event.preventDefault(); void saveName(); }}>
                          <input
                            autoFocus
                            value={renaming.name}
                            maxLength={80}
                            disabled={busy}
                            onChange={(event) => setRenaming({ id: profile.id, name: event.target.value })}
                            onKeyDown={(event) => event.key === 'Escape' && setRenaming(null)}
                            className="min-w-0 flex-1 rounded-md border border-slate-300 px-2.5 py-1 text-sm outline-none focus:border-indigo-400 focus:ring-2 focus:ring-indigo-100"
                          />
                          <Button type="submit" size="sm" disabled={busy}>{t('Save')}</Button>
                          <Button type="button" size="sm" variant="ghost" disabled={busy} onClick={() => setRenaming(null)}>{t('Cancel')}</Button>
                        </form>
                      ) : (
                        <p className="truncate font-semibold text-slate-900">{profile.name}</p>
                      )}
                      <p className="mt-0.5 text-xs text-slate-500">
                        {t('samples: {count}', { count: usable })} · {t('{seconds} s of speech', { seconds: Math.round(sampleSeconds(profile)) })}
                        {profile.confirmedMeetings > 0 && ` · ${t('confirmed in meetings: {count}', { count: profile.confirmedMeetings })}`}
                      </p>
                      {usable === 0 && (
                        <p className="mt-1 flex items-center gap-1 text-xs text-amber-700">
                          <AlertCircle className="h-3.5 w-3.5" aria-hidden />
                          {t('No usable samples: this voice is not matched. Remember it again from a meeting.')}
                        </p>
                      )}
                    </div>
                    {renaming?.id !== profile.id && (
                      <div className="flex shrink-0 items-center gap-1">
                        <button
                          type="button"
                          title={t('Rename')}
                          aria-label={t('Rename')}
                          onClick={() => setRenaming({ id: profile.id, name: profile.name })}
                          className="grid h-8 w-8 place-items-center rounded-md text-slate-500 hover:bg-slate-100 hover:text-slate-800"
                        >
                          <Pencil className="h-4 w-4" />
                        </button>
                        <button
                          type="button"
                          title={t('Forget voice')}
                          aria-label={t('Forget voice')}
                          onClick={() => setForgetting(profile)}
                          className="grid h-8 w-8 place-items-center rounded-md text-slate-500 hover:bg-red-50 hover:text-red-700"
                        >
                          <Trash2 className="h-4 w-4" />
                        </button>
                      </div>
                    )}
                  </div>

                  {open && (
                    <ul className="ml-[4.75rem] mt-3 space-y-1">
                      {profile.samples.length === 0 && <li className="text-sm text-slate-500">{t('No samples')}</li>}
                      {profile.samples.map((sample) => {
                        const playing = clipPlayer.playing === sample.id;
                        return (
                          <li key={sample.id} className="flex items-center gap-2 rounded-md py-1 pr-1 text-sm hover:bg-slate-50">
                            <button
                              type="button"
                              disabled={!sample.meetingId}
                              onClick={() => sample.meetingId && listenTo(sample.id, sample.meetingId, sample.start, sample.end)}
                              aria-label={playing ? t('Stop') : t('Listen')}
                              title={sample.meetingId ? (playing ? t('Stop') : t('Listen')) : t('The source meeting was deleted')}
                              className="grid h-7 w-7 shrink-0 place-items-center rounded-full bg-slate-100 text-slate-700 hover:bg-indigo-100 hover:text-indigo-700 disabled:opacity-40"
                            >
                              {playing ? <Square className="h-3 w-3 fill-current" /> : <Play className="h-3.5 w-3.5 fill-current" />}
                            </button>
                            <span className="w-28 shrink-0 tabular-nums text-slate-600">
                              {formatClipTime(sample.start)}–{formatClipTime(sample.end)}
                            </span>
                            <span className="min-w-0 flex-1 truncate">
                              {sample.meetingId ? (
                                <Link href={`/meeting-details?id=${encodeURIComponent(sample.meetingId)}`} className="text-slate-700 underline-offset-4 hover:text-indigo-700 hover:underline">
                                  {sample.meetingTitle ?? t('Meeting')}
                                </Link>
                              ) : (
                                <span className="text-slate-400">{t('The source meeting was deleted')}</span>
                              )}
                            </span>
                            {!sample.current && (
                              <span className="shrink-0 rounded-full bg-amber-50 px-2 py-0.5 text-xs text-amber-800" title={t('Made by an older voice model; not used for matching')}>
                                {t('outdated')}
                              </span>
                            )}
                            <span className="shrink-0 text-xs text-slate-400">
                              {new Date(sample.createdAt).toLocaleDateString(getIntlLocale(locale), { day: 'numeric', month: 'short' })}
                            </span>
                            <button
                              type="button"
                              disabled={busy}
                              title={t('Delete sample')}
                              aria-label={t('Delete sample')}
                              onClick={() => void run(() => voiceprints.deleteSample(sample.id), 'Could not delete the sample')}
                              className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-slate-400 hover:bg-red-50 hover:text-red-700"
                            >
                              <X className="h-3.5 w-3.5" />
                            </button>
                          </li>
                        );
                      })}
                    </ul>
                  )}
                </li>
              );
            })}
          </ul>
        )}
      </div>

      <Dialog open={forgetting !== null} onOpenChange={(open) => !open && !busy && setForgetting(null)}>
        <DialogContent className="sm:max-w-[440px]">
          <DialogHeader>
            <DialogTitle>{t('Forget the voice of “{name}”?', { name: forgetting?.name ?? '' })}</DialogTitle>
            <DialogDescription>
              {t('New meetings will no longer be matched against this voice. Names already confirmed in past meetings stay as they are.')}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" disabled={busy} onClick={() => setForgetting(null)}>{t('Cancel')}</Button>
            <Button variant="destructive" disabled={busy} onClick={() => void forget()}>{t('Forget voice')}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
