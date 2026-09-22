//! Keeps migration files byte-identical to what git stores.
//!
//! sqlx checksums each migration's exact bytes at compile time and compares
//! that checksum against the one recorded in the user's database. A file saved
//! with CRLF therefore produces a build that rejects every database written by
//! a build with the canonical LF bytes - and vice versa. The app then refuses
//! to start with "migration ... was previously applied but has been modified",
//! even though the SQL never changed.
//!
//! `.gitattributes` pins these files to LF, but that only governs checkout: an
//! editor or tool that rewrites one in place bypasses it, and git still reports
//! the file as unmodified because it normalizes on comparison. So normalize
//! here too, where it is the last step before sqlx reads the bytes.

use std::{fs, path::Path};

pub fn ensure_lf_line_endings() {
    let directory = Path::new("migrations");
    println!("cargo:rerun-if-changed=migrations");

    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            println!(
                "cargo:warning=Could not read {}: {error}",
                directory.display()
            );
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("sql") {
            continue;
        }
        println!("cargo:rerun-if-changed={}", path.display());

        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                println!("cargo:warning=Could not read {}: {error}", path.display());
                continue;
            }
        };
        if !bytes.contains(&b'\r') {
            continue;
        }

        match fs::write(&path, strip_carriage_returns(&bytes)) {
            Ok(()) => println!(
                "cargo:warning=Normalized CRLF line endings in {} - sqlx checksums these exact \
                 bytes, so CRLF here would reject databases written by any other build",
                path.display()
            ),
            Err(error) => println!(
                "cargo:warning=Could not normalize line endings in {}: {error}",
                path.display()
            ),
        }
    }
}

/// Drop only the carriage returns that form a CRLF pair, so a lone `\r` inside
/// a string literal would survive rather than being silently rewritten.
fn strip_carriage_returns(bytes: &[u8]) -> Vec<u8> {
    let mut normalized = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
            index += 1;
            continue;
        }
        normalized.push(bytes[index]);
        index += 1;
    }
    normalized
}
