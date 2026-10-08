import React from 'react';
import { ChevronLeft, ChevronRight, X } from 'lucide-react';
import { cn } from '@/lib/utils';
import { ProgressIndicator } from './shared/ProgressIndicator';
import { useOnboarding } from '@/contexts/OnboardingContext';
import type { OnboardingContainerProps } from '@/types/onboarding';
import { useI18n } from '@/lib/i18n';

export function OnboardingContainer({
  title,
  description,
  children,
  step,
  totalSteps = 5,
  stepOffset = 0,
  hideProgress = false,
  className,
  showNavigation = false,
  onNext,
  onPrevious,
  canGoNext = true,
  canGoPrevious = true,
}: OnboardingContainerProps) {
  const { goToStep, goPrevious, goNext, isPreview, endPreview } = useOnboarding();
  const { t } = useI18n();

  React.useEffect(() => {
    if (!isPreview) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') endPreview();
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [isPreview, endPreview]);

  const handlePrevious = () => {
    if (onPrevious) {
      onPrevious();
    } else {
      goPrevious();
    }
  };

  const handleNext = () => {
    if (onNext) {
      onNext();
    } else {
      goNext();
    }
  };

  const handleStepClick = (s: number) => {
    goToStep(s + stepOffset);
  };

  return (
    <div className="fixed inset-0 bg-slate-50 flex items-center justify-center z-50 overflow-hidden">
      {isPreview && (
        <button
          type="button"
          onClick={endPreview}
          aria-label={t('Close preview')}
          title={t('Close preview')}
          className="absolute right-4 top-4 z-10 grid h-9 w-9 place-items-center rounded-full text-slate-500 transition-colors hover:bg-slate-200 hover:text-slate-900"
        >
          <X className="h-5 w-5" />
        </button>
      )}
      <div className={cn('w-full max-w-2xl h-full max-h-screen flex flex-col px-6 py-6', className)}>
        {/* Progress Indicator with Navigation - Fixed */}
        {step && !hideProgress && (
          <div className="mb-2 relative flex-shrink-0">
            {/* Navigation Buttons */}
            {showNavigation && (
              <div className="absolute top-1/2 -translate-y-1/2 left-0 right-0 flex justify-between pointer-events-none">
                <button
                  onClick={handlePrevious}
                  disabled={!canGoPrevious || step === 1}
                  className={cn(
                    'pointer-events-auto w-8 h-8 rounded-full bg-white border border-slate-200 shadow-sm flex items-center justify-center transition-all duration-200',
                    canGoPrevious && step !== 1
                      ? 'hover:bg-slate-50 hover:shadow-md hover:scale-110 text-slate-700'
                      : 'opacity-0 cursor-not-allowed'
                  )}
                >
                  <ChevronLeft className="w-4 h-4" />
                </button>

                <button
                  onClick={handleNext}
                  disabled={!canGoNext || step === totalSteps}
                  className={cn(
                    'pointer-events-auto w-8 h-8 rounded-full bg-white border border-slate-200 shadow-sm flex items-center justify-center transition-all duration-200',
                    canGoNext && step !== totalSteps
                      ? 'hover:bg-slate-50 hover:shadow-md hover:scale-110 text-slate-700'
                      : 'opacity-0 cursor-not-allowed'
                  )}
                >
                  <ChevronRight className="w-4 h-4" />
                </button>
              </div>
            )}

            {/* Progress Indicator */}
            <ProgressIndicator current={step} total={totalSteps} onStepClick={handleStepClick} />
          </div>
        )}

        {/* Header - Fixed */}
        <div className="mb-4 text-center space-y-3 flex-shrink-0">
          <h1 className="text-4xl font-semibold text-slate-900 animate-fade-in-up">{title}</h1>
          {description && (
            <p className="text-base text-slate-600 max-w-md mx-auto animate-fade-in-up delay-75">
              {description}
            </p>
          )}
        </div>

        {/* Content - Scrollable */}
        <div className="flex-1 overflow-y-auto pr-2">
          <div className="space-y-6">{children}</div>
        </div>
      </div>
    </div>
  );
}
