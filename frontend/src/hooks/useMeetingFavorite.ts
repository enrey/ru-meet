'use client';

import { useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { useI18n } from '@/lib/i18n';

/** Toggle a meeting's favorite flag optimistically in the shared meeting list. */
export function useMeetingFavorite() {
  const { setMeetings } = useSidebar();
  const { t } = useI18n();

  return useCallback(async (meetingId: string, favorite: boolean) => {
    const apply = (value: boolean) => setMeetings((current) =>
      current.map((item) => (item.id === meetingId ? { ...item, isFavorite: value } : item)));
    apply(favorite);
    try {
      await invoke('api_set_meeting_favorite', { meetingId, favorite });
    } catch (error) {
      apply(!favorite);
      toast.error(t('Could not update favorites'), { description: String(error) });
    }
  }, [setMeetings, t]);
}
