ALTER TABLE transcripts ADD COLUMN diarization_source TEXT;
ALTER TABLE transcripts ADD COLUMN diarization_parent TEXT;
ALTER TABLE transcripts ADD COLUMN speaker_edited INTEGER NOT NULL DEFAULT 0;
ALTER TABLE diarization_turns ADD COLUMN original_speaker TEXT;
