use sha2::{Digest, Sha256};
use std::{
    env,
    fs::{self, File},
    io::{Read, Write},
    path::Path,
};

// Keep in sync with `SILERO_VAD_MODEL_URL`/`_SHA256`/`_SIZE` in
// `src/audio/vad.rs` - see that file for why this is pinned to v5.1.2
// instead of `master`.
const MODEL_URL: &str = "https://raw.githubusercontent.com/snakers4/silero-vad/v5.1.2/src/silero_vad/data/silero_vad.onnx";
const MODEL_SHA256: &str = "2623a2953f6ff3d2c1e61740c6cdb7168133479b267dfef114a4a3cc5bdd788f";
const MODEL_SIZE: u64 = 2_327_524;

/// Downloads and bundles the Silero VAD model at build time so it ships in
/// the installer instead of being fetched (and silently able to go stale
/// or fail) on a user's machine at first recording. Windows-only for now,
/// matching `onnxruntime::ensure_onnxruntime_runtime` - macOS/Linux keep
/// downloading it via `ensure_silero_model` in `audio/vad.rs`.
pub fn ensure_silero_vad_model() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    println!("cargo:rerun-if-changed=binaries/silero");

    let manifest_dir =
        env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR environment variable not set");
    let destination_dir = Path::new(&manifest_dir).join("binaries").join("silero");
    let destination = destination_dir.join("silero_vad.onnx");

    if verify_file(&destination).is_ok() {
        println!(
            "cargo:warning=Using verified bundled Silero VAD model at {}",
            destination.display()
        );
        return;
    }

    fs::create_dir_all(&destination_dir)
        .expect("Failed to create binaries/silero directory");

    let temporary_path = destination_dir.join(".silero_vad.onnx.download");
    let _ = fs::remove_file(&temporary_path);

    if let Err(error) = download(&temporary_path) {
        let _ = fs::remove_file(&temporary_path);
        panic!("Failed to download Silero VAD model: {error}");
    }
    if let Err(error) = verify_downloaded_file(&temporary_path) {
        let _ = fs::remove_file(&temporary_path);
        panic!("Downloaded Silero VAD model failed verification: {error}");
    }
    fs::rename(&temporary_path, &destination).unwrap_or_else(|error| {
        panic!(
            "Failed to finalize bundled Silero VAD model at {}: {error}",
            destination.display()
        )
    });

    println!(
        "cargo:warning=Bundled verified Silero VAD model at {}",
        destination.display()
    );
}

fn download(destination: &Path) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|error| format!("failed to create download client: {error}"))?;
    let mut response = client
        .get(MODEL_URL)
        .send()
        .map_err(|error| format!("failed to download Silero VAD model: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Silero VAD model download failed: {error}"))?;
    let mut file = File::create(destination)
        .map_err(|error| format!("failed to create {}: {error}", destination.display()))?;
    std::io::copy(&mut response, &mut file)
        .map_err(|error| format!("failed to write {}: {error}", destination.display()))?;
    file.flush()
        .map_err(|error| format!("failed to flush {}: {error}", destination.display()))?;
    Ok(())
}

fn verify_downloaded_file(path: &Path) -> Result<(), String> {
    verify_file(path)
}

fn verify_file(path: &Path) -> Result<(), String> {
    let metadata =
        fs::metadata(path).map_err(|error| format!("failed to inspect {path:?}: {error}"))?;
    if metadata.len() != MODEL_SIZE {
        return Err(format!(
            "size {} != expected {MODEL_SIZE}",
            metadata.len()
        ));
    }

    let mut file =
        File::open(path).map_err(|error| format!("failed to open {path:?}: {error}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("failed to hash {path:?}: {error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    let actual_sha256 = format!("{:x}", hasher.finalize());
    if actual_sha256 != MODEL_SHA256 {
        return Err(format!("SHA-256 {actual_sha256} != expected {MODEL_SHA256}"));
    }

    Ok(())
}
