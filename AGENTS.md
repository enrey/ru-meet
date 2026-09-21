# Agent Instructions

## Scope

- The supported product is the self-contained Rust/Tauri desktop app in `frontend/`; the removed Python/FastAPI backend is not an implementation target.
- Add frontend-facing backend behavior through Tauri commands/events and Rust services under `frontend/src-tauri/src/`.
- `frontend/AGENTS.md` adds generated Next.js-specific instructions for that subtree.
- Windows and MacOS production platforms are the first class citizens.

## Package Manager and Commands

- Run frontend commands from `frontend/` and use **pnpm** with `frontend/pnpm-lock.yaml`.

| Task | Command |
|---|---|
| Install dependencies | `pnpm install --frozen-lockfile` |
| Type-check frontend | `pnpm exec tsc --noEmit` |
| Check Rust workspace | `cargo check --workspace --target-dir target/agent-validation` (from repository root) |
| Test Rust workspace | `cargo test --workspace --target-dir target/agent-validation` (from repository root) |
| Production build | `pnpm run tauri:build` |

- Use the `herdr` skill to start the app and inspect its logs. The development app runs, or may already be running, as `pnpm run tauri:dev` in the `debug_and_run` pane.
- Reserve the default `target/` artifacts for `pnpm run tauri:dev`; run agent Cargo checks and tests only with `--target-dir target/agent-validation` so validation cannot invalidate the interactive development cache.

## External References

| Need | File |
|---|---|
| Product and source-build overview | `README.md` |
| Platform build requirements | `docs/BUILDING.md` |
| GPU configuration | `docs/GPU_ACCELERATION.md` |
| High-level architecture | `docs/architecture.md` |
| Current CI behavior | `.github/workflows/` |

- Write ADRs under `docs/adr/` in Russian.

## Architecture and Data

- Audio capture, transcription, persistence, diarization, and summary orchestration live in the Tauri core; the UI communicates through Tauri commands and events.
- Keep recording audio and VAD-filtered transcription as separate pipeline paths in `frontend/src-tauri/src/audio/pipeline.rs`.
- Diarization engines `PyAnnote + WeSpeaker` (`polyvoice`) and `NVIDIA Sortformer v2` (`parakeet-rs`) implement `DiarizationEngine` in `frontend/src-tauri/src/audio/diarization.rs`.
- Completed speaker turns are stored in `diarization_turns`; the meeting timeline reads them independently of paginated transcript segments.
- Resolve application/model storage through `frontend/src-tauri/src/portable.rs` and Tauri path APIs; do not hardcode OS paths or introduce an independent data root.

## Critical Invariants

- Branding uses `productName`/`mainBinaryName = "ru_meet"`, but keep Tauri `identifier = "com.meetily.ai"` and Cargo package `meetily`; changing either breaks data/update continuity or internal tooling.
- Parakeet, GigaAM, and Silero VAD must share one compatible `ort` version and Windows ONNX Runtime bundle. After dependency changes, run `cargo tree -i ort` and confirm only one version.
- Keep Windows dynamic ONNX Runtime loading and resource-relative DLL resolution aligned with `frontend/src-tauri/build/onnxruntime.rs` and `frontend/src-tauri/tauri.windows.conf.json`.
- Keep Silero VAD pinned and hash/size verified in both `frontend/src-tauri/src/audio/vad.rs` and `frontend/src-tauri/build/silero_vad.rs`; validate a candidate model with real speech before upgrading it.
- Never call `reqwest::blocking` directly on a Tokio worker; use `tokio::task::block_in_place` as in `frontend/src-tauri/src/audio/vad.rs`.
- SQL migrations are immutable exact-byte inputs. New `frontend/src-tauri/migrations/*.sql` files must use LF as enforced by `.gitattributes`; never rewrite an applied migration.
- Use `perf_debug!`/`perf_trace!` for hot-path Rust logging and keep the `microphone`/`system` audio-device terminology.

## Release Notes

- `.github/workflows/release.yml` creates a draft release; publishing it for updater visibility remains manual.
- Keep executable-name assumptions in `.github/workflows/` and `scripts/build_portable.ps1` synchronized with `mainBinaryName`.
