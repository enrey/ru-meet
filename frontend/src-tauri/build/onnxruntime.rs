use sha2::{Digest, Sha256};
use std::{
    env,
    fs::{self, File},
    io::{Read, Write},
    path::Path,
    process,
};

#[cfg(windows)]
use std::os::windows::fs::MetadataExt;

const WINDOWS_X64_TARGET: &str = "x86_64-pc-windows-msvc";
// GPU-capable onnxruntime.dll for Windows, sourced from the `onnxruntime-node`
// npm package rather than Microsoft's official channels, because neither of
// those has what we need at a compatible version:
//  - microsoft/onnxruntime's own GitHub releases don't publish a DirectML
//    Windows zip at all (only plain CPU and CUDA12/13 variants exist there).
//  - Microsoft's `Microsoft.ML.OnnxRuntime.DirectML` NuGet package does, but
//    its latest release is stuck at onnxruntime 1.24.4 - below the >=1.27.x
//    floor `ort`'s own default `api-27` feature requires (see "ONNX Runtime
//    (`ort`) Version Management" in CLAUDE.md).
//  - `ort`'s own maintainer (pyke) publishes a version-matched (1.28.0)
//    DirectML build for this exact target via their CDN
//    (cdn.pyke.io/0/pyke:ort-rs/ms@1.28.0/x86_64-pc-windows-msvc+directml.tar.lzma2),
//    but it's a *statically linked* `onnxruntime.lib` (341MB) meant for
//    normal link-time linking, not a `load-dynamic`-loadable `.dll` - using
//    it would mean dropping the runtime Resource-relative path resolution
//    this app relies on (see `ensure_onnx_runtime_available` in lib.rs) and
//    statically bloating every Windows install by ~340MB, GPU or not.
// `onnxruntime-node` (Microsoft's own Node.js bindings) bundles a genuine
// dynamic onnxruntime.dll with DirectML support, and at 1.30.0 - newer than
// even the CPU-only 1.28.0 build this replaces. It has no separate
// `onnxruntime_providers_shared.dll`; CPU and DirectML both appear to be
// compiled directly into `onnxruntime.dll` in this build (unverified beyond
// "the app links and starts" - re-check if a future onnxruntime-node release
// changes this). URL/hashes verified by downloading the package directly
// from the npm registry and hashing it (2026-09-19), not copied from a third
// party.
const ARCHIVE_URL: &str =
    "https://registry.npmjs.org/onnxruntime-node/-/onnxruntime-node-1.30.0.tgz";
const ARCHIVE_SHA256: &str = "6e3390d6b783e7be946fad629292799da28d0b42f84856e50d2c1b0383291e75";
const ARCHIVE_SIZE: u64 = 113_507_888;

struct Artifact {
    archive_path: &'static str,
    output_name: &'static str,
    size: u64,
    sha256: &'static str,
}

const ARTIFACTS: [Artifact; 4] = [
    Artifact {
        archive_path: "package/bin/napi-v6/win32/x64/onnxruntime.dll",
        output_name: "onnxruntime.dll",
        size: 28_754_232,
        sha256: "508c362f5673483dd3a086379c392795b2e42d10d5e6f3f90ebd7ac21c97af67",
    },
    Artifact {
        // DirectML execution provider. Resolved implicitly by
        // onnxruntime.dll's own imports (standard Windows DLL search order
        // checks the loading module's own directory) once a session actually
        // requests the DirectML provider - see the `directml` Cargo feature
        // and `ort::execution_providers::DirectMLExecutionProvider` call
        // sites; bundling this file alone does not change any session's
        // behavior.
        archive_path: "package/bin/napi-v6/win32/x64/DirectML.dll",
        output_name: "DirectML.dll",
        size: 18_527_584,
        sha256: "234e8898778cdec88d3cb0539508273494082812c968699f3de665a018971625",
    },
    Artifact {
        // DirectX Shader Compiler: DirectML JIT-compiles its HLSL compute
        // shaders through this at runtime. Without it, DirectML provider
        // registration fails even though onnxruntime.dll itself loads fine -
        // this is not optional.
        archive_path: "package/bin/napi-v6/win32/x64/dxcompiler.dll",
        output_name: "dxcompiler.dll",
        size: 17_986_360,
        sha256: "593d42df78c7f9cbd97c1374af107cfe20985759f98b77afc1448fe41ee3cc76",
    },
    Artifact {
        // DXIL validator, required alongside dxcompiler.dll for the same
        // reason.
        archive_path: "package/bin/napi-v6/win32/x64/dxil.dll",
        output_name: "dxil.dll",
        size: 1_508_664,
        sha256: "cf9a3981263f8ec30c9905d136eeaf4b4573209c602198671b958ec86905dea8",
    },
];

// onnxruntime-node's npm tarball doesn't ship a LICENSE file (unlike the
// official GitHub release zip this used to extract from), so onnxruntime's
// own license is embedded directly - fetched independently from the matching
// v1.30.0 tag; it's a short, stable MIT license that doesn't change between
// releases. DirectML.dll/dxcompiler.dll/dxil.dll are separate Microsoft
// projects (DirectML, DirectXShaderCompiler), also MIT-licensed but not
// bundled here yet - do this properly before a real release build.
const LICENSE_FILE_NAME: &str = "onnxruntime-LICENSE.txt";
const LICENSE_TEXT: &str = "MIT License\n\nCopyright (c) Microsoft Corporation\n\nPermission is hereby granted, free of charge, to any person obtaining a copy\nof this software and associated documentation files (the \"Software\"), to deal\nin the Software without restriction, including without limitation the rights\nto use, copy, modify, merge, publish, distribute, sublicense, and/or sell\ncopies of the Software, and to permit persons to whom the Software is\nfurnished to do so, subject to the following conditions:\n\nThe above copyright notice and this permission notice shall be included in all\ncopies or substantial portions of the Software.\n\nTHE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR\nIMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,\nFITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE\nAUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER\nLIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,\nOUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE\nSOFTWARE.\n";

pub fn ensure_onnxruntime_runtime() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    println!("cargo:rerun-if-changed=binaries/onnxruntime");

    let target = env::var("TARGET").expect("TARGET environment variable not set");
    if target != WINDOWS_X64_TARGET {
        panic!("ONNX Runtime is bundled only for {WINDOWS_X64_TARGET}; got {target}");
    }

    let manifest_dir =
        env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR environment variable not set");
    let destination = Path::new(&manifest_dir)
        .join("binaries")
        .join("onnxruntime");

    match verify_staged_runtime(&destination) {
        Ok(()) => {
            println!(
                "cargo:warning=Using verified bundled ONNX Runtime from {}",
                destination.display()
            );
            return;
        }
        Err(verification_error) => match fs::symlink_metadata(&destination) {
            Ok(metadata) if is_link_or_reparse_point(&metadata) => {
                panic!(
                    "Refusing to replace linked or reparse-point ONNX Runtime stage at {}: {verification_error}",
                    destination.display()
                );
            }
            Ok(metadata) if metadata.is_dir() => {
                println!(
                    "cargo:warning=Replacing invalid bundled ONNX Runtime: {verification_error}"
                );
                fs::remove_dir_all(&destination).unwrap_or_else(|remove_error| {
                    panic!(
                        "Failed to remove invalid ONNX Runtime directory at {}: {remove_error}",
                        destination.display()
                    )
                });
            }
            Ok(metadata) if metadata.is_file() => {
                println!(
                    "cargo:warning=Replacing invalid bundled ONNX Runtime: {verification_error}"
                );
                fs::remove_file(&destination).unwrap_or_else(|remove_error| {
                    panic!(
                        "Failed to remove invalid ONNX Runtime file at {}: {remove_error}",
                        destination.display()
                    )
                });
            }
            Ok(_) => {
                panic!(
                    "Refusing to replace special ONNX Runtime stage at {}: {verification_error}",
                    destination.display()
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                panic!(
                    "Failed to inspect invalid ONNX Runtime stage at {}: {error}",
                    destination.display()
                );
            }
        },
    }

    fs::create_dir_all(
        destination
            .parent()
            .expect("ONNX Runtime destination has no parent"),
    )
    .expect("Failed to create ONNX Runtime binaries directory");

    let temporary_archive = env::temp_dir().join(format!(
        "meetily-onnxruntime-{}-{}.tgz",
        process::id(),
        target
    ));
    let temporary_destination =
        destination.with_file_name(format!(".onnxruntime-{}-{}", process::id(), target));
    let _ = fs::remove_file(&temporary_archive);
    let _ = fs::remove_dir_all(&temporary_destination);

    let result = stage_runtime(&temporary_archive, &temporary_destination);
    let _ = fs::remove_file(&temporary_archive);

    if let Err(error) = result {
        let _ = fs::remove_dir_all(&temporary_destination);
        panic!("Failed to stage bundled ONNX Runtime: {error}");
    }

    fs::rename(&temporary_destination, &destination).unwrap_or_else(|error| {
        let _ = fs::remove_dir_all(&temporary_destination);
        panic!(
            "Failed to finalize bundled ONNX Runtime at {}: {error}",
            destination.display()
        );
    });

    verify_staged_runtime(&destination).unwrap_or_else(|error| {
        panic!("Bundled ONNX Runtime verification failed after staging: {error}")
    });
    println!(
        "cargo:warning=Bundled verified ONNX Runtime at {}",
        destination.display()
    );
}

fn stage_runtime(archive_path: &Path, destination: &Path) -> Result<(), String> {
    download_archive(archive_path)?;
    verify_file(archive_path, ARCHIVE_URL, ARCHIVE_SIZE, ARCHIVE_SHA256)?;

    fs::create_dir_all(destination)
        .map_err(|error| format!("failed to create {}: {error}", destination.display()))?;

    // The npm tarball is a plain .tgz (gzip + tar) containing every platform's
    // binaries (win32/x64, win32/arm64, darwin/arm64, ...) - unlike the old
    // zip source, `tar::Archive` only supports forward iteration (no
    // by-name random access), so this extracts everything matching one of
    // our wanted paths in a single pass and stops once all are found rather
    // than decompressing the full ~340MB of every platform's binaries.
    let archive_file = File::open(archive_path)
        .map_err(|error| format!("failed to open {}: {error}", archive_path.display()))?;
    let decoder = flate2::read::GzDecoder::new(archive_file);
    let mut archive = tar::Archive::new(decoder);
    let entries = archive
        .entries()
        .map_err(|error| format!("failed to read ONNX Runtime archive: {error}"))?;

    let mut remaining: Vec<&Artifact> = ARTIFACTS.iter().collect();
    for entry in entries {
        if remaining.is_empty() {
            break;
        }
        let mut entry =
            entry.map_err(|error| format!("failed to read ONNX Runtime archive entry: {error}"))?;
        let entry_path = entry
            .path()
            .map_err(|error| format!("failed to read ONNX Runtime archive entry path: {error}"))?
            .to_path_buf();

        let Some(position) = remaining
            .iter()
            .position(|artifact| entry_path == Path::new(artifact.archive_path))
        else {
            continue;
        };
        let artifact = remaining.remove(position);

        let output = destination.join(artifact.output_name);
        let mut file = File::create(&output)
            .map_err(|error| format!("failed to create {}: {error}", output.display()))?;
        std::io::copy(&mut entry, &mut file)
            .map_err(|error| format!("failed to extract {}: {error}", artifact.archive_path))?;
        verify_file(
            &output,
            artifact.output_name,
            artifact.size,
            artifact.sha256,
        )?;
    }

    if !remaining.is_empty() {
        let missing: Vec<&str> = remaining.iter().map(|a| a.archive_path).collect();
        return Err(format!(
            "ONNX Runtime archive is missing expected entries: {}",
            missing.join(", ")
        ));
    }

    let license_path = destination.join(LICENSE_FILE_NAME);
    fs::write(&license_path, LICENSE_TEXT).map_err(|error| {
        format!(
            "failed to write {}: {error}",
            license_path.display()
        )
    })?;

    Ok(())
}

fn download_archive(destination: &Path) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|error| format!("failed to create download client: {error}"))?;
    let mut response = client
        .get(ARCHIVE_URL)
        .send()
        .map_err(|error| format!("failed to download ONNX Runtime: {error}"))?
        .error_for_status()
        .map_err(|error| format!("ONNX Runtime download failed: {error}"))?;
    let mut file = File::create(destination)
        .map_err(|error| format!("failed to create {}: {error}", destination.display()))?;
    copy_exact(&mut response, &mut file, ARCHIVE_SIZE)
}

fn copy_exact<R: Read, W: Write>(
    source: &mut R,
    destination: &mut W,
    expected_size: u64,
) -> Result<(), String> {
    let copied = std::io::copy(&mut source.by_ref().take(expected_size), destination)
        .map_err(|error| format!("failed to copy ONNX Runtime download: {error}"))?;
    if copied != expected_size {
        return Err(format!(
            "downloaded ONNX Runtime archive has size {copied}, expected {expected_size}"
        ));
    }

    let mut probe = [0_u8; 1];
    if source
        .read(&mut probe)
        .map_err(|error| format!("failed to copy ONNX Runtime download: {error}"))?
        != 0
    {
        return Err(format!(
            "downloaded ONNX Runtime archive exceeds expected size {expected_size}"
        ));
    }

    Ok(())
}

fn is_link_or_reparse_point(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }

    #[cfg(windows)]
    {
        return metadata.file_attributes() & 0x0000_0400 != 0;
    }

    #[cfg(not(windows))]
    false
}

fn verify_staged_runtime(destination: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(destination)
        .map_err(|error| format!("failed to inspect runtime stage: {error}"))?;
    if is_link_or_reparse_point(&metadata) {
        return Err("runtime stage is a link or reparse point".to_string());
    }
    if !metadata.file_type().is_dir() {
        return Err("runtime stage is not a directory".to_string());
    }

    let entries = fs::read_dir(destination)
        .map_err(|error| format!("failed to enumerate runtime stage: {error}"))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("failed to enumerate runtime stage entry: {error}"))?;
        let entry_metadata = fs::symlink_metadata(entry.path()).map_err(|error| {
            format!(
                "failed to inspect runtime stage entry {}: {error}",
                entry.path().display()
            )
        })?;
        if is_link_or_reparse_point(&entry_metadata) {
            return Err(format!(
                "runtime stage entry {} is a link or reparse point",
                entry.path().display()
            ));
        }
        if !entry_metadata.file_type().is_file() {
            return Err(format!(
                "runtime stage entry {} is not a regular file",
                entry.path().display()
            ));
        }

        let name = entry.file_name();
        let is_declared_artifact = ARTIFACTS
            .iter()
            .any(|artifact| name.as_os_str() == artifact.output_name)
            || name.as_os_str() == LICENSE_FILE_NAME;
        if !is_declared_artifact {
            return Err(format!(
                "runtime stage contains undeclared entry {}",
                entry.path().display()
            ));
        }
    }

    for artifact in ARTIFACTS {
        verify_file(
            &destination.join(artifact.output_name),
            artifact.output_name,
            artifact.size,
            artifact.sha256,
        )?;
    }

    let license_path = destination.join(LICENSE_FILE_NAME);
    let actual_license = fs::read_to_string(&license_path)
        .map_err(|error| format!("failed to read {}: {error}", license_path.display()))?;
    if actual_license != LICENSE_TEXT {
        return Err(format!(
            "{} does not match the expected embedded license text",
            license_path.display()
        ));
    }

    Ok(())
}

fn verify_file(
    path: &Path,
    name: &str,
    expected_size: u64,
    expected_sha256: &str,
) -> Result<(), String> {
    let metadata =
        fs::metadata(path).map_err(|error| format!("failed to inspect {name}: {error}"))?;
    if metadata.len() != expected_size {
        return Err(format!(
            "{name} has size {}, expected {expected_size}",
            metadata.len()
        ));
    }

    let mut file = File::open(path).map_err(|error| format!("failed to open {name}: {error}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("failed to hash {name}: {error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    let actual_sha256 = format!("{:x}", hasher.finalize());
    if actual_sha256 != expected_sha256 {
        return Err(format!(
            "{name} has SHA-256 {actual_sha256}, expected {expected_sha256}"
        ));
    }

    Ok(())
}
