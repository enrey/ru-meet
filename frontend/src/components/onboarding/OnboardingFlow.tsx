import { useOnboarding } from '@/contexts/OnboardingContext';
import { usePlatform } from '@/hooks/usePlatform';
import {
  WelcomeStep,
  PermissionsStep,
  DownloadProgressStep,
  SetupOverviewStep,
} from './steps';
import { useI18n } from '@/lib/i18n';

interface OnboardingFlowProps {
  onComplete: () => void;
}

export function OnboardingFlow({ onComplete }: OnboardingFlowProps) {
  const { currentStep, databaseReady } = useOnboarding();
  const { t } = useI18n();
  const isMac = usePlatform() === 'macos';

  // 4-Step Onboarding Flow (System-Recommended Models):
  // Step 1: Welcome - Introduce Meetily features
  // Step 2: Setup Overview - Database initialization + show recommended downloads
  // Step 3: Download Progress - Download Parakeet + Summary Model (auto-selected based on platform/RAM)
  // Step 4: Permissions - Request mic + system audio (macOS only)

  if (!databaseReady) {
    return <div className="flex h-screen items-center justify-center text-sm text-slate-500">{t('Preparing local storage…')}</div>;
  }

  return (
    <div className="onboarding-flow">
      {currentStep === 1 && <WelcomeStep />}
      {currentStep === 2 && <SetupOverviewStep isMac={isMac} />}
      {currentStep === 3 && <DownloadProgressStep isMac={isMac} />}
      {currentStep === 4 && isMac && <PermissionsStep />}
    </div>
  );
}
