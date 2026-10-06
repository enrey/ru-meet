import { en, ru } from '@blocknote/core/locales';
import { getLocale } from './index';

/**
 * BlockNote's own UI strings (slash menu, toolbar, placeholders) for the
 * current interface language. Editors pick it up when they are created.
 */
export function blockNoteDictionary() {
  return getLocale() === 'ru' ? ru : en;
}
