import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button } from '@/components/ui/button';
import { OnboardingContainer } from '../OnboardingContainer';
import { useOnboarding } from '@/contexts/OnboardingContext';
import { DIARIZATION_MODELS } from '@/lib/diarization-models';
import { getSummaryModelSizeLabel } from '@/lib/onboarding-summary-model';
import { useI18n } from '@/lib/i18n';

const ENGINES: Array<{ id: 'gigaam' | 'parakeet'; name: string; detail: string; size: string }> = [
  { id: 'gigaam', name: 'GigaAM v3', detail: 'Russian speech', size: '~186 MB' },
  { id: 'parakeet', name: 'Parakeet TDT v3', detail: 'Multilingual speech', size: '~670 MB' },
];

const SUMMARY_MODELS = [
  { id: 'qwen3.5:4b', name: 'Qwen 3.5 4B', tier: 'High quality' },
  { id: 'qwen3.5:2b', name: 'Qwen 3.5 2B', tier: 'Balanced' },
];

export function SetupOverviewStep() {
  const {
    goNext, goPrevious, transcriptionProvider, setTranscriptionProvider,
    downloadTranscription, setDownloadTranscription, downloadSummary, setDownloadSummary,
    downloadDiarization, setDownloadDiarization, diarizationEngine, setDiarizationEngine,
    downloadTts, setDownloadTts,
    selectedSummaryModel, recommendedSummaryModel, selectSummaryModel,
  } = useOnboarding();
  const { t } = useI18n();
  const [isMac, setIsMac] = useState(false);
  /** Null until known: the installer ships the model on Windows only. */
  const [vadBundled, setVadBundled] = useState<boolean | null>(null);

  useEffect(() => {
    invoke<{ bundled: boolean }>('vad_model_status')
      .then(({ bundled }) => setVadBundled(bundled))
      .catch(() => setVadBundled(false));
  }, []);

  useEffect(() => {
    import('@tauri-apps/plugin-os')
      .then(({ platform }) => setIsMac(typeof platform === 'function' ? platform() === 'macos' : navigator.userAgent.includes('Mac')))
      .catch(() => setIsMac(navigator.userAgent.includes('Mac')));
  }, []);

  return (
    <OnboardingContainer
      title={t('Choose your models')}
      description={t('Choose a transcription engine and what to download now. You can add models later in Settings.')}
      step={2}
      totalSteps={isMac ? 4 : 3}
    >
      <div className="mx-auto w-full max-w-lg space-y-5">
        <fieldset className="space-y-3 rounded-lg border border-slate-200 bg-white p-4">
          <legend className="px-1 font-medium text-slate-900">{t('Download now')}</legend>
          <label className="flex cursor-pointer items-start gap-3 text-sm">
            <input type="checkbox" checked={downloadTranscription} onChange={(event) => setDownloadTranscription(event.target.checked)} className="mt-0.5" />
            <span>{t('Selected transcription model')}<span className="block text-slate-500">{t('Needed for recording and transcription')}</span></span>
          </label>
          <label className="block pl-6 text-sm text-slate-700">{t('Transcription engine')}
            <select value={transcriptionProvider} onChange={(event) => setTranscriptionProvider(event.target.value as typeof transcriptionProvider)} className="mt-1 block w-full rounded-md border border-slate-300 bg-white px-3 py-2">
              {ENGINES.map((engine) => (
                <option key={engine.id} value={engine.id}>{engine.name} ({t(engine.detail)}) · {engine.size}</option>
              ))}
            </select>
          </label>
          <label className="flex cursor-pointer items-start gap-3 text-sm"><input type="checkbox" checked={downloadDiarization} onChange={(event) => setDownloadDiarization(event.target.checked)} /><span>{t('Speaker diarization models')}<span className="block text-slate-500">{t('Optional; identify speakers after a recording')}</span></span></label>
          {downloadDiarization && <label className="block pl-6 text-sm text-slate-700">{t('Diarization engine')}
            <select value={diarizationEngine} onChange={(event) => setDiarizationEngine(event.target.value as typeof diarizationEngine)} className="mt-1 block w-full rounded-md border border-slate-300 bg-white px-3 py-2">
              {Object.entries(DIARIZATION_MODELS).map(([id, model]) => (
                <option key={id} value={id}>{model.name} · {model.size}</option>
              ))}
            </select>
          </label>}
          <label className="flex cursor-pointer items-start gap-3 text-sm"><input type="checkbox" checked={downloadSummary} onChange={(event) => setDownloadSummary(event.target.checked)} /><span>{t('Local summarization model')}<span className="block text-slate-500">{t('Optional; you can set up a model or external provider later')}</span></span></label>
          {downloadSummary && <label className="block pl-6 text-sm text-slate-700">{t('Summarization model')}
            <select value={selectedSummaryModel} onChange={(event) => selectSummaryModel(event.target.value)} className="mt-1 block w-full rounded-md border border-slate-300 bg-white px-3 py-2">
              {SUMMARY_MODELS.map((model) => (
                <option key={model.id} value={model.id}>
                  {model.name} ({t(model.tier)}) · {getSummaryModelSizeLabel(model.id)}{model.id === recommendedSummaryModel ? ` · ${t('Recommended')}` : ''}
                </option>
              ))}
            </select>
          </label>}
          <label className="flex cursor-pointer items-start gap-3 text-sm"><input type="checkbox" checked={downloadTts} onChange={(event) => setDownloadTts(event.target.checked)} /><span>{t('Speech model for reading summaries aloud')}<span className="block text-slate-500">{t('Optional; Qwen3-TTS 1.7B · ~1.5 GB')}</span></span></label>
          <label className="flex cursor-not-allowed items-start gap-3 text-sm"><input type="checkbox" checked disabled /><span>{t('Speech detection model')}<span className="block text-slate-500">Silero VAD v5.1.2{vadBundled === null ? '' : vadBundled ? ` · ${t('Built into the installer')}` : ` · ~2.3 MB · ${t('Required for recording')}`}</span></span></label>
        </fieldset>
        <div className="flex gap-3"><Button variant="outline" onClick={goPrevious}>{t('Back')}</Button><Button onClick={goNext} className="h-11 flex-1 bg-indigo-600 text-white hover:bg-indigo-700">{t('Continue')}</Button></div>
      </div>
    </OnboardingContainer>
  );
}
