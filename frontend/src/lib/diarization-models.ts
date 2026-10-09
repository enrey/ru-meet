export const DIARIZATION_MODELS = {
  'pyannote-wespeaker': {
    name: 'polyvoice (PyAnnote + WeSpeaker)',
    size: '~8.7 MB',
    description: 'Compact INT8 segmentation and voice embeddings. Fastest on CPU, but may split one person into several speakers.',
    recommended: false,
  },
  'speakrs-pyannote-wespeaker': {
    name: 'speakrs (PyAnnote + WeSpeaker)',
    size: '~57 MB',
    description: 'Full pyannote community-1 pipeline with PLDA and VBx. Most accurate speaker count; slower, runs on a single CPU thread.',
    /** Shown as the recommended choice: speaker counts are what users judge. */
    recommended: true,
  },
  'nvidia-sortformer-v2': {
    name: 'NVIDIA Sortformer v2',
    size: '492 MB',
    description: 'End-to-end neural diarization for meetings with up to four speakers.',
    recommended: false,
  },
} as const;

export type DiarizationEngineId = keyof typeof DIARIZATION_MODELS;

export const DEFAULT_DIARIZATION_ENGINE: DiarizationEngineId = 'pyannote-wespeaker';

export function getDiarizationModelInfo(engine: string) {
  return DIARIZATION_MODELS[engine as DiarizationEngineId] ?? DIARIZATION_MODELS[DEFAULT_DIARIZATION_ENGINE];
}
