Look @CLAUDE.md

## Current architecture notes (keep in sync)

- The supported product is the Rust/Tauri desktop app under `frontend/`; the
  Python/FastAPI `backend/` is legacy only.
- ONNX workloads share `ort = 2.0.0-rc.12`: Parakeet, direct official Silero
  VAD (`silero_vad.onnx`), and GigaAM via `transcribe-rs`. Do not reintroduce
  the `silero-rs` wrapper, which pinned an incompatible ORT release.
- On Windows, `ort::init_from(...).commit()` can block while loading the
  dynamic runtime DLL. It must stay off Tauri's setup/UI thread; it is started
  in the `onnx-runtime-init` native background thread in
  `frontend/src-tauri/src/lib.rs`. Blocking it during setup causes a permanent
  white, "Not responding" window before DevTools can open.
- Speaker diarization offers `PyAnnote + WeSpeaker` through `polyvoice` and
  `NVIDIA Sortformer v2` through `parakeet-rs`. Both implement the same
  `DiarizationEngine` abstraction. Completed speaker turns are stored in
  `diarization_turns`; the meeting timeline reads them independently of
  paginated transcript segments.
- `parakeet-rs` uses `tokenizers` with default features disabled. Keep
  `esaxx-rs` transitive only: a direct dependency with default features enables
  its C++ build (`/MT` on MSVC), conflicting with the app's `/MD` runtime.
  `tokenizers` uses the Rust suffix-array implementation without that feature;
  no vendored `esaxx-rs` patch is needed.
- `SpeakerTimeline` on the meeting details page loads turns through
  `get_meeting_speaker_turns` and refreshes on `diarization-labels-saved`.
  Older meetings without turn rows fall back to labeled transcript intervals.
  Clicking a turn locates a phrase via `api_find_transcript_at_time`; the
  paginated transcript hook loads the matching page before scrolling.
- Speaker renaming goes through `rename_meeting_speaker`: the transcript
  repository updates `diarization_turns` and `transcripts` in one transaction
  and rejects names already used in that meeting. The frontend also patches
  the currently loaded transcript window, preserving pagination and scroll
  position; do not replace this with a full refetch on rename.
