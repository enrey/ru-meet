import { useEffect, useState } from 'react';
import { Check, Globe2, Languages } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { OnboardingContainer } from '../OnboardingContainer';
import { useOnboarding } from '@/contexts/OnboardingContext';
import { cn } from '@/lib/utils';
import { DIARIZATION_MODELS } from '@/lib/diarization-models';

const ENGINES: Array<{ id: 'gigaam' | 'parakeet'; icon: typeof Languages; name: string; detail: string; size: string }> = [
  { id: 'gigaam', icon: Languages, name: 'GigaAM v3', detail: 'Russian speech', size: '~186 MB' },
  { id: 'parakeet', icon: Globe2, name: 'Parakeet TDT v3', detail: 'Multilingual speech', size: '~670 MB' },
];

export function SetupOverviewStep() {
  const {
    goNext, goPrevious, transcriptionProvider, setTranscriptionProvider,
    downloadTranscription, setDownloadTranscription, downloadSummary, setDownloadSummary,
    downloadDiarization, setDownloadDiarization, diarizationEngine, setDiarizationEngine,
  } = useOnboarding();
  const [isMac, setIsMac] = useState(false);

  useEffect(() => {
    import('@tauri-apps/plugin-os')
      .then(({ platform }) => setIsMac(typeof platform === 'function' ? platform() === 'macos' : navigator.userAgent.includes('Mac')))
      .catch(() => setIsMac(navigator.userAgent.includes('Mac')));
  }, []);

  return (
    <OnboardingContainer
      title="Choose your models"
      description="Choose a transcription engine and what to download now. You can add models later in Settings."
      step={2}
      totalSteps={isMac ? 4 : 3}
    >
      <div className="mx-auto w-full max-w-lg space-y-5">
        <fieldset className="space-y-3 rounded-lg border border-gray-200 bg-white p-4">
          <legend className="px-1 font-medium text-gray-900">Download now</legend>
          <label className="flex cursor-pointer items-start gap-3 text-sm">
            <input type="checkbox" checked={downloadTranscription} onChange={(event) => setDownloadTranscription(event.target.checked)} className="mt-0.5" />
            <span>Selected transcription model<span className="block text-gray-500">Needed for recording and transcription</span></span>
          </label>
          <div className="space-y-2 pl-6">
            {ENGINES.map((engine) => {
              const Icon = engine.icon;
              const selected = transcriptionProvider === engine.id;
              return (
                <label
                  key={engine.id}
                  className={cn(
                    'flex cursor-pointer items-center gap-3 rounded-xl border px-3 py-2.5 transition-all duration-200',
                    selected ? 'border-gray-900 bg-gray-100' : 'border-gray-200 bg-white hover:border-gray-300'
                  )}
                >
                  <input
                    type="radio"
                    name="transcription-engine"
                    checked={selected}
                    onChange={() => setTranscriptionProvider(engine.id)}
                    className="sr-only"
                  />
                  <div className={cn('flex size-8 shrink-0 items-center justify-center rounded-full', selected ? 'bg-gray-200' : 'bg-gray-50')}>
                    <Icon className={cn('h-4 w-4', selected ? 'text-gray-900' : 'text-gray-500')} />
                  </div>
                  <div className="min-w-0 flex-1">
                    <div className="text-sm font-medium text-gray-900">{engine.name}</div>
                    <div className="text-xs text-gray-500">{engine.detail} · {engine.size}</div>
                  </div>
                  <div className={cn('flex size-5 shrink-0 items-center justify-center rounded-full', selected ? 'bg-gray-900' : 'bg-gray-100')}>
                    {selected && <Check className="h-3 w-3 text-white" />}
                  </div>
                </label>
              );
            })}
          </div>
          <label className="flex cursor-pointer items-start gap-3 text-sm"><input type="checkbox" checked={downloadDiarization} onChange={(event) => setDownloadDiarization(event.target.checked)} /><span>Speaker diarization models<span className="block text-gray-500">Optional; identify speakers after a recording</span></span></label>
          {downloadDiarization && <label className="block pl-6 text-sm text-gray-700">Diarization engine
            <select value={diarizationEngine} onChange={(event) => setDiarizationEngine(event.target.value as typeof diarizationEngine)} className="mt-1 block w-full rounded-md border border-gray-300 bg-white px-3 py-2">
              {Object.entries(DIARIZATION_MODELS).map(([id, model]) => (
                <option key={id} value={id}>{model.name} · {model.size}</option>
              ))}
            </select>
          </label>}
          <label className="flex cursor-pointer items-start gap-3 text-sm"><input type="checkbox" checked={downloadSummary} onChange={(event) => setDownloadSummary(event.target.checked)} /><span>Local summarization model<span className="block text-gray-500">Optional; you can set up a model or external provider later</span></span></label>
        </fieldset>
        <div className="flex gap-3"><Button variant="outline" onClick={goPrevious}>Back</Button><Button onClick={goNext} className="h-11 flex-1 bg-gray-900 text-white hover:bg-gray-800">Continue</Button></div>
      </div>
    </OnboardingContainer>
  );
}
