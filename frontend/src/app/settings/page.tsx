'use client';

import React, { useState, useEffect, useLayoutEffect, useRef } from 'react';
import { ArrowLeft, Settings2, Mic, Database as DatabaseIcon, SparkleIcon, FlaskConical, UsersRound } from 'lucide-react';
import { useRouter } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
import { motion, useReducedMotion } from 'framer-motion';
import { TranscriptSettings } from '@/components/TranscriptSettings';
import { RecordingSettings } from '@/components/RecordingSettings';
import { PreferenceSettings } from '@/components/PreferenceSettings';
import { SummaryModelSettings } from '@/components/SummaryModelSettings';
import { BetaSettings } from '@/components/BetaSettings';
import { DiarizationSettings } from '@/components/DiarizationSettings';
import { useConfig } from '@/contexts/ConfigContext';

// Tabs configuration (constant)
const TABS = [
  { value: 'general', label: 'General', icon: Settings2 },
  { value: 'recording', label: 'Recordings', icon: Mic },
  { value: 'Transcriptionmodels', label: 'Transcription', icon: DatabaseIcon },
  { value: 'summaryModels', label: 'Summary', icon: SparkleIcon },
  { value: 'diarization', label: 'Diarization', icon: UsersRound },
  { value: 'beta', label: 'Beta', icon: FlaskConical }
] as const;

export default function SettingsPage() {
  const router = useRouter();
  const { transcriptModelConfig, setTranscriptModelConfig } = useConfig();

  // Animation state for tabs
  const [activeTab, setActiveTab] = useState('general');
  const tabRefs = useRef<(HTMLAnchorElement | null)[]>([]);
  const scrollRef = useRef<HTMLDivElement>(null);
  const sectionRefs = useRef<(HTMLElement | null)[]>([]);
  const reduceMotion = useReducedMotion();
  const [underlineStyle, setUnderlineStyle] = useState({ left: 0, width: 0 });

  // Load saved transcript configuration on mount
  useEffect(() => {
    const loadTranscriptConfig = async () => {
      try {
        const config = await invoke('api_get_transcript_config') as any;
        if (config) {
          console.log('Loaded saved transcript config:', config);
          setTranscriptModelConfig({
            provider: config.provider || 'localWhisper',
            model: config.model || 'large-v3',
            apiKey: config.apiKey || null
          });
        }
      } catch (error) {
        console.error('Failed to load transcript config:', error);
      }
    };
    loadTranscriptConfig();
  }, [setTranscriptModelConfig]);

  // Update underline position when active tab changes
  useLayoutEffect(() => {
    const activeIndex = TABS.findIndex(tab => tab.value === activeTab);
    const activeTabElement = tabRefs.current[activeIndex];

    if (!activeTabElement) return;
    const updateUnderline = () => {
      const { offsetLeft, offsetWidth } = activeTabElement;
      setUnderlineStyle({ left: offsetLeft, width: offsetWidth });
    };
    updateUnderline();
    const observer = new ResizeObserver(updateUnderline);
    tabRefs.current.forEach(tab => { if (tab) observer.observe(tab); });
    return () => observer.disconnect();
  }, [activeTab]);

  useEffect(() => {
    const container = scrollRef.current;
    if (!container) return;
    let frame = 0;
    const updateActiveSection = () => {
      const top = container.getBoundingClientRect().top;
      let index = 0;
      sectionRefs.current.forEach((section, sectionIndex) => {
        if (section && section.getBoundingClientRect().top <= top + 48) index = sectionIndex;
      });
      if (container.scrollTop > 0 && container.scrollTop + container.clientHeight >= container.scrollHeight - 2) {
        index = TABS.length - 1;
      }
      setActiveTab(TABS[index].value);
    };
    const scheduleUpdate = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(updateActiveSection);
    };
    container.addEventListener('scroll', scheduleUpdate, { passive: true });
    const observer = new ResizeObserver(scheduleUpdate);
    observer.observe(container);
    sectionRefs.current.forEach(section => { if (section) observer.observe(section); });
    updateActiveSection();
    return () => {
      cancelAnimationFrame(frame);
      container.removeEventListener('scroll', scheduleUpdate);
      observer.disconnect();
    };
  }, []);

  const scrollToSection = (index: number) => {
    const container = scrollRef.current;
    const section = sectionRefs.current[index];
    if (!container || !section) return;
    container.scrollTo({
      top: container.scrollTop + section.getBoundingClientRect().top - container.getBoundingClientRect().top - 24,
      behavior: reduceMotion ? 'instant' : 'smooth',
    });
  };

  return (
    <div className="h-screen min-h-0 bg-gray-50 flex flex-col overflow-hidden">
      {/* Fixed Header */}
      <div className="sticky top-0 z-10 bg-gray-50 border-b border-gray-200">
        <div className="max-w-6xl mx-auto px-8 py-6">
          <div className="flex items-center gap-4">
            <button
              onClick={() => router.back()}
              className="flex items-center gap-2 text-gray-600 hover:text-gray-900 transition-colors"
            >
              <ArrowLeft className="w-5 h-5" />
              <span>Back</span>
            </button>
            <h1 className="text-3xl font-bold">Settings</h1>
          </div>
        </div>
      </div>

      <nav aria-label="Settings sections" className="shrink-0 bg-gray-50 px-4 sm:px-8">
        <div className="max-w-6xl mx-auto overflow-x-auto">
            <div className="flex relative w-max min-w-full border-b border-gray-200">
              {TABS.map((tab, index) => {
                const Icon = tab.icon;
                return (
                  <a
                    key={tab.value}
                    href={`#settings-${tab.value}`}
                    aria-current={activeTab === tab.value ? 'location' : undefined}
                    onClick={event => {
                      event.preventDefault();
                      scrollToSection(index);
                    }}
                    ref={el => { tabRefs.current[index] = el }}
                    className={`flex items-center gap-2 px-4 sm:px-6 py-4 whitespace-nowrap transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-blue-600 relative z-10 ${activeTab === tab.value ? 'text-blue-600 font-medium' : 'text-gray-600 hover:text-gray-900'}`}
                  >
                    <Icon className="w-4 h-4" aria-hidden="true" />
                    {tab.label}
                  </a>
                );
              })}

              <motion.div
                className="absolute bottom-0 z-20 h-0.5 bg-blue-600"
                aria-hidden="true"
                animate={{ left: underlineStyle.left, width: underlineStyle.width }}
                transition={reduceMotion ? { duration: 0 } : { type: 'spring', stiffness: 400, damping: 40 }}
              />
            </div>
        </div>
      </nav>

      {/* All sections share one scroll container and stay mounted. */}
      <div ref={scrollRef} className="flex-1 min-h-0 overflow-y-auto overscroll-contain">
        <div className="max-w-6xl mx-auto px-4 sm:px-8 py-6">
          {TABS.map((tab, index) => (
            <section
              key={tab.value}
              id={`settings-${tab.value}`}
              aria-labelledby={`settings-heading-${tab.value}`}
              ref={element => { sectionRefs.current[index] = element; }}
              className="py-8 first:pt-0 border-b border-gray-200 last:border-b-0"
            >
              <h2 id={`settings-heading-${tab.value}`} className="text-xl font-semibold text-gray-900 mb-6">
                {tab.label}
              </h2>
              {tab.value === 'general' && <PreferenceSettings />}
              {tab.value === 'recording' && <RecordingSettings />}
              {tab.value === 'Transcriptionmodels' && (
                <TranscriptSettings
                  transcriptModelConfig={transcriptModelConfig}
                  setTranscriptModelConfig={setTranscriptModelConfig}
                />
              )}
              {tab.value === 'summaryModels' && <SummaryModelSettings />}
              {tab.value === 'diarization' && <DiarizationSettings />}
              {tab.value === 'beta' && <BetaSettings />}
            </section>
          ))}
        </div>
      </div>
    </div>
  );
};
