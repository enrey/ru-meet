'use client';

import React from 'react';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { RecordingActivityBar } from '@/components/RecordingActivityBar';
import { DockedMeetingPlayer } from '@/components/DockedMeetingPlayer';
import { MeetingPlayerProvider } from '@/contexts/MeetingPlaybackContext';

interface MainContentProps {
  children: React.ReactNode;
}

const MainContent: React.FC<MainContentProps> = ({ children }) => {
  const { isCollapsed } = useSidebar();

  return (
    <main
      className={`flex flex-col h-screen flex-1 min-w-0 overflow-hidden transition-all duration-300 ${
        isCollapsed ? 'ml-16' : 'ml-64'
      }`}
    >
      {/* The meeting player outlives its page, so it is owned here. */}
      <MeetingPlayerProvider>
        <RecordingActivityBar />
        <DockedMeetingPlayer />
        <div className="pl-8 flex-1 min-h-0 min-w-0 w-full max-w-full overflow-hidden">
          {children}
        </div>
      </MeetingPlayerProvider>
    </main>
  );
};

export default MainContent;
