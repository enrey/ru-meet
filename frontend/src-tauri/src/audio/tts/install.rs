//! Installs what reading aloud needs: the llama.cpp runtime with `llama-tts`,
//! a Qwen3-TTS checkpoint and the reference voice it clones.
//!
//! Everything is pinned: the runtime to one llama.cpp release with its
//! SHA-256, the checkpoint to one Hugging Face revision with its file sizes.
//! Files land in a temporary name first, so an interrupted download never
//! looks installed.

use super::commands::{models_root, runtime_root, speaker_path};
use super::qwen::Variant;
use anyhow::{anyhow, bail, Result};
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Runtime};
use tokio::io::AsyncWriteExt;

/// The release the app was validated against (see `qwen.rs` for why the
/// released binaries rather than our bindings).
const LLAMA_RELEASE: &str = "b11120";

struct RuntimeArchive {
    name: &'static str,
    size: u64,
    sha256: &'static str,
}

#[cfg(target_os = "windows")]
const RUNTIME: Option<RuntimeArchive> = Some(RuntimeArchive {
    name: "llama-b11120-bin-win-vulkan-x64.zip",
    size: 31_971_307,
    sha256: "626d6cf928f5e98ebcaf550231a0f86c83e7a364ebabbc6674e707f64f795515",
});
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const RUNTIME: Option<RuntimeArchive> = Some(RuntimeArchive {
    name: "llama-b11120-bin-macos-arm64.tar.gz",
    size: 11_205_150,
    sha256: "f34a2df471a1a6cb4fafd5dacd5f63a54fdc43971235df4d05634ec760eff106",
});
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const RUNTIME: Option<RuntimeArchive> = Some(RuntimeArchive {
    name: "llama-b11120-bin-macos-x64.tar.gz",
    size: 11_235_287,
    sha256: "538689fdbb2695a416f4c57baaaa4cf0368774ea22f7180c2ef0e5e1f3eccdfd",
});
#[cfg(not(any(
    target_os = "windows",
    all(target_os = "macos", any(target_arch = "aarch64", target_arch = "x86_64"))
)))]
const RUNTIME: Option<RuntimeArchive> = None;

struct ModelFile {
    /// Name inside the variant directory, as `qwen.rs` expects it.
    local: &'static str,
    remote: &'static str,
    size: u64,
}

struct Checkpoint {
    repo: &'static str,
    revision: &'static str,
    files: [ModelFile; 2],
}

/// Only the 1.7B checkpoint is published in the format `llama-tts` reads
/// (a backbone plus an `mmproj` codec); the 0.6B one is installed by hand.
fn checkpoint(variant: Variant) -> Option<Checkpoint> {
    match variant {
        Variant::Large => Some(Checkpoint {
            repo: "ggml-org/Qwen3-TTS-12Hz-1.7B-Base-GGUF",
            revision: "ca27d74bc954b73dadab5b71ca265d87fc861a7c",
            files: [
                ModelFile {
                    local: "model.gguf",
                    remote: "Qwen3-TTS-12Hz-1.7B-Base-Q4_K_M.gguf",
                    size: 1_035_965_280,
                },
                ModelFile {
                    local: "mmproj.gguf",
                    remote: "mmproj-Qwen3-TTS-12Hz-1.7B-Base-Q8_0.gguf",
                    size: 446_422_912,
                },
            ],
        }),
        Variant::Small => None,
    }
}

pub(super) fn is_downloadable(variant: Variant) -> bool {
    RUNTIME.is_some() && checkpoint(variant).is_some()
}

/// The reference recording the Base checkpoints clone.
const DEFAULT_VOICE: &[u8] = include_bytes!("../../../assets/tts-default-voice.wav");

static DOWNLOADING: AtomicBool = AtomicBool::new(false);

struct Busy;

impl Drop for Busy {
    fn drop(&mut self) {
        DOWNLOADING.store(false, Ordering::SeqCst);
    }
}

/// Byte progress across every file of one install.
struct Progress<'a, R: Runtime> {
    app: &'a AppHandle<R>,
    model: &'static str,
    total: u64,
    done: u64,
}

impl<R: Runtime> Progress<'_, R> {
    fn emit(&self, current: u64, message: &str) {
        let percent = ((self.done + current) * 100 / self.total.max(1)).min(99);
        let _ = self.app.emit(
            "tts-download-progress",
            serde_json::json!({ "model": self.model, "progress": percent, "message": message }),
        );
    }
}

/// Install the runtime, the voice and `variant`'s checkpoint, skipping what
/// is already on disk.
pub(super) async fn install<R: Runtime>(app: &AppHandle<R>, variant: Variant) -> Result<()> {
    if DOWNLOADING.swap(true, Ordering::SeqCst) {
        bail!("The speech model is already downloading");
    }
    let _busy = Busy;
    let runtime = RUNTIME
        .as_ref()
        .ok_or_else(|| anyhow!("Reading aloud is not available on this platform"))?;
    let checkpoint = checkpoint(variant)
        .ok_or_else(|| anyhow!("{} cannot be downloaded; install it by hand", variant.label()))?;

    let runtime_dir = runtime_root(app)?;
    let model_dir = models_root(app)?.join(variant.id());
    let runtime_missing = !runtime_dir.join(super::qwen::executable_name()).is_file();
    let missing: Vec<&ModelFile> = checkpoint
        .files
        .iter()
        .filter(|file| !has_size(&model_dir.join(file.local), file.size))
        .collect();
    let mut progress = Progress {
        app,
        model: variant.id(),
        total: missing.iter().map(|file| file.size).sum::<u64>()
            + if runtime_missing { runtime.size } else { 0 },
        done: 0,
    };
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(3600))
        .build()?;

    if runtime_missing {
        install_runtime(&client, runtime, &runtime_dir, &mut progress).await?;
    }
    tokio::fs::create_dir_all(&model_dir).await?;
    for file in missing {
        let url = format!(
            "https://huggingface.co/{}/resolve/{}/{}",
            checkpoint.repo, checkpoint.revision, file.remote
        );
        let message = format!("Downloading the speech model… ({})", file.remote);
        download(&client, &url, &model_dir.join(file.local), file.size, None, &mut |bytes| {
            progress.emit(bytes, &message)
        })
        .await?;
        progress.done += file.size;
    }

    let voice = speaker_path(app)?;
    if !voice.is_file() {
        if let Some(parent) = voice.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&voice, DEFAULT_VOICE).await?;
    }
    let _ = app.emit(
        "tts-download-progress",
        serde_json::json!({ "model": variant.id(), "progress": 100, "message": "The speech model is ready" }),
    );
    Ok(())
}

fn has_size(path: &Path, size: u64) -> bool {
    std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.len() == size)
}

async fn install_runtime<R: Runtime>(
    client: &reqwest::Client,
    runtime: &RuntimeArchive,
    runtime_dir: &Path,
    progress: &mut Progress<'_, R>,
) -> Result<()> {
    let parent = runtime_dir
        .parent()
        .ok_or_else(|| anyhow!("The speech runtime directory has no parent"))?;
    tokio::fs::create_dir_all(parent).await?;
    let archive = parent.join(format!(".{}", runtime.name));
    let url = format!(
        "https://github.com/ggml-org/llama.cpp/releases/download/{LLAMA_RELEASE}/{}",
        runtime.name
    );
    download(client, &url, &archive, runtime.size, Some(runtime.sha256), &mut |bytes| {
        progress.emit(bytes, "Downloading the speech runtime…")
    })
    .await?;
    progress.done += runtime.size;

    let target = runtime_dir.to_path_buf();
    let name = runtime.name;
    tokio::task::spawn_blocking(move || {
        let result = unpack_runtime(&archive, name, &target);
        let _ = std::fs::remove_file(&archive);
        result
    })
    .await?
}

/// Unpack into a staging directory next to `runtime_dir` and swap it in
/// only once `llama-tts` is there.
fn unpack_runtime(archive: &Path, name: &str, runtime_dir: &Path) -> Result<()> {
    let staging = runtime_dir.with_file_name(".tts-runtime.partial");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;
    extract(archive, name, &staging)?;
    if !staging.join(super::qwen::executable_name()).is_file() {
        bail!("The speech runtime archive has no {}", super::qwen::executable_name());
    }
    let _ = std::fs::remove_dir_all(runtime_dir);
    std::fs::rename(&staging, runtime_dir)?;
    Ok(())
}

/// Keep `llama-tts` and the libraries it loads, flattened into `destination`.
fn extract(archive: &Path, name: &str, destination: &Path) -> Result<()> {
    let wanted = |file: &str| {
        file == super::qwen::executable_name()
            || file.ends_with(".dll")
            || file.ends_with(".dylib")
    };
    if name.ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(archive)?)?;
        for index in 0..zip.len() {
            let mut entry = zip.by_index(index)?;
            let Some(file) = entry
                .enclosed_name()
                .and_then(|path| path.file_name().map(|name| name.to_string_lossy().into_owned()))
            else {
                continue;
            };
            if entry.is_file() && wanted(&file) {
                let mut output = std::fs::File::create(destination.join(&file))?;
                std::io::copy(&mut entry, &mut output)?;
            }
        }
    } else {
        let gzip = flate2::read::GzDecoder::new(std::fs::File::open(archive)?);
        let mut tar = tar::Archive::new(gzip);
        for entry in tar.entries()? {
            let mut entry = entry?;
            let Some(file) = entry
                .path()?
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
            else {
                continue;
            };
            // Symlinks too: the dylibs are linked by their versioned names.
            if wanted(&file) {
                entry.unpack(destination.join(&file))?;
            }
        }
    }
    Ok(())
}

/// Stream `url` into `destination`, checking the size and, when given, the
/// SHA-256 before the file takes its real name.
async fn download(
    client: &reqwest::Client,
    url: &str,
    destination: &Path,
    expected: u64,
    sha256: Option<&str>,
    on_bytes: &mut (dyn FnMut(u64) + Send),
) -> Result<()> {
    let partial: PathBuf = destination.with_extension("downloading");
    let _ = tokio::fs::remove_file(&partial).await;
    let response = client.get(url).send().await?.error_for_status()?;
    let mut stream = response.bytes_stream();
    let mut output = tokio::fs::File::create(&partial).await?;
    let mut hasher = Sha256::new();
    let mut received = 0_u64;
    let mut reported = 0_u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        output.write_all(&chunk).await?;
        hasher.update(&chunk);
        received += chunk.len() as u64;
        if received - reported >= 4 << 20 {
            reported = received;
            on_bytes(received.min(expected));
        }
    }
    output.flush().await?;
    drop(output);

    let digest = format!("{:x}", hasher.finalize());
    let problem = if received != expected {
        Some(format!("received {received} bytes, expected {expected}"))
    } else if sha256.is_some_and(|sha256| sha256 != digest) {
        Some("the checksum does not match".to_string())
    } else {
        None
    };
    if let Some(problem) = problem {
        let _ = tokio::fs::remove_file(&partial).await;
        bail!("Download of {url} failed: {problem}");
    }
    tokio::fs::rename(&partial, destination).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_voice_is_a_wav_file() {
        assert_eq!(&DEFAULT_VOICE[..4], b"RIFF");
        assert_eq!(&DEFAULT_VOICE[8..12], b"WAVE");
    }


    /// Checks and unpacks a runtime archive fetched separately: set
    /// `TTS_RUNTIME_ARCHIVE` to its path and run with `--ignored`.
    #[test]
    #[ignore]
    fn a_fetched_runtime_archive_matches_the_pin_and_unpacks() {
        let runtime = RUNTIME.as_ref().expect("no runtime for this platform");
        let archive = std::path::PathBuf::from(std::env::var("TTS_RUNTIME_ARCHIVE").unwrap());
        let bytes = std::fs::read(&archive).unwrap();
        assert_eq!(bytes.len() as u64, runtime.size);
        assert_eq!(format!("{:x}", Sha256::digest(&bytes)), runtime.sha256);
        let root = tempfile::tempdir().unwrap();
        let runtime_dir = root.path().join("tts-runtime");
        unpack_runtime(&archive, runtime.name, &runtime_dir).unwrap();
        let executable = runtime_dir.join(super::super::qwen::executable_name());
        let output = std::process::Command::new(&executable).arg("--version").output().unwrap();
        let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        assert!(text.contains("11120"), "unexpected llama-tts --version output: {text}");
    }
}
