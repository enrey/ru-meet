import { getIntlLocale, translate } from '@/lib/i18n';

/**
 * Default title of a recording starting now: "Встреча 23 ноября, 10:53".
 * Matches the Rust core's `audio::meeting_title` for recordings it starts.
 */
export function defaultMeetingTitle(now = new Date()): string {
  const locale = getIntlLocale();
  const date = new Intl.DateTimeFormat(locale, { day: 'numeric', month: locale.startsWith('ru') ? 'long' : 'short' }).format(now);
  const time = new Intl.DateTimeFormat(locale, { hour: '2-digit', minute: '2-digit', hour12: false }).format(now);
  return `${translate('Meeting')} ${date}, ${time}`;
}
