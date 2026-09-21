import { useEffect, useState } from 'react';
import { listen, UnlistenFn } from '@tauri-apps/api/event';

export interface SummaryProgress {
  promptTokens: number;
  generatedTokens: number;
  tokensPerSec: number;
}

interface SummaryProgressEvent {
  prompt_tokens: number;
  generated_tokens: number;
  tokens_per_sec: number;
}

/**
 * Live token counts emitted by the built-in model while it generates.
 *
 * Returns null whenever nothing is running, and while `isGenerating` is true
 * but no token has been produced yet — the model spends that stretch reading
 * the prompt, which reports no incremental progress.
 *
 * There is deliberately no percentage: generation ends at the model's
 * end-of-generation token rather than at its token limit, so a total is not
 * knowable in advance.
 */
export function useSummaryProgress(isGenerating: boolean): SummaryProgress | null {
  const [progress, setProgress] = useState<SummaryProgress | null>(null);

  useEffect(() => {
    if (!isGenerating) {
      setProgress(null);
      return;
    }

    let unlisten: UnlistenFn | undefined;
    let cancelled = false;

    listen<SummaryProgressEvent>('summary-progress', (event) => {
      setProgress({
        promptTokens: event.payload.prompt_tokens,
        generatedTokens: event.payload.generated_tokens,
        tokensPerSec: event.payload.tokens_per_sec,
      });
    })
      .then((fn) => {
        // The run can finish before the listener is registered.
        if (cancelled) {
          fn();
          return;
        }
        unlisten = fn;
      })
      .catch((err) => {
        console.warn('Failed to subscribe to summary progress:', err);
      });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [isGenerating]);

  return progress;
}
