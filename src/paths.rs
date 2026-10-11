//! Where bundled files (assets/, scripts/) live: the source tree the binary
//! was built from, or else the directory next to the executable. Independent
//! of the working directory and of `cargo run`'s environment.

use std::path::{Path, PathBuf};

pub fn resource_dir() -> PathBuf {
    let built = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if built.join("assets").is_dir() {
        return built;
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn assets_dir() -> PathBuf {
    resource_dir().join("assets")
}
