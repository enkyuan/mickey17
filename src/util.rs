use anyhow::{Context, Result, ensure};
use std::{
    io::Write,
    path::{Component, Path, PathBuf},
};
use walkdir::WalkDir;
pub fn site_dir(p: &Path) -> PathBuf {
    if !p.join("index.html").exists() && p.join("site").is_dir() {
        p.join("site")
    } else {
        p.into()
    }
}
pub fn text_files(p: &Path) -> Result<Vec<PathBuf>> {
    ensure!(p.is_dir(), "site directory does not exist: {}", p.display());
    WalkDir::new(p)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| match e {
            Ok(e)
                if e.file_type().is_file()
                    && matches!(
                        e.path()
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("")
                            .to_ascii_lowercase()
                            .as_str(),
                        "html"
                            | "htm"
                            | "js"
                            | "mjs"
                            | "css"
                            | "json"
                            | "xml"
                            | "txt"
                            | "svg"
                            | "webmanifest"
                            | "rsc"
                    ) =>
            {
                Some(Ok(e.into_path()))
            }
            Err(e) => Some(Err(e.into())),
            _ => None,
        })
        .collect()
}
/// Decode an incoming URL path once, then check its exact filesystem path.
pub fn contained(root: &Path, raw: &str) -> Result<PathBuf> {
    let decoded = percent_encoding::percent_decode_str(raw).decode_utf8()?;
    // Keep mirrored paths portable, including refusing Windows alternate data streams.
    ensure!(!decoded.contains(['\\', '\0', ':']), "invalid path");
    let rel = Path::new(decoded.trim_start_matches('/'));
    checked_path(root, &root.join(rel))
}

/// Validate an already-decoded path without interpreting percent characters.
pub fn checked_path(root: &Path, path: &Path) -> Result<PathBuf> {
    let rel = path.strip_prefix(root).context("path outside root")?;
    ensure!(
        rel.components().all(|c| matches!(c, Component::Normal(_))),
        "path traversal"
    );
    let mut out = root.to_path_buf();
    for component in rel.components() {
        out.push(component);
        if let Ok(meta) = out.symlink_metadata() {
            ensure!(!meta.file_type().is_symlink(), "symlink refused");
        }
    }
    Ok(out)
}
pub fn write(root: &Path, path: &Path, bytes: &[u8]) -> Result<()> {
    checked_path(root, path)?;
    std::fs::create_dir_all(path.parent().context("missing parent")?)?;
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut file) => file.write_all(bytes)?,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            ensure!(
                std::fs::read(path)? == bytes,
                "query/path collision: {}",
                path.display()
            );
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

/// Preserve the original on a failed write; rename only a complete sibling file.
pub fn atomic_replace(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut temporary = tempfile::NamedTempFile::new_in(path.parent().context("missing parent")?)?;
    temporary
        .as_file()
        .set_permissions(std::fs::metadata(path)?.permissions())?;
    temporary.write_all(bytes)?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    temporary.persist(path)?;
    Ok(())
}
