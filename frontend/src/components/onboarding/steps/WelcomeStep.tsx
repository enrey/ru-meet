import React from 'react';
import { Lock, Sparkles, Cpu } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { OnboardingContainer } from '../OnboardingContainer';
import { useOnboarding } from '@/contexts/OnboardingContext';
import { Locale, UI_LOCALES, useI18n } from '@/lib/i18n';

export function WelcomeStep() {
  const { goNext } = useOnboarding();
  const { t, locale, setLocale } = useI18n();

  const features = [
    {
      icon: Lock,
      title: t('Your data never leaves your device'),
    },
    {
      icon: Sparkles,
      title: t('Intelligent summaries & insights'),
    },
    {
      icon: Cpu,
      title: t('Works offline, no cloud required'),
    },
  ];

  return (
    <OnboardingContainer
      title={t('Welcome to Meetily')}
      description={t('Record. Transcribe. Summarize. All on your device.')}
      step={1}
      hideProgress={true}
    >
      <div className="flex flex-col items-center space-y-10">
        {/* Interface language: a clean install starts in Russian */}
        <div role="radiogroup" aria-label={t('Interface language')} className="flex gap-1 rounded-full border border-gray-200 bg-white p-1 text-sm">
          {UI_LOCALES.map(option => (
            <button
              key={option.value}
              type="button"
              role="radio"
              aria-checked={locale === option.value}
              onClick={() => setLocale(option.value as Locale)}
              className={`rounded-full px-3 py-1 transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-gray-900 ${locale === option.value ? 'bg-gray-900 text-white' : 'text-gray-600 hover:text-gray-900'}`}
            >
              {option.label}
            </button>
          ))}
        </div>

        {/* Divider */}
        <div className="w-16 h-px bg-gray-300" />

        {/* Features Card */}
        <div className="w-full max-w-md bg-white rounded-lg border border-gray-200 shadow-sm p-6 space-y-4">
          {features.map((feature, index) => {
            const Icon = feature.icon;
            return (
              <div key={index} className="flex items-start gap-3">
                <div className="flex-shrink-0 mt-0.5">
                  <div className="w-5 h-5 rounded-full bg-gray-100 flex items-center justify-center">
                    <Icon className="w-3 h-3 text-gray-700" />
                  </div>
                </div>
                <p className="text-sm text-gray-700 leading-relaxed">{feature.title}</p>
              </div>
            );
          })}
        </div>

        {/* CTA Section */}
        <div className="w-full max-w-xs space-y-3">
          <Button
            onClick={goNext}
            className="w-full h-11 bg-gray-900 hover:bg-gray-800 text-white"
          >
            {t('Get Started')}
          </Button>
          <p className="text-xs text-center text-gray-500">{t('Takes less than 3 minutes')}</p>
        </div>
      </div>
    </OnboardingContainer>
  );
}
