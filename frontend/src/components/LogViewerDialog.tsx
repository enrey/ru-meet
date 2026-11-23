'use client';

import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Copy, FolderOpen } from 'lucide-react';
import { toast } from 'sonner';
import { Button } from './ui/button';
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from './ui/dialog';
import { useI18n } from '@/lib/i18n';

interface LogTail {
  path: string;
  text: string;
  truncated: boolean;
}

/** How often an open viewer reads the log again. */
const REFRESH_MS = 2000;

/**
 * The end of the app's log file, kept current while open. Scrolled to the
 * newest line unless the user scrolled up to read.
 */
export function LogViewerDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (open: boolean) => void }) {
  const { t } = useI18n();
  const [log, setLog] = useState<LogTail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const scrollRef = useRef<HTMLPreElement>(null);
  const followRef = useRef(true);

  const load = useCallback(() => {
    invoke<LogTail>('get_log_tail')
      .then((tail) => {
        setLog(tail);
        setError(null);
      })
      .catch((reason) => setError(String(reason)));
  }, []);

  useEffect(() => {
    if (!open) return;
    followRef.current = true;
    load();
    const timer = window.setInterval(load, REFRESH_MS);
    return () => window.clearInterval(timer);
  }, [open, load]);

  useLayoutEffect(() => {
    const element = scrollRef.current;
    if (element && followRef.current) element.scrollTop = element.scrollHeight;
  }, [log?.text]);

  const copy = async () => {
    if (!log) return;
    try {
      await navigator.clipboard.writeText(log.text);
      toast.success(t('Log copied'));
    } catch (reason) {
      toast.error(t('Could not copy the log'), { description: String(reason) });
    }
  };

  const openFolder = () => {
    invoke('open_logs_folder').catch((reason) =>
      toast.error(t('Could not open the logs folder'), { description: String(reason) }));
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex h-[85vh] max-w-[min(1100px,95vw)] flex-col gap-3 sm:max-w-[min(1100px,95vw)]">
        <DialogHeader>
          <DialogTitle>{t('Application log')}</DialogTitle>
          <DialogDescription className="truncate font-mono text-xs" title={log?.path}>
            {log?.path ?? t('Loading...')}
          </DialogDescription>
        </DialogHeader>
        {log?.truncated && (
          <p className="text-xs text-slate-500">{t('Showing the end of the log. The whole file is in the logs folder.')}</p>
        )}
        {error ? (
          <p className="rounded-md bg-red-50 p-3 text-sm text-red-700">{error}</p>
        ) : (
          <pre
            ref={scrollRef}
            onScroll={(event) => {
              const element = event.currentTarget;
              followRef.current = element.scrollHeight - element.scrollTop - element.clientHeight < 24;
            }}
            className="min-h-0 flex-1 overflow-auto whitespace-pre rounded-md border border-slate-200 bg-slate-950 p-3 font-mono text-[11px] leading-relaxed text-slate-100"
          >
            {log?.text}
          </pre>
        )}
        <div className="flex justify-end gap-2">
          <Button variant="outline" size="sm" onClick={openFolder}>
            <FolderOpen className="mr-2 h-4 w-4" />
            {t('Open logs folder')}
          </Button>
          <Button variant="outline" size="sm" onClick={() => void copy()} disabled={!log}>
            <Copy className="mr-2 h-4 w-4" />
            {t('Copy')}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
