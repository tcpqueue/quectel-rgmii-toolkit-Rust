//! Packs adb and the module installation files into one compressed blob embedded in the exe.
//!
//! Layout before compression: magic, file count, then per file the path length (u16 LE), UTF-8
//! path, size (u64 LE), SHA-256 and contents. The blob's own hash names the runtime directory.

use sha2::{Digest, Sha256};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

const MAGIC: &[u8] = b"SAPAYLOAD1\n";
/// Top-level files copied next to the extracted `development` directory.
const ROOT_FILES: [&str; 4] = ["adb.exe", "AdbWinApi.dll", "AdbWinUsbApi.dll", "LICENSE"];

fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, PathBuf)>) {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .map(|e| e.unwrap())
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = format!("{prefix}/{}", entry.file_name().to_string_lossy());
        let path = entry.path();
        if path.is_dir() {
            walk(&path, &name, out)
        } else {
            out.push((name, path))
        }
    }
}

fn main() {
    let repo = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
        .parent()
        .unwrap()
        .to_owned();
    let mut files: Vec<(String, PathBuf)> = ROOT_FILES
        .iter()
        .map(|name| (name.to_string(), repo.join(name)))
        .collect();
    walk(&repo.join("development"), "development", &mut files);
    println!("cargo:rerun-if-changed=build.rs");
    let mut raw = MAGIC.to_vec();
    raw.extend((files.len() as u32).to_le_bytes());
    for (name, path) in &files {
        println!("cargo:rerun-if-changed={}", path.display());
        let data = fs::read(path).unwrap_or_else(|e| {
            panic!(
                "payload file {} is missing ({e}); build the device binary first (scripts/build.sh)",
                path.display()
            )
        });
        raw.extend((name.len() as u16).to_le_bytes());
        raw.extend(name.as_bytes());
        raw.extend((data.len() as u64).to_le_bytes());
        raw.extend(Sha256::digest(&data));
        raw.extend(&data);
    }
    println!(
        "cargo:rerun-if-changed={}",
        repo.join("development").display()
    );
    let id = hex::encode(&Sha256::digest(&raw)[..8]);
    let packed = miniz_oxide::deflate::compress_to_vec(&raw, 9);
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("payload.bin");
    fs::write(&out, packed).unwrap();
    println!("cargo:rustc-env=SA_PAYLOAD_ID={id}");
}
