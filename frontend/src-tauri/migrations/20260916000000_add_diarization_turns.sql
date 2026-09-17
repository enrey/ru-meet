CREATE TABLE IF NOT EXISTS diarization_turns (
    meeting_id TEXT NOT NULL,
    start_time REAL NOT NULL,
    end_time REAL NOT NULL,
    speaker TEXT NOT NULL,
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_diarization_turns_meeting
    ON diarization_turns(meeting_id, start_time);
