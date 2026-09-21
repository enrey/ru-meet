export const DIARIZATION_MODELS = {
  'pyannote-wespeaker': {
    name: 'PyAnnote + WeSpeaker',
    size: '~8.7 MB',
    description: 'Compact speaker segmentation and voice embeddings for local processing.',
  },
  'nvidia-sortformer-v2': {
    name: 'NVIDIA Sortformer v2',
    size: '492 MB',
    description: 'End-to-end neural diarization for meetings with up to four speakers.',
  },
} as const;

export type DiarizationEngineId = keyof typeof DIARIZATION_MODELS;

export function getDiarizationModelInfo(engine: string) {
  return DIARIZATION_MODELS[engine as DiarizationEngineId] ?? DIARIZATION_MODELS['pyannote-wespeaker'];
}
