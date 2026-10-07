#[path = "build/ffmpeg.rs"]
mod ffmpeg;
#[path = "build/migrations.rs"]
mod migrations;
#[path = "build/onnxruntime.rs"]
mod onnxruntime;
#[path = "build/silero_vad.rs"]
mod silero_vad;

fn main() {
    // GPU Acceleration Detection and Build Guidance
    detect_and_report_gpu_capabilities();

    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-lib=framework=AVFoundation");
        println!("cargo:rustc-link-lib=framework=Cocoa");
        println!("cargo:rustc-link-lib=framework=Foundation");

        // Let the enhanced_macos crate handle its own Swift compilation
        // The swift-rs crate build will be handled in the enhanced_macos crate's build.rs
    }

    // sqlx checksums each migration's exact bytes; keep them canonical so this
    // build cannot disagree with one made on another machine.
    migrations::ensure_lf_line_endings();

    // Download and bundle FFmpeg binary at build-time to eliminate runtime download delays
    ffmpeg::ensure_ffmpeg_binary();
    onnxruntime::ensure_onnxruntime_runtime();
    silero_vad::ensure_silero_vad_model();

    tauri_build::build()
}

/// Detects GPU acceleration capabilities and provides build guidance
fn detect_and_report_gpu_capabilities() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    println!("cargo:warning=🚀 Building Meetily for: {}", target_os);

    match target_os.as_str() {
        "macos" => {
            println!("cargo:warning=Whisper: ✅ macOS: Metal GPU acceleration ENABLED by default");
            #[cfg(feature = "coreml")]
            println!("cargo:warning=Whisper: ✅ CoreML acceleration ENABLED");
        }
        "windows" => {
            if cfg!(feature = "cuda") {
                println!("cargo:warning=Whisper: ✅ Windows: CUDA GPU acceleration ENABLED");
            } else if cfg!(feature = "vulkan") {
                println!("cargo:warning=Whisper: ✅ Windows: Vulkan GPU acceleration ENABLED");
            } else if cfg!(feature = "openblas") {
                println!("cargo:warning=Whisper: ✅ Windows: OpenBLAS CPU optimization ENABLED");
            } else {
                println!(
                    "cargo:warning=Whisper: ⚠️  Windows: Using CPU-only mode (no GPU or BLAS acceleration)"
                );
                println!("cargo:warning=Whisper: 💡 For NVIDIA GPU: cargo build --release --features cuda");
                println!(
                    "cargo:warning=Whisper: 💡 For AMD/Intel GPU: cargo build --release --features vulkan"
                );
                println!("cargo:warning=Whisper: 💡 For CPU optimization: cargo build --release --features openblas");

                // Try to detect NVIDIA GPU
                if which::which("nvidia-smi").is_ok() {
                    println!("cargo:warning=Whisper: 🎯 NVIDIA GPU detected! Consider rebuilding with --features cuda");
                }
            }
        }
        "linux" => {
            if cfg!(feature = "cuda") {
                println!("cargo:warning=Whisper: ✅ Linux: CUDA GPU acceleration ENABLED");
            } else if cfg!(feature = "vulkan") {
                println!("cargo:warning=Whisper: ✅ Linux: Vulkan GPU acceleration ENABLED");
            } else if cfg!(feature = "hipblas") {
                println!("cargo:warning=Whisper: ✅ Linux: AMD ROCm (HIP) acceleration ENABLED");
            } else if cfg!(feature = "openblas") {
                println!("cargo:warning=Whisper: ✅ Linux: OpenBLAS CPU optimization ENABLED");
            } else {
                println!(
                    "cargo:warning=Whisper: ⚠️  Linux: Using CPU-only mode (no GPU or BLAS acceleration)"
                );
                println!("cargo:warning=Whisper: 💡 For NVIDIA GPU: cargo build --release --features cuda");
                println!("cargo:warning=Whisper: 💡 For AMD GPU: cargo build --release --features hipblas");
                println!(
                    "cargo:warning=Whisper: 💡 For other GPUs: cargo build --release --features vulkan"
                );
                println!("cargo:warning=Whisper: 💡 For CPU optimization: cargo build --release --features openblas");

                // Try to detect NVIDIA GPU
                if which::which("nvidia-smi").is_ok() {
                    println!("cargo:warning=Whisper: 🎯 NVIDIA GPU detected! Consider rebuilding with --features cuda");
                }

                // Try to detect AMD GPU
                if which::which("rocm-smi").is_ok() {
                    println!("cargo:warning=Whisper: 🎯 AMD GPU detected! Consider rebuilding with --features hipblas");
                }
            }
        }
        _ => {
            println!("cargo:warning=ℹ️  Unknown platform: {}", target_os);
        }
    }

    // Performance guidance
    if !cfg!(feature = "cuda")
        && !cfg!(feature = "vulkan")
        && !cfg!(feature = "hipblas")
        && !cfg!(feature = "openblas")
        && target_os != "macos"
    {
        println!("cargo:warning=Whisper: 📊 Performance: CPU-only builds are significantly slower than GPU/BLAS builds");
        println!("cargo:warning=Whisper: 📚 See README.md for GPU/BLAS setup instructions");
    }
}
