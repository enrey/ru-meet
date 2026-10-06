//! Speech synthesis with Qwen3-TTS, run through llama.cpp's `llama-tts`.
//!
//! The pipeline is two GGUF models - a backbone LLM that turns text into
//! semantic codes and a codec that turns those codes into a waveform - and
//! llama.cpp runs both on the GPU through Vulkan. Both models are "Base"
//! checkpoints, which means the voice is not a name but a short reference
//! recording they clone, so every synthesis takes a speaker file.
//!
//! Why a separate process rather than the `llama-helper` sidecar this app
//! already runs: the pipeline lives in llama.cpp's mtmd layer, and the version
//! of it vendored by our Rust bindings crashes in the Vulkan backend on this
//! graph (`GGML_ASSERT` in `GET_ROWS`, fixed upstream later). The released
//! binaries do not, so the app drives those until the bindings catch up.
//!
//! Measured on an RTX 5060 Ti: 22 frames per second against the 12 needed for
//! real time, so synthesis runs comfortably ahead of playback.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Which checkpoint to speak with. Both are Base models; they differ in size,
/// and so in how natural they sound and how fast they run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Variant {
    /// Qwen3-TTS 12Hz 0.6B - the default: lighter and quicker.
    #[serde(rename = "0.6b")]
    Small,
    /// Qwen3-TTS 12Hz 1.7B - slower, more natural.
    #[serde(rename = "1.7b")]
    Large,
}

impl Variant {
    pub fn id(self) -> &'static str {
        match self {
            Variant::Small => "0.6b",
            Variant::Large => "1.7b",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Variant::Small => "Qwen3-TTS 0.6B (faster)",
            Variant::Large => "Qwen3-TTS 1.7B (more natural)",
        }
    }

    pub fn all() -> [Variant; 2] {
        [Variant::Small, Variant::Large]
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Variant::all().into_iter().find(|variant| variant.id() == id)
    }
}

/// Everything the engine needs on disk, resolved once.
pub struct QwenTts {
    executable: PathBuf,
    model: PathBuf,
    codec: PathBuf,
    speaker: PathBuf,
    variant: Variant,
}

impl QwenTts {
    /// `runtime` holds the llama.cpp binaries, `models` the unpacked
    /// checkpoints, `speaker` the reference recording to clone.
    pub fn load(runtime: &Path, models: &Path, speaker: &Path, variant: Variant) -> Result<Self> {
        let executable = runtime.join(executable_name());
        if !executable.is_file() {
            bail!(
                "The speech runtime is missing: {}",
                executable.display()
            );
        }
        let directory = models.join(variant.id());
        let model = directory.join("model.gguf");
        let codec = directory.join("mmproj.gguf");
        for (what, path) in [("model", &model), ("codec", &codec)] {
            if !path.is_file() {
                bail!("The {what} for {} is missing: {}", variant.id(), path.display());
            }
        }
        if !speaker.is_file() {
            bail!("The reference voice is missing: {}", speaker.display());
        }

        Ok(Self {
            executable,
            model,
            codec,
            speaker: speaker.to_path_buf(),
            variant,
        })
    }

    pub fn variant(&self) -> Variant {
        self.variant
    }

    /// Speak one chunk of text straight into `output`, which receives a WAV.
    pub fn synthesize(&self, text: &str, output: &Path) -> Result<()> {
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        let mut command = Command::new(&self.executable);
        command
            .arg("-m")
            .arg(&self.model)
            .arg("-mm")
            .arg(&self.codec)
            // Everything on the GPU: on CPU this model runs slower than the
            // speech it produces, and the codec is the worst of it.
            .arg("-ngl")
            .arg("99")
            .arg("--mmproj-offload")
            .arg("--tts-lang")
            .arg("russian")
            .arg("--tts-speaker-file")
            .arg(&self.speaker)
            .arg("-p")
            .arg(text)
            .arg("-o")
            .arg(output);

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        let result = command
            .output()
            .with_context(|| format!("could not run {}", self.executable.display()))?;

        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            let reason = stderr
                .lines()
                .rev()
                .find(|line| line.contains("error") || line.contains("failed"))
                .unwrap_or("see the log for details");
            return Err(anyhow!("speech synthesis failed: {reason}"));
        }
        if !output.is_file() {
            bail!("speech synthesis produced no audio");
        }
        Ok(())
    }
}

fn executable_name() -> &'static str {
    if cfg!(windows) {
        "llama-tts.exe"
    } else {
        "llama-tts"
    }
}
