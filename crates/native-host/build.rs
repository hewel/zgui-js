use sha2::{Digest, Sha256};
use std::{env, fs, path::{Path, PathBuf}, process::Command};

fn sources(path: &Path, files: &mut Vec<PathBuf>) {
    println!("cargo:rerun-if-changed={}", path.display());
    for entry in fs::read_dir(path).expect("read build source directory") {
        let entry = entry.expect("read build source entry");
        let path = entry.path();
        if entry.file_type().expect("source file type").is_dir() {
            if !matches!(entry.file_name().to_str(), Some("target" | ".git" | "node_modules")) {
                sources(&path, files);
            }
        } else if matches!(path.extension().and_then(|value| value.to_str()), Some("rs" | "toml")) {
            files.push(path);
        }
    }
}

fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"))
        .join("../..").canonicalize().expect("workspace root");
    let mut files = Vec::new();
    sources(&root.join("crates"), &mut files);
    sources(&root.join("vendor"), &mut files);
    for name in ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "bun.lock"] {
        let path = root.join(name);
        println!("cargo:rerun-if-changed={}", path.display());
        files.push(path);
    }
    files.sort();
    let mut hash = Sha256::new();
    for file in files {
        let name = file.strip_prefix(&root).expect("source belongs to workspace").to_string_lossy();
        let bytes = fs::read(&file).expect("read build source");
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    println!("cargo:rustc-env=ZGUI_BUILD_SOURCE_SHA256={:x}", hash.finalize());
    let rustc = Command::new(env::var_os("RUSTC").expect("Rust compiler"))
        .arg("--version").output().expect("query Rust compiler");
    assert!(rustc.status.success(), "Rust compiler version query failed");
    println!("cargo:rustc-env=ZGUI_BUILD_RUSTC={}", String::from_utf8(rustc.stdout).expect("Rust version UTF-8").trim());
    println!("cargo:rustc-env=ZGUI_BUILD_TARGET={}", env::var("TARGET").expect("build target"));
    println!("cargo:rustc-env=ZGUI_BUILD_PROFILE={}", env::var("PROFILE").expect("build profile"));
}
