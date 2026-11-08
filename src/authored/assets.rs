//! Local reviewed artwork is input, with explicit ownership and immutable hashes.
use super::*;
use anyhow::{Result, ensure};
pub(super) type Files = BTreeMap<String, Vec<u8>>;
pub(super) fn load(
    spec_path: &Path,
    spec: &Specification,
    model: &project::Project,
) -> Result<Files> {
    ensure!(
        spec.source_assets.len() <= 256,
        "owned asset count exceeded"
    );
    let base = spec_path
        .parent()
        .unwrap_or(Path::new("."))
        .canonicalize()?;
    let pattern = regex::Regex::new(
        r"^assets/(?:fonts|images|vectors|videos)/[a-zA-Z0-9][a-zA-Z0-9/_.-]*\.(?:woff2?|ttf|otf|png|jpe?g|webp|avif|gif|svg|ico|webm|mp4)$",
    )?;
    let mut files = Files::new();
    let mut portable = std::collections::BTreeSet::new();
    let mut total = 0;
    for (path, asset) in &spec.source_assets {
        ensure!(
            pattern.is_match(path)
                && !path.contains("//")
                && path
                    .split('/')
                    .all(|p| p != ".." && p != "." && names::meaningful(p))
                && portable.insert(path.to_lowercase()),
            "invalid meaningful asset path: {path}"
        );
        ensure!(
            !asset.purpose.trim().is_empty() && !asset.evidence.is_empty(),
            "owned asset needs purpose and evidence"
        );
        for source in &asset.evidence {
            ensure!(
                model
                    .observations
                    .get(&source.observation)
                    .is_some_and(|o| source.node < o.capture.nodes.len()),
                "invalid owned asset evidence"
            );
        }
        ensure!(
            !Path::new(&asset.file).is_absolute(),
            "owned asset file must be relative"
        );
        let file = util::checked_path(&base, &base.join(&asset.file))?;
        let bytes = read(&file, 16 * 1024 * 1024)?;
        ensure!(
            fingerprint(&bytes) == asset.sha256,
            "owned asset hash mismatch: {path}"
        );
        total += bytes.len();
        ensure!(
            total <= 256 * 1024 * 1024,
            "owned asset byte budget exceeded"
        );
        files.insert(path.clone(), bytes);
    }
    Ok(files)
}
