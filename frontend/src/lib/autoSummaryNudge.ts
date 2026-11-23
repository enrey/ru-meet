import { toast } from 'sonner';
import { translate } from '@/lib/i18n';

const STORAGE_KEY = 'autoSummaryNudge';
/** Offer automatic summaries on every this many summaries made by hand. */
const EVERY = 3;

interface NudgeState {
  count: number;
  dismissed: boolean;
}

function load(): NudgeState {
  try {
    const parsed = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '{}') as Partial<NudgeState>;
    return { count: Number(parsed.count) || 0, dismissed: parsed.dismissed === true };
  } catch {
    return { count: 0, dismissed: false };
  }
}

function save(state: NudgeState) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
}

/**
 * Count a summary created by hand while automatic summaries are off; every
 * third time, offer to turn them on, until the user declines for good.
 */
export function countManualSummary(enableAutoSummary: () => void) {
  const state = load();
  if (state.dismissed) return;
  state.count += 1;
  save(state);
  if (state.count % EVERY !== 0) return;

  toast(translate('Create summaries automatically?'), {
    description: translate('You often create summaries by hand. They can be created after every recording by themselves.'),
    duration: 15000,
    action: {
      label: translate('Turn on'),
      onClick: () => {
        enableAutoSummary();
        save({ ...load(), dismissed: true });
        toast.success(translate('Summaries will be created after every recording'));
      },
    },
    cancel: {
      label: translate("Don't offer again"),
      onClick: () => save({ ...load(), dismissed: true }),
    },
  });
}
