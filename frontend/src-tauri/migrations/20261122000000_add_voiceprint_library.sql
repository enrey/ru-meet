-- Local voiceprint library: people the user taught the app to recognise.
CREATE TABLE IF NOT EXISTS voice_profiles (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- One confirmed sample: a normalized embedding of a single-speaker interval of
-- a meeting recording. Audio is not copied; the interval points into the
-- meeting's own recording and loses that link if the meeting is deleted.
CREATE TABLE IF NOT EXISTS voice_samples (
    id TEXT PRIMARY KEY,
    profile_id TEXT NOT NULL REFERENCES voice_profiles(id) ON DELETE CASCADE,
    extractor TEXT NOT NULL,
    embedding BLOB NOT NULL,
    meeting_id TEXT REFERENCES meetings(id) ON DELETE SET NULL,
    start_time REAL NOT NULL,
    end_time REAL NOT NULL,
    created_at TEXT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_voice_samples_source
    ON voice_samples(extractor, meeting_id, start_time, end_time);
CREATE INDEX IF NOT EXISTS idx_voice_samples_profile
    ON voice_samples(profile_id);

-- Link between a speaker label of one meeting and a profile, kept apart from
-- the label text. Only 'confirmed' links have renamed the speaker.
CREATE TABLE IF NOT EXISTS meeting_speaker_profiles (
    meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    speaker TEXT NOT NULL,
    profile_id TEXT NOT NULL REFERENCES voice_profiles(id) ON DELETE CASCADE,
    status TEXT NOT NULL CHECK (status IN ('suggested', 'confirmed', 'rejected')),
    similarity REAL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (meeting_id, speaker)
);

CREATE INDEX IF NOT EXISTS idx_meeting_speaker_profiles_profile
    ON meeting_speaker_profiles(profile_id);
