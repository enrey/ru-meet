'use client';

import React from 'react';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { RecordingActivityBar } from '@/components/RecordingActivityBar';

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
      <RecordingActivityBar />
      <div className="pl-8 flex-1 min-h-0 min-w-0 w-full max-w-full overflow-hidden">
        {children}
      </div>
    </main>
  );
};

export default MainContent;
