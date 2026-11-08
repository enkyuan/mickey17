//! Opt-in asset body reuse. Every reuse is validated with the origin server.
use crate::{mirror::MAX_BODY, util};
use anyhow::{Context, Result, ensure};
use reqwest::{StatusCode, blocking::Client, header};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{io::Read, io::Write, path::PathBuf};
use url::Url;

pub fn fingerprint(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Deserialize, Serialize)]
struct Entry {
    url: String,
    digest: String,
    content_type: String,
    etag: Option<String>,
    last_modified: Option<String>,
}

pub struct AssetCache {
    root: PathBuf,
}

pub struct AssetResponse {
    pub bytes: Vec<u8>,
    pub content_type: String,
    pub reused: bool,
}

impl AssetCache {
    pub(crate) fn root(&self) -> &std::path::Path {
        &self.root
    }
    pub fn open(root: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&root)?;
        ensure!(
            !root.symlink_metadata()?.file_type().is_symlink(),
            "cache root is a symlink"
        );
        let root = root.canonicalize()?;
        for directory in ["objects", "urls"] {
            let path = util::checked_path(&root, &root.join(directory))?;
            std::fs::create_dir_all(path)?;
        }
        Ok(Self { root })
    }

    fn entry_path(&self, url: &Url) -> Result<PathBuf> {
        util::checked_path(
            &self.root,
            &self
                .root
                .join("urls")
                .join(fingerprint(url.as_str().as_bytes())),
        )
    }

    fn load(&self, url: &Url) -> Option<(Entry, Vec<u8>)> {
        let path = self.entry_path(url).ok()?;
        let entry: Entry = serde_json::from_slice(&read_bounded(&path, 16 * 1024).ok()?).ok()?;
        if entry.url != url.as_str()
            || entry.digest.len() != 64
            || !entry.digest.bytes().all(|c| c.is_ascii_hexdigit())
            || (entry.etag.is_none() && entry.last_modified.is_none())
            || entry
                .etag
                .iter()
                .chain(entry.last_modified.iter())
                .any(|value| header::HeaderValue::from_str(value).is_err())
        {
            return None;
        }
        let object =
            util::checked_path(&self.root, &self.root.join("objects").join(&entry.digest)).ok()?;
        let bytes = read_bounded(&object, MAX_BODY).ok()?;
        (fingerprint(&bytes) == entry.digest).then_some((entry, bytes))
    }

    fn save(&self, url: &Url, entry: &Entry, bytes: &[u8]) -> Result<()> {
        let object =
            util::checked_path(&self.root, &self.root.join("objects").join(&entry.digest))?;
        // Complete temporary files prevent readers from seeing partially written objects.
        if !object_matches(&object, bytes) {
            self.persist(&object, bytes)?;
        }
        self.persist(&self.entry_path(url)?, &serde_json::to_vec(entry)?)
    }

    fn invalidate(&self, url: &Url) -> Result<()> {
        let path = self.entry_path(url)?;
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    fn persist(&self, path: &std::path::Path, bytes: &[u8]) -> Result<()> {
        util::checked_path(&self.root, path)?;
        let mut file =
            tempfile::NamedTempFile::new_in(path.parent().context("cache parent missing")?)?;
        file.write_all(bytes)?;
        file.flush()?;
        file.persist(path)?;
        Ok(())
    }

    pub fn fetch(&self, client: &Client, url: &Url) -> Result<AssetResponse> {
        // A URL with embedded credentials must not enter this shared asset cache.
        let eligible_url = url.username().is_empty() && url.password().is_none();
        let cached = eligible_url.then(|| self.load(url)).flatten();
        let mut request = client.get(url.clone());
        if let Some((entry, _)) = &cached {
            if let Some(etag) = &entry.etag {
                request = request.header(header::IF_NONE_MATCH, etag);
            } else if let Some(modified) = &entry.last_modified {
                request = request.header(header::IF_MODIFIED_SINCE, modified);
            }
        }
        let response = request.send()?;
        let same_resource = response.url() == url;
        let forbidden = response.headers().contains_key(header::SET_COOKIE)
            || response.headers().contains_key(header::VARY)
            || response
                .headers()
                .get_all(header::CACHE_CONTROL)
                .iter()
                .any(|value| {
                    value.to_str().map_or(true, |s| {
                        s.split(',').any(|directive| {
                            matches!(
                                directive
                                    .trim()
                                    .split('=')
                                    .next()
                                    .unwrap_or("")
                                    .to_ascii_lowercase()
                                    .as_str(),
                                "no-store" | "private"
                            )
                        })
                    })
                });
        if forbidden || !same_resource {
            self.invalidate(url)?;
        }
        if response.status() == StatusCode::NOT_MODIFIED {
            if let Some((mut entry, bytes)) = cached
                && !forbidden
                && same_resource
            {
                let content_type = header_text(response.headers(), header::CONTENT_TYPE)
                    .unwrap_or_else(|| entry.content_type.clone());
                let etag =
                    header_text(response.headers(), header::ETAG).or_else(|| entry.etag.clone());
                let last_modified = header_text(response.headers(), header::LAST_MODIFIED)
                    .or_else(|| entry.last_modified.clone());
                if entry.content_type != content_type
                    || entry.etag != etag
                    || entry.last_modified != last_modified
                {
                    entry.content_type = content_type;
                    entry.etag = etag;
                    entry.last_modified = last_modified;
                    self.persist(&self.entry_path(url)?, &serde_json::to_vec(&entry)?)?;
                }
                return Ok(AssetResponse {
                    bytes,
                    content_type: entry.content_type,
                    reused: true,
                });
            }
            // Unexpected or non-cacheable 304: do a normal read rather than return stale bytes.
            let (bytes, content_type) = crate::mirror::fetch(client, url, false)?;
            return Ok(AssetResponse {
                bytes,
                content_type,
                reused: false,
            });
        }
        // A changed response must not retain the previous body while buffering the new one.
        drop(cached);
        let response = response.error_for_status()?;
        ensure!(
            response.status().is_success(),
            "unexpected HTTP status {}",
            response.status()
        );
        let content_type =
            header_text(response.headers(), header::CONTENT_TYPE).unwrap_or_default();
        let entry = Entry {
            url: url.to_string(),
            digest: String::new(),
            content_type: content_type.clone(),
            etag: header_text(response.headers(), header::ETAG),
            last_modified: header_text(response.headers(), header::LAST_MODIFIED),
        };
        ensure!(
            response.content_length().unwrap_or(0) <= MAX_BODY,
            "response exceeds 64 MiB"
        );
        let mut bytes = Vec::new();
        response.take(MAX_BODY + 1).read_to_end(&mut bytes)?;
        ensure!(bytes.len() as u64 <= MAX_BODY, "response exceeds 64 MiB");
        if eligible_url
            && same_resource
            && !forbidden
            && (entry.etag.is_some() || entry.last_modified.is_some())
        {
            let entry = Entry {
                digest: fingerprint(&bytes),
                ..entry
            };
            self.save(url, &entry, &bytes)?;
        }
        Ok(AssetResponse {
            bytes,
            content_type,
            reused: false,
        })
    }
}

fn object_matches(path: &std::path::Path, bytes: &[u8]) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return false;
    };
    if !metadata.file_type().is_file() || metadata.len() != bytes.len() as u64 {
        return false;
    }
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut buffer = [0u8; 16 * 1024];
    for chunk in bytes.chunks(buffer.len()) {
        if file.read_exact(&mut buffer[..chunk.len()]).is_err() || &buffer[..chunk.len()] != chunk {
            return false;
        }
    }
    file.read(&mut buffer[..1]).is_ok_and(|count| count == 0)
}

fn header_text(headers: &header::HeaderMap, name: header::HeaderName) -> Option<String> {
    headers.get(name)?.to_str().ok().map(str::to_owned)
}

fn read_bounded(path: &std::path::Path, limit: u64) -> Result<Vec<u8>> {
    ensure!(
        std::fs::symlink_metadata(path)?.file_type().is_file(),
        "cache entry is not a regular file"
    );
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "cache entry exceeds limit");
    Ok(bytes)
}
