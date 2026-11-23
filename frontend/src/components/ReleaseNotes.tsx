import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { ReleaseNotesEntry } from '@/services/updateService';
import { getIntlLocale, useI18n } from '@/lib/i18n';

/** Published releases, newest first, from GitHub; also refreshes the local cache. Network: call only on user action. */
export function fetchReleaseHistory(): Promise<ReleaseNotesEntry[]> {
  return invoke<ReleaseNotesEntry[]>('fetch_release_history');
}

/** Release notes cached locally by update checks or an explicit fetch; no network. */
export function getCachedReleaseHistory(): Promise<ReleaseNotesEntry[]> {
  return invoke<ReleaseNotesEntry[]>('get_cached_release_history');
}

/** Drops the leading `# Title` line: the version heading already names the release. */
function stripLeadingTitle(notes: string): string {
  return notes.replace(/^\s*#\s[^\n]*\n+/, '');
}

function ReleaseNotesMarkdown({ notes }: { notes: string }) {
  return (
    <div className="prose prose-sm prose-slate max-w-none text-slate-700 prose-headings:mt-3 prose-headings:mb-1 prose-headings:text-sm prose-headings:font-semibold prose-p:my-1 prose-ul:my-1 prose-li:my-0">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          a: ({ href, children }) => (
            <a
              href={href}
              className="text-indigo-600 underline"
              onClick={(event) => {
                event.preventDefault();
                if (href && /^https:\/\//.test(href)) void invoke('open_external_url', { url: href });
              }}
            >
              {children}
            </a>
          ),
        }}
      >
        {stripLeadingTitle(notes)}
      </ReactMarkdown>
    </div>
  );
}

interface ReleaseNotesListProps {
  releases: ReleaseNotesEntry[];
  /** Marks the matching entry as the installed version. */
  installedVersion?: string | null;
}

/** Version-headed release notes, one section per release. */
export function ReleaseNotesList({ releases, installedVersion }: ReleaseNotesListProps) {
  const { t, locale } = useI18n();
  const formatDate = (date: string) => {
    try {
      return new Date(date).toLocaleDateString(getIntlLocale(locale));
    } catch {
      return date;
    }
  };

  return (
    <div className="space-y-5">
      {releases.map((release) => (
        <section key={release.version} className="space-y-1">
          <h3 className="flex items-baseline gap-2 border-b border-slate-200 pb-1 text-sm font-semibold text-slate-900">
            {t('Version {version}', { version: release.version })}
            {release.pub_date && (
              <span className="text-xs font-normal text-slate-500">{formatDate(release.pub_date)}</span>
            )}
            {installedVersion === release.version && (
              <span className="ml-auto rounded bg-emerald-50 px-1.5 py-0.5 text-xs font-normal text-emerald-700">
                {t('Installed')}
              </span>
            )}
          </h3>
          <ReleaseNotesMarkdown notes={release.notes} />
        </section>
      ))}
    </div>
  );
}
