//! Saved-audio computation in this executable's headless worker mode.
//! All attempt lifecycle and recording preemption stay here, outside workflows.
use crate::{
    audio::{
        diarization::SpeakerTurn,
        transcription::{ProviderId, TranscriptionEngine},
    },
    background_process::Process,
};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::Arc,
    time::Duration,
};
use tauri::{AppHandle, Manager, Runtime};
pub(crate) const VAD_REDEMPTION_TIME_MS: u32 = 2000;
const FLAG: &str = "--background-worker";
const PREFIX: &str = "RU_MEET_WORKER ";

#[derive(Clone, Serialize, Deserialize)]
pub(crate) enum Request {
    Transcribe {
        path: PathBuf,
        copy_to: Option<PathBuf>,
        provider: ProviderId,
        model: String,
        language: Option<String>,
        allow_empty: bool,
    },
    Speakers {
        path: PathBuf,
        engine: String,
        collapse: bool,
    },
    Summary {
        data: PathBuf,
        model: String,
        system: String,
        user: String,
    },
    PackSpeech {
        folder: PathBuf,
        chunks: Vec<(usize, String)>,
        sentences: Vec<PathBuf>,
    },
    #[cfg(test)]
    Probe {
        marker: PathBuf,
        descendant: PathBuf,
        fail: bool,
    },
}
#[derive(Serialize, Deserialize)]
pub(crate) enum Output {
    Transcript {
        duration_seconds: f64,
        transcripts: Vec<(String, f64, f64)>,
        speech_segments: usize,
    },
    Speakers(Vec<SpeakerTurn>),
    Summary(String),
    Audio {
        name: String,
        duration: f32,
        words: Vec<crate::audio::tts::summary_audio::SpokenWord>,
    },
    #[cfg(test)]
    Probe,
}
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Progress {
    pub stage: String,
    pub percentage: u32,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<crate::summary::summary_engine::sidecar::GenerationProgress>,
}
#[derive(Serialize, Deserialize)]
struct Envelope {
    request: Request,
    models: PathBuf,
    diarization_models: PathBuf,
    runtime: Option<PathBuf>,
    silero: Option<PathBuf>,
    result: PathBuf,
}
fn progress(stage: &str, percentage: u32, message: impl Into<String>) {
    let value = Progress {
        stage: stage.into(),
        percentage,
        message: message.into(),
        generation: None,
    };
    let mut stdout = std::io::stdout().lock();
    if let Ok(json) = serde_json::to_string(&value) {
        let _ = writeln!(stdout, "{PREFIX}{json}");
        let _ = stdout.flush();
    }
}

pub(crate) async fn transcribe<R: Runtime>(
    app: &AppHandle<R>,
    path: PathBuf,
    copy_to: Option<PathBuf>,
    provider: Option<&str>,
    model: Option<&str>,
    language: Option<String>,
    allow_empty: bool,
    keep_running: impl Fn() -> bool,
    on_progress: impl Fn(Progress),
) -> Result<(f64, Vec<(String, f64, f64)>, usize)> {
    let (provider, model) = crate::audio::transcription::worker_config(app, provider, model)
        .await
        .map_err(|e| anyhow!(e))?;
    match run(
        app,
        Request::Transcribe {
            path,
            copy_to,
            provider,
            model,
            language,
            allow_empty,
        },
        keep_running,
        on_progress,
    )
    .await?
    {
        Output::Transcript {
            duration_seconds,
            transcripts,
            speech_segments,
        } => Ok((duration_seconds, transcripts, speech_segments)),
        _ => bail!("Unexpected worker result"),
    }
}
pub(crate) async fn speakers<R: Runtime>(
    app: &AppHandle<R>,
    path: PathBuf,
    engine: String,
    collapse: bool,
    keep_running: impl Fn() -> bool,
    on_progress: impl Fn(Progress),
) -> Result<Vec<SpeakerTurn>> {
    match run(
        app,
        Request::Speakers {
            path,
            engine,
            collapse,
        },
        keep_running,
        on_progress,
    )
    .await?
    {
        Output::Speakers(turns) => Ok(turns),
        _ => bail!("Unexpected worker result"),
    }
}
async fn run<R: Runtime>(
    app: &AppHandle<R>,
    request: Request,
    keep_running: impl Fn() -> bool,
    on_progress: impl Fn(Progress),
) -> Result<Output> {
    let root = crate::portable::app_data_dir(app)?.join("worker-jobs");
    std::fs::create_dir_all(&root)?;
    let work = tempfile::tempdir_in(root)?;
    let envelope = Envelope {
        request,
        models: crate::portable::app_data_dir(app)?.join("models"),
        diarization_models: crate::audio::diarization::models_root()?,
        runtime: app
            .path()
            .resolve("onnxruntime.dll", tauri::path::BaseDirectory::Resource)
            .ok(),
        silero: app
            .path()
            .resolve(
                "silero/silero_vad.onnx",
                tauri::path::BaseDirectory::Resource,
            )
            .ok(),
        result: work.path().join("result.json"),
    };
    execute(
        envelope,
        || {
            let mut command = Command::new(std::env::current_exe()?);
            command.arg(FLAG);
            Ok(command)
        },
        &crate::background_work::scheduler(),
        keep_running,
        on_progress,
    )
    .await
}
async fn execute(
    envelope: Envelope,
    command: impl Fn() -> Result<Command>,
    scheduler: &Arc<crate::background_work::Scheduler>,
    keep_running: impl Fn() -> bool,
    on_progress: impl Fn(Progress),
) -> Result<Output> {
    let input = serde_json::to_vec(&envelope)?;
    loop {
        let permit = crate::background_work::acquire_while_on(scheduler, &keep_running)
            .await
            .ok_or_else(|| anyhow!("Background processing cancelled"))?;
        let interrupted = permit.interruption();
        let _ = std::fs::remove_file(&envelope.result);
        let mut command = command()?;
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let (process, stdout) = Process::spawn(&mut command)?;
        let stop = process.clone();
        permit.stop_on_recording(Arc::new(move || stop.stop()));
        let (sender, mut events) = tokio::sync::mpsc::unbounded_channel();
        std::thread::Builder::new()
            .name("worker-progress".into())
            .spawn(move || {
                if let Some(stdout) = stdout {
                    for line in std::io::BufReader::new(stdout).lines() {
                        let Ok(line) = line else { break };
                        if let Some(json) = line.strip_prefix(PREFIX) {
                            if let Ok(value) = serde_json::from_str::<Progress>(json) {
                                let _ = sender.send(value);
                            }
                        }
                    }
                }
            })?;
        // Assignment to the process group and stop registration precede any work.
        if !interrupted.is_cancelled() {
            if let Err(error) = process.send(&input) {
                if !interrupted.is_cancelled() {
                    return Err(error);
                }
            }
        }
        let mut events_open = true;
        let mut tick = tokio::time::interval(Duration::from_millis(50));
        let result = loop {
            tokio::select! {
                biased;
                _ = interrupted.cancelled() => {
                    process.stopped().await;
                    break None;
                }
                event = events.recv(), if events_open => match event {
                    Some(event) => on_progress(event),
                    None => events_open = false,
                },
                _ = tick.tick() => {
                    if !keep_running() {
                        process.stopped().await;
                        bail!("Background processing cancelled");
                    }
                    if let Some(status) = process.try_wait()? {
                        process.stopped().await; // no orphaned helpers between attempts
                        // A completed attempt cannot be committed after recording preempted it.
                        if interrupted.is_cancelled() { break None; }
                        let output: Result<Output, String> = std::fs::read(&envelope.result)
                            .with_context(|| format!("Background worker exited with {status} without a result"))
                            .and_then(|bytes| serde_json::from_slice(&bytes).map_err(Into::into))?;
                        if !status.success() { return Err(anyhow!(output.err().unwrap_or_else(|| format!("Background worker exited with {status}")))); }
                        break Some(output.map_err(|e| anyhow!(e))?);
                    }
                }
            }
        };
        drop(permit);
        drop(process);
        if !keep_running() {
            bail!("Background processing cancelled");
        }
        if let Some(output) = result {
            return Ok(output);
        }
        on_progress(Progress {
            stage: "paused".into(),
            percentage: 0,
            generation: None,
            message: "Paused for recording; this task will restart automatically".into(),
        });
    }
}

/// Called before Tauri starts. Worker never opens a window or database.
pub fn run_if_requested() {
    if !std::env::args_os().any(|arg| arg == FLAG) {
        return;
    }
    let result = serve();
    if let Err(error) = &result {
        log::error!("Background worker: {error:#}");
    }
    std::process::exit(if result.is_ok() { 0 } else { 1 });
}
fn serve() -> Result<()> {
    let mut stdin = std::io::BufReader::new(std::io::stdin());
    let mut line = String::new();
    stdin.read_line(&mut line)?;
    let envelope: Envelope = serde_json::from_str(&line).context("Invalid worker request")?;
    // Parent death closes stdin. Also stop any decoder descendants on Unix.
    std::thread::spawn(move || {
        let mut remaining = String::new();
        use std::io::Read;
        let _ = stdin.read_to_string(&mut remaining);
        #[cfg(unix)]
        unsafe {
            libc::kill(-libc::getpgrp(), libc::SIGKILL);
        }
        std::process::exit(1);
    });
    let result = (|| -> Result<Output> {
        #[cfg(test)]
        if let Request::Probe {
            marker,
            descendant,
            fail,
        } = &envelope.request
        {
            if *fail {
                bail!("synthetic inference error");
            }
            if marker.exists() {
                return Ok(Output::Probe);
            }
            let mut command = Command::new(std::env::current_exe()?);
            command
                .args([
                    "--exact",
                    "background_worker::tests::fixture",
                    "--ignored",
                    "--nocapture",
                ])
                .env("RU_MEET_DESCENDANT", descendant)
                .stdin(Stdio::null())
                .stdout(Stdio::null());
            let _child = command.spawn()?;
            std::fs::write(marker, b"started")?;
            progress("probe", 0, "started");
            std::thread::sleep(Duration::from_secs(30));
            return Ok(Output::Probe);
        }
        if !matches!(
            envelope.request,
            Request::Summary { .. } | Request::PackSpeech { .. }
        ) {
            crate::init_worker_onnx(envelope.runtime.as_deref())?;
            crate::audio::vad::configure_worker(envelope.models.clone(), envelope.silero.clone());
            crate::audio::diarization::configure_worker(envelope.diarization_models.clone());
        }
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        runtime.block_on(compute(envelope.request, envelope.models))
    })()
    .map_err(|error| format!("{error:#}"));
    let bytes = serde_json::to_vec(&result)?;
    std::fs::write(&envelope.result, bytes)?;
    result.map(|_| ()).map_err(|error| anyhow!(error))
}
async fn compute(request: Request, models: PathBuf) -> Result<Output> {
    match request {
        Request::PackSpeech {
            folder,
            chunks,
            sentences,
        } => {
            progress("encoding", 90, "Assembling summary audio");
            let (name, duration, words) =
                crate::audio::tts::summary_audio::pack_in_worker(&folder, &chunks, &sentences)?;
            Ok(Output::Audio {
                name,
                duration,
                words,
            })
        }
        Request::Summary {
            data,
            model,
            system,
            user,
        } => {
            crate::summary::summary_engine::client::set_progress_emitter(Box::new(|generation| {
                let event = Progress {
                    stage: "generating".into(),
                    percentage: 0,
                    message: "Generating summary".into(),
                    generation: Some(generation),
                };
                let mut stdout = std::io::stdout().lock();
                if let Ok(json) = serde_json::to_string(&event) {
                    let _ = writeln!(stdout, "{PREFIX}{json}");
                    let _ = stdout.flush();
                }
            }));
            let result = crate::summary::summary_engine::generate_with_builtin(
                &data, &model, &system, &user, None,
            )
            .await?;
            Ok(Output::Summary(result))
        }
        Request::Speakers {
            path,
            engine,
            collapse,
        } => {
            progress("decoding", 5, "Decoding audio for speaker diarization");
            let turns = crate::audio::diarization::compute_in_worker(&path, &engine, collapse)?;
            Ok(Output::Speakers(turns))
        }
        Request::Transcribe {
            path,
            copy_to,
            provider,
            model,
            language,
            allow_empty,
        } => {
            let path = if let Some(destination) = copy_to {
                progress("copying", 10, "Copying audio file");
                let staging = destination.with_extension("worker-copy");
                if !destination.is_file() {
                    std::fs::copy(path, &staging)?;
                    std::fs::rename(&staging, &destination)?;
                }
                destination
            } else {
                path
            };
            let decoded = crate::audio::decoder::decode_audio_file_with_progress(
                &path,
                Some(Box::new(|p, m| progress("decoding", 15 + p / 20, m))),
            )?;
            let duration_seconds = decoded.duration_seconds;
            let samples = decoded.to_whisper_format_with_progress(Some(Box::new(|p, m| {
                progress("resampling", 20 + p / 20, m)
            })));
            drop(decoded);
            progress("vad", 25, "Detecting speech");
            let segments = crate::audio::vad::get_speech_chunks_with_progress(
                &samples,
                VAD_REDEMPTION_TIME_MS,
                |p, count| {
                    progress(
                        "vad",
                        25 + p / 20,
                        format!("Detecting speech: {p}% ({count} segments)"),
                    );
                    true
                },
            )?;
            drop(samples);
            let speech_segments = segments.len();
            if speech_segments == 0 && !allow_empty {
                bail!("No speech detected in audio file");
            }
            let mut pieces = Vec::new();
            for (index, segment) in segments.into_iter().enumerate() {
                if segment.samples.len() > 25 * 16000 {
                    pieces.extend(
                        crate::audio::common::split_segment_at_silence(&segment, 25 * 16000)
                            .into_iter()
                            .map(|piece| (index, piece)),
                    );
                } else {
                    pieces.push((index, segment));
                }
            }
            let mut transcripts = Vec::new();
            if !pieces.is_empty() {
                progress("transcribing", 30, "Loading transcription engine");
                let engine = TranscriptionEngine::for_worker(models, provider, &model)
                    .await
                    .map_err(|e| anyhow!(e))?;
                let count = pieces.len();
                for (i, (source, piece)) in pieces.into_iter().enumerate() {
                    progress(
                        "transcribing",
                        30 + (i * 50 / count) as u32,
                        format!("Transcribing segment {} of {count}", i + 1),
                    );
                    if piece.samples.len() < 1600 {
                        continue;
                    }
                    let result = engine
                        .transcribe(piece.samples, language.clone())
                        .await
                        .map_err(|e| anyhow!(e))?;
                    if !result.text.trim().is_empty() {
                        transcripts.push((
                            source,
                            result.text,
                            piece.start_timestamp_ms,
                            piece.end_timestamp_ms,
                        ));
                    }
                }
            }
            Ok(Output::Transcript {
                duration_seconds,
                transcripts: crate::audio::common::rejoin_split_utterances(transcripts),
                speech_segments,
            })
        }
        #[cfg(test)]
        Request::Probe { .. } => unreachable!(),
    }
}

/// Built-in LLM goes through the same killable tree, including llama-helper.
pub(crate) async fn summary(
    data: &PathBuf,
    model: &str,
    system: &str,
    user: &str,
    cancellation: Option<&tokio_util::sync::CancellationToken>,
) -> Result<String, String> {
    let run = async {
        let root = data.join("worker-jobs");
        std::fs::create_dir_all(&root)?;
        let work = tempfile::tempdir_in(root)?;
        let envelope = Envelope {
            request: Request::Summary {
                data: data.clone(),
                model: model.into(),
                system: system.into(),
                user: user.into(),
            },
            models: data.join("models"),
            diarization_models: data.join("models/diarization"),
            runtime: None,
            silero: None,
            result: work.path().join("result.json"),
        };
        let output = execute(
            envelope,
            || {
                let mut command = Command::new(std::env::current_exe()?);
                command.arg(FLAG);
                Ok(command)
            },
            &crate::background_work::scheduler(),
            || cancellation.is_none_or(|token| !token.is_cancelled()),
            |event| {
                if let Some(generation) = event.generation {
                    crate::summary::summary_engine::client::emit_progress(generation);
                }
            },
        )
        .await?;
        match output {
            Output::Summary(text) => Ok(text),
            _ => bail!("Unexpected summary worker result"),
        }
    };
    run.await.map_err(|error: anyhow::Error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::time::{timeout, Instant};
    // Production admits one heavyweight process; keep fixtures serial too.
    static CHILD_TESTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    fn fixture_command() -> Result<Command> {
        let mut command = Command::new(std::env::current_exe()?);
        command.args([
            "--exact",
            "background_worker::tests::fixture",
            "--ignored",
            "--nocapture",
        ]);
        Ok(command)
    }
    fn envelope(root: &std::path::Path, fail: bool) -> Envelope {
        Envelope {
            request: Request::Probe {
                marker: root.join("started"),
                descendant: root.join("descendant"),
                fail,
            },
            models: root.into(),
            diarization_models: root.into(),
            runtime: None,
            silero: None,
            result: root.join("result.json"),
        }
    }
    #[test]
    #[ignore = "subprocess fixture; invoked by lifecycle tests"]
    fn fixture() {
        if let Some(path) = std::env::var_os("RU_MEET_BLOCK_INPUT") {
            std::fs::write(path, b"started").unwrap();
            std::thread::sleep(Duration::from_secs(30));
            return;
        }
        if let Some(path) = std::env::var_os("RU_MEET_DESCENDANT") {
            std::thread::sleep(Duration::from_millis(900));
            std::fs::write(path, b"descendant survived").unwrap();
            return;
        }
        if let Err(error) = serve() {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
    #[tokio::test]
    async fn full_input_pipe_does_not_block_recording_preemption() {
        let _serial = CHILD_TESTS.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("not-reading");
        let mut command = fixture_command().unwrap();
        command
            .env("RU_MEET_BLOCK_INPUT", &marker)
            .stdin(Stdio::piped())
            .stdout(Stdio::null());
        let (process, _) = Process::spawn(&mut command).unwrap();
        timeout(Duration::from_secs(45), async {
            while !marker.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let scheduler = Arc::new(crate::background_work::Scheduler::default());
        let permit = crate::background_work::acquire_while_on(&scheduler, || true)
            .await
            .unwrap();
        let stop = process.clone();
        permit.stop_on_recording(Arc::new(move || stop.stop()));
        let sender = process.clone();
        let write = std::thread::spawn(move || sender.send(&vec![b'x'; 2 * 1024 * 1024]));
        tokio::time::sleep(Duration::from_millis(100)).await;
        let started = Instant::now();
        let recording = scheduler.reserve_recording();
        assert!(started.elapsed() < Duration::from_millis(150));
        timeout(Duration::from_secs(5), process.stopped())
            .await
            .unwrap();
        assert!(write.join().unwrap().is_err());
        drop(recording);
        drop(permit);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn native_worker_decodes_and_runs_bundled_vad_without_loading_asr_for_silence() {
        let _serial = CHILD_TESTS.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("silence.wav");
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36_u32 + 64000).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16_u32.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&16000_u32.to_le_bytes());
        wav.extend_from_slice(&32000_u32.to_le_bytes());
        wav.extend_from_slice(&2_u16.to_le_bytes());
        wav.extend_from_slice(&16_u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&64000_u32.to_le_bytes());
        wav.resize(44 + 64000, 0);
        std::fs::write(&path, wav).unwrap();
        let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries");
        let scheduler = Arc::new(crate::background_work::Scheduler::default());
        for allow_empty in [true, false] {
            let mut input = envelope(dir.path(), false);
            input.request = Request::Transcribe {
                path: path.clone(),
                copy_to: None,
                provider: ProviderId::Whisper,
                model: "no-model-needed-for-silence".into(),
                language: None,
                allow_empty,
            };
            input.runtime = Some(resources.join("onnxruntime/onnxruntime.dll"));
            input.silero = Some(resources.join("silero/silero_vad.onnx"));
            let output = timeout(
                Duration::from_secs(30),
                execute(input, fixture_command, &scheduler, || true, |_| {}),
            )
            .await
            .unwrap();
            if allow_empty {
                match output.unwrap() {
                    Output::Transcript {
                        duration_seconds,
                        transcripts,
                        speech_segments,
                    } => {
                        assert!((duration_seconds - 2.0).abs() < 0.01);
                        assert!(transcripts.is_empty());
                        assert_eq!(speech_segments, 0);
                    }
                    _ => panic!("wrong worker result"),
                }
            } else {
                assert!(output
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("No speech detected"));
            }
        }
        let packed = dir.path().join("packed");
        let mut input = envelope(dir.path(), false);
        input.request = Request::PackSpeech {
            folder: packed.clone(),
            chunks: vec![(0, "one two".into())],
            sentences: vec![path],
        };
        let output = timeout(
            Duration::from_secs(30),
            execute(input, fixture_command, &scheduler, || true, |_| {}),
        )
        .await
        .unwrap()
        .unwrap();
        match output {
            Output::Audio {
                name,
                duration,
                words,
            } => {
                assert!(packed.join(name).is_file());
                assert!((duration - 2.0).abs() < 0.01);
                assert_eq!(words.len(), 2);
                assert_eq!(words[0].text, "one");
            }
            _ => panic!("wrong pack result"),
        }
    }

    #[tokio::test]
    async fn recording_preempts_worker_tree_immediately_and_same_job_retries() {
        let _serial = CHILD_TESTS.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("started");
        let descendant = dir.path().join("descendant");
        let scheduler = Arc::new(crate::background_work::Scheduler::default());
        let runner_scheduler = scheduler.clone();
        let input = envelope(dir.path(), false);
        let run = tokio::spawn(async move {
            execute(input, fixture_command, &runner_scheduler, || true, |_| {}).await
        });
        timeout(Duration::from_secs(45), async {
            while !marker.exists() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("fixture never started");
        let start = Instant::now();
        let recording = scheduler.reserve_recording();
        assert!(
            start.elapsed() < Duration::from_millis(150),
            "recording waited for native computation"
        );
        // Descendant would write its marker while recording if only its parent died.
        tokio::time::sleep(Duration::from_millis(1300)).await;
        assert!(
            !descendant.exists(),
            "decoder descendant continued during recording"
        );
        assert!(
            !run.is_finished(),
            "preemption must pause rather than fail the job"
        );
        drop(recording);
        assert!(matches!(
            timeout(Duration::from_secs(45), run)
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
            Output::Probe
        ));
    }
    #[tokio::test]
    async fn cancellation_while_paused_never_starts_worker() {
        let dir = tempfile::tempdir().unwrap();
        let scheduler = Arc::new(crate::background_work::Scheduler::default());
        let recording = scheduler.reserve_recording();
        let running = Arc::new(AtomicBool::new(true));
        let flag = running.clone();
        let runner_scheduler = scheduler.clone();
        let input = envelope(dir.path(), false);
        let job = tokio::spawn(async move {
            execute(
                input,
                fixture_command,
                &runner_scheduler,
                || flag.load(Ordering::SeqCst),
                |_| {},
            )
            .await
        });
        running.store(false, Ordering::SeqCst);
        let error = timeout(Duration::from_secs(2), job)
            .await
            .unwrap()
            .unwrap()
            .err()
            .unwrap();
        assert!(error.to_string().contains("cancelled"));
        assert!(!dir.path().join("started").exists());
        drop(recording);
    }
    #[tokio::test]
    async fn actual_worker_failure_is_reported_and_does_not_keep_admission() {
        let _serial = CHILD_TESTS.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let scheduler = Arc::new(crate::background_work::Scheduler::default());
        let error = timeout(
            Duration::from_secs(45),
            execute(
                envelope(dir.path(), true),
                fixture_command,
                &scheduler,
                || true,
                |_| {},
            ),
        )
        .await
        .unwrap()
        .err()
        .unwrap();
        assert!(error.to_string().contains("synthetic inference error"));
        std::fs::write(dir.path().join("started"), b"retry").unwrap();
        assert!(matches!(
            timeout(
                Duration::from_secs(45),
                execute(
                    envelope(dir.path(), false),
                    fixture_command,
                    &scheduler,
                    || true,
                    |_| {}
                )
            )
            .await
            .unwrap()
            .unwrap(),
            Output::Probe
        ));
    }
}

/// Assemble in a temporary directory on the meeting's volume; publish only a
/// completed attempt. The worker owns PCM buffers and its FFmpeg descendants.
pub(crate) async fn pack_speech<R: Runtime>(
    app: &AppHandle<R>,
    folder: &std::path::Path,
    chunks: Vec<(usize, String)>,
    sentences: Vec<PathBuf>,
    keep_running: impl Fn() -> bool,
) -> Result<(
    String,
    f32,
    Vec<crate::audio::tts::summary_audio::SpokenWord>,
)> {
    std::fs::create_dir_all(folder)?;
    let work = tempfile::tempdir_in(folder)?;
    let data = crate::portable::app_data_dir(app)?;
    let input = Envelope {
        request: Request::PackSpeech {
            folder: work.path().into(),
            chunks,
            sentences,
        },
        models: data.join("models"),
        diarization_models: crate::audio::diarization::models_root()?,
        runtime: None,
        silero: None,
        result: work.path().join("result.json"),
    };
    let output = execute(
        input,
        || {
            let mut command = Command::new(std::env::current_exe()?);
            command.arg(FLAG);
            Ok(command)
        },
        &crate::background_work::scheduler(),
        &keep_running,
        |_| {},
    )
    .await?;
    let _publish = crate::background_work::acquire_while(&keep_running)
        .await
        .ok_or_else(|| anyhow!("Speech preparation cancelled"))?;
    match output {
        Output::Audio {
            name,
            duration,
            words,
        } => {
            std::fs::rename(work.path().join(&name), folder.join(&name))?;
            Ok((name, duration, words))
        }
        _ => bail!("Unexpected audio worker result"),
    }
}
