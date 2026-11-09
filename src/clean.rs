//! `mochi clean`: inventory and remove regenerable build artifacts in a workspace.
//!
//! The workspace (normally `.mochi`) mixes irreplaceable evidence (captures,
//! provenance, source packs) with regenerable output. This module only ever
//! removes directories whose *exact* names are on a short allow-list and that
//! sit at a fixed relative position inside a project root, and it refuses
//! anything that touches evidence or a symlink. Default mode is a dry run.
//!
//! Every filesystem access after the workspace root is opened goes through a
//! held directory descriptor (`openat` with `O_NOFOLLOW`, `fstatat` without
//! following, `unlinkat`). A path component swapped for a symlink while a clean
//! is running therefore cannot redirect a read or a deletion: the descriptors
//! still name the directories that were verified.
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    io::Write,
    path::{Component, Path, PathBuf},
    time::SystemTime,
};

/// Device and inode of a directory.
pub type FileId = (u64, u64);

/// Directory names that always mark evidence (compared ASCII case-insensitively,
/// so a case-insensitive filesystem cannot alias them). `previous-projects`
/// holds frozen backups of earlier exports and is protected as a whole.
const EVIDENCE_NAMES: &[&str] = &[
    "provenance",
    "source-packs",
    "semantic-conversion",
    "capture",
    "reconstruction",
    "previous-projects",
    ".git",
];
/// Further workspace-level evidence directories. They are honoured when scanning
/// the workspace and build outputs, but not inside `node_modules` or cargo targets,
/// where third-party packages may legitimately ship directories with these names.
const WORKSPACE_EVIDENCE_NAMES: &[&str] = &["handoff", "references", "qualification"];
const PROTECTED: &str = "protected evidence, not scanned: ";
/// Relative locations removed when a project root is selected, build class.
const BUILD_PATHS: &[&[&str]] = &[
    &[".output"],
    &[".tanstack"],
    &[".mochi", "dist"],
    &["node_modules", ".vite"],
    &["node_modules", ".nitro"],
    &["node_modules", ".vite-temp"],
];
/// Deepest directory nesting inspected or removed inside one candidate.
const MAX_DEPTH: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Class {
    Build,
    Deps,
    Cargo,
}
impl Class {
    pub fn label(self) -> &'static str {
        match self {
            Class::Build => "build",
            Class::Deps => "deps",
            Class::Cargo => "cargo",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Will be deleted by `--apply`.
    Remove,
    /// Listed but not selected (`--deps` / `--cargo` not given).
    OptIn,
    /// Protected by `--keep`.
    Kept,
    /// Protected as the newest build of its site family (`--keep-latest`).
    KeptLatest,
    /// Refused for the stated safety reason; never deleted.
    Refused(String),
}
impl Status {
    fn label(&self) -> String {
        match self {
            Status::Remove => "remove".into(),
            Status::OptIn => "skipped (opt-in flag not given)".into(),
            Status::Kept => "kept (--keep)".into(),
            Status::KeptLatest => "kept (--keep-latest)".into(),
            Status::Refused(why) => format!("REFUSED: {why}"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub path: PathBuf,
    pub class: Class,
    pub bytes: u64,
    pub status: Status,
    /// Project root this belongs to (`None` only for cargo target directories).
    pub project: Option<PathBuf>,
    /// Identity of the directory when it was planned; deletion requires it to match.
    pub id: Option<FileId>,
}

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub apply: bool,
    pub deps: bool,
    pub cargo: bool,
    pub keep: Vec<PathBuf>,
    pub keep_latest: bool,
}

#[derive(Debug)]
pub struct Plan {
    pub root: PathBuf,
    pub root_id: FileId,
    pub candidates: Vec<Candidate>,
    /// Unreadable entries, skipped symlinks and protected evidence directories.
    pub notes: Vec<String>,
}
impl Plan {
    pub fn removable_bytes(&self) -> u64 {
        self.candidates
            .iter()
            .filter(|c| c.status == Status::Remove)
            .map(|c| c.bytes)
            .sum()
    }
    pub fn refused(&self) -> usize {
        self.candidates
            .iter()
            .filter(|c| matches!(c.status, Status::Refused(_)))
            .count()
    }
}

#[derive(Debug, Default)]
pub struct Report {
    pub removed: Vec<PathBuf>,
    pub failed: Vec<(PathBuf, String)>,
    pub bytes: u64,
}

/// Directory handles. Everything below the workspace root is reached through these.
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod sys {
    use super::FileId;
    use rustix::fs::{
        AtFlags, CWD, Dir as RawDir, FileType, FlockOperation, Mode, OFlags, flock, fstat, openat,
        statat, unlinkat,
    };
    use std::{
        ffi::{OsStr, OsString},
        io::{self, Read},
        os::{fd::OwnedFd, unix::ffi::OsStrExt},
        path::Path,
        time::{Duration, SystemTime},
    };

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Kind {
        Dir,
        File,
        Symlink,
        Other,
    }
    #[derive(Clone, Copy, Debug)]
    pub struct Meta {
        pub id: FileId,
        pub kind: Kind,
        pub size: u64,
        pub modified: Option<SystemTime>,
    }
    pub struct Dir(OwnedFd);

    fn meta_of(st: &rustix::fs::Stat) -> Meta {
        let kind = match FileType::from_raw_mode(st.st_mode) {
            FileType::Directory => Kind::Dir,
            FileType::RegularFile => Kind::File,
            FileType::Symlink => Kind::Symlink,
            _ => Kind::Other,
        };
        #[allow(clippy::unnecessary_cast, clippy::cast_sign_loss)]
        let (dev, size) = (st.st_dev as u64, st.st_size as u64);
        #[allow(clippy::unnecessary_cast, clippy::cast_sign_loss)]
        let modified = u64::try_from(st.st_mtime)
            .ok()
            .map(|s| SystemTime::UNIX_EPOCH + Duration::new(s, st.st_mtime_nsec as u32));
        Meta {
            id: (dev, st.st_ino),
            kind,
            size,
            modified,
        }
    }
    const DIR_FLAGS: OFlags = OFlags::RDONLY
        .union(OFlags::DIRECTORY)
        .union(OFlags::NOFOLLOW)
        .union(OFlags::CLOEXEC);

    impl Dir {
        /// Open `path` as a directory; the final component must not be a symlink.
        pub fn open(path: &Path) -> io::Result<Dir> {
            Ok(Dir(openat(CWD, path, DIR_FLAGS, Mode::empty())?))
        }
        /// Open a child directory without following a symlink.
        pub fn child(&self, name: &OsStr) -> io::Result<Dir> {
            Ok(Dir(openat(&self.0, name, DIR_FLAGS, Mode::empty())?))
        }
        pub fn meta(&self) -> io::Result<Meta> {
            Ok(meta_of(&fstat(&self.0)?))
        }
        /// Metadata of an entry, never following a symlink.
        pub fn child_meta(&self, name: &OsStr) -> io::Result<Meta> {
            Ok(meta_of(&statat(&self.0, name, AtFlags::SYMLINK_NOFOLLOW)?))
        }
        pub fn names(&self) -> io::Result<Vec<OsString>> {
            let mut dir = RawDir::read_from(&self.0)?;
            let mut names = Vec::new();
            while let Some(entry) = dir.read() {
                let bytes = entry?.file_name().to_bytes().to_vec();
                if bytes != b"." && bytes != b".." {
                    names.push(OsStr::from_bytes(&bytes).to_os_string());
                }
            }
            Ok(names)
        }
        /// Read a small regular file (not through a symlink); `None` if absent or not one.
        pub fn read_file(&self, name: &OsStr, max: u64) -> Option<String> {
            let fd = openat(
                &self.0,
                name,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                Mode::empty(),
            )
            .ok()?;
            let file = std::fs::File::from(fd);
            if file.metadata().ok()?.len() > max || !file.metadata().ok()?.is_file() {
                return None;
            }
            let mut text = String::new();
            file.take(max).read_to_string(&mut text).ok()?;
            Some(text)
        }
        pub fn unlink(&self, name: &OsStr) -> io::Result<()> {
            Ok(unlinkat(&self.0, name, AtFlags::empty())?)
        }
        pub fn rmdir(&self, name: &OsStr) -> io::Result<()> {
            Ok(unlinkat(&self.0, name, AtFlags::REMOVEDIR)?)
        }
        /// Whether another process holds an exclusive `flock` on the named file.
        pub fn locked_elsewhere(&self, name: &OsStr) -> io::Result<bool> {
            let fd = match openat(
                &self.0,
                name,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                Mode::empty(),
            ) {
                Ok(fd) => fd,
                Err(rustix::io::Errno::NOENT) => return Ok(false),
                Err(e) => return Err(e.into()),
            };
            match flock(&fd, FlockOperation::NonBlockingLockExclusive) {
                Ok(()) => Ok(false),
                Err(rustix::io::Errno::WOULDBLOCK) => Ok(true),
                Err(e) => Err(e.into()),
            }
        }
    }
}
/// Directory handles are not implemented off macOS and Linux: planning fails.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod sys {
    use super::FileId;
    use std::{
        ffi::{OsStr, OsString},
        io,
        path::Path,
        time::SystemTime,
    };
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Kind {
        Dir,
        File,
        Symlink,
        Other,
    }
    #[derive(Clone, Copy, Debug)]
    pub struct Meta {
        pub id: FileId,
        pub kind: Kind,
        pub size: u64,
        pub modified: Option<SystemTime>,
    }
    pub struct Dir;
    fn unsupported<T>() -> io::Result<T> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "mochi clean is supported on macOS and Linux only",
        ))
    }
    impl Dir {
        pub fn open(_: &Path) -> io::Result<Dir> {
            unsupported()
        }
        pub fn child(&self, _: &OsStr) -> io::Result<Dir> {
            unsupported()
        }
        pub fn meta(&self) -> io::Result<Meta> {
            unsupported()
        }
        pub fn child_meta(&self, _: &OsStr) -> io::Result<Meta> {
            unsupported()
        }
        pub fn names(&self) -> io::Result<Vec<OsString>> {
            unsupported()
        }
        pub fn read_file(&self, _: &OsStr, _: u64) -> Option<String> {
            None
        }
        pub fn unlink(&self, _: &OsStr) -> io::Result<()> {
            unsupported()
        }
        pub fn rmdir(&self, _: &OsStr) -> io::Result<()> {
            unsupported()
        }
        pub fn locked_elsewhere(&self, _: &OsStr) -> io::Result<bool> {
            unsupported()
        }
    }
}
use sys::{Dir, Kind};

fn lower(name: &str) -> String {
    name.to_ascii_lowercase()
}
/// Names that are evidence anywhere: the core list and any `capture*` prefix.
fn is_core_evidence_name(name: &str) -> bool {
    let l = lower(name);
    EVIDENCE_NAMES.contains(&l.as_str()) || l.starts_with("capture")
}
/// Core names plus the workspace-level extras (`handoff`, `verification*`, ...).
fn is_evidence_name(name: &str) -> bool {
    let l = lower(name);
    is_core_evidence_name(name)
        || WORKSPACE_EVIDENCE_NAMES.contains(&l.as_str())
        || l.starts_with("verification")
}
fn has(dir: &Dir, name: &str) -> Option<Kind> {
    dir.child_meta(OsStr::new(name)).ok().map(|m| m.kind)
}
/// A capture directory: a `manifest.json` beside a `site/`. This deliberately
/// over-matches: either may be a symlink, and `manifest.json` may be of any type.
fn is_capture(dir: &Dir) -> bool {
    has(dir, "manifest.json").is_some()
        && matches!(has(dir, "site"), Some(Kind::Dir | Kind::Symlink))
}
fn is_project_root(dir: &Dir) -> bool {
    has(dir, "package.json") == Some(Kind::File) && has(dir, "bun.lock") == Some(Kind::File)
}
/// The first line of every cache directory tag (<https://bford.info/cachedir/>).
const CACHEDIR_SIGNATURE: &str = "Signature: 8a477f597d28d172789f06886806bc55";
/// A cargo target directory: named `target`, not a project root, and holding a
/// marker cargo itself writes with the content cargo writes (an empty or foreign
/// `CACHEDIR.TAG` / `.rustc_info.json` does not count).
fn is_cargo_target(dir: &Dir, name: &OsStr) -> bool {
    let tagged = || {
        dir.read_file(OsStr::new("CACHEDIR.TAG"), 4096)
            .is_some_and(|t| t.starts_with(CACHEDIR_SIGNATURE) && t.contains("cargo"))
    };
    let rustc_info = || {
        dir.read_file(OsStr::new(".rustc_info.json"), 1 << 20)
            .is_some_and(|t| {
                serde_json::from_str::<serde_json::Value>(&t)
                    .is_ok_and(|v| v.get("rustc_fingerprint").is_some())
            })
    };
    name == "target" && !is_project_root(dir) && (tagged() || rustc_info())
}
/// Whether a build is using this cargo target: cargo holds an `flock` on
/// `.cargo-lock` in the target and in each profile directory while it runs.
fn cargo_in_use(target: &Dir) -> bool {
    let lock = OsStr::new(".cargo-lock");
    let locked = |dir: &Dir| dir.locked_elsewhere(lock).unwrap_or(true);
    if locked(target) {
        return true;
    }
    let Ok(names) = target.names() else {
        return true;
    };
    for name in names {
        if target.child_meta(&name).is_ok_and(|m| m.kind == Kind::Dir) {
            match target.child(&name) {
                Ok(child) => {
                    if locked(&child) {
                        return true;
                    }
                    // `target/<triple>/<profile>/.cargo-lock`
                    if let Ok(inner) = child.names() {
                        for n in inner {
                            if child.child_meta(&n).is_ok_and(|m| m.kind == Kind::Dir)
                                && child.child(&n).map(|d| locked(&d)).unwrap_or(true)
                            {
                                return true;
                            }
                        }
                    }
                }
                Err(_) => return true,
            }
        }
    }
    false
}

struct Tree {
    bytes: u64,
    /// First evidence directory found inside, if any.
    evidence: Option<PathBuf>,
    /// Entries that could not be read or verified; their contents were not checked.
    unreadable: Vec<String>,
}
/// How strictly a directory tree is searched for evidence before removal.
#[derive(Clone, Copy)]
enum Scope {
    /// Build outputs: any evidence name, capture prefix, capture layout or `.git`.
    Build,
    /// `node_modules`: third-party packages legitimately contain `.git`, `capture*`
    /// and `references` directories, so only the capture layout and the core
    /// unambiguous evidence names count.
    Vendored,
    /// Cargo `target`: crates named `capture*` produce `capture*` build
    /// directories, but cargo never writes `.git`, so `.git` counts here.
    Cargo,
}
fn tree_evidence(dir: &Dir, name: &str, scope: Scope) -> bool {
    let l = lower(name);
    let unambiguous =
        l != ".git" && !l.starts_with("capture") && EVIDENCE_NAMES.contains(&l.as_str());
    match scope {
        Scope::Build => is_evidence_name(name) || is_capture(dir),
        Scope::Vendored => unambiguous || is_capture(dir),
        Scope::Cargo => unambiguous || l == ".git" || is_capture(dir),
    }
}
fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}
/// Size a directory without following symlinks, look for evidence inside it, and
/// refuse to cross onto another filesystem.
fn survey(dir: &Dir, display: &Path, scope: Scope) -> Tree {
    let mut tree = Tree {
        bytes: 0,
        evidence: None,
        unreadable: Vec::new(),
    };
    let Ok(own) = dir.meta() else {
        tree.unreadable
            .push(format!("{}: cannot stat", display.display()));
        return tree;
    };
    // The directory itself may already be evidence (a capture layout).
    if tree_evidence(dir, &name_of(display), scope) {
        tree.evidence = Some(display.to_path_buf());
    }
    survey_into(dir, display, scope, own.id.0, 0, &mut tree);
    tree
}
fn survey_into(dir: &Dir, path: &Path, scope: Scope, dev: u64, depth: usize, tree: &mut Tree) {
    if depth >= MAX_DEPTH {
        tree.unreadable.push(format!(
            "{}: nested deeper than {MAX_DEPTH}",
            path.display()
        ));
        return;
    }
    let names = match dir.names() {
        Ok(n) => n,
        Err(e) => {
            tree.unreadable.push(format!("{}: {e}", path.display()));
            return;
        }
    };
    for name in names {
        let child_path = path.join(&name);
        let meta = match dir.child_meta(&name) {
            Ok(m) => m,
            Err(e) => {
                tree.unreadable
                    .push(format!("{}: {e}", child_path.display()));
                continue;
            }
        };
        tree.bytes += meta.size;
        if meta.kind != Kind::Dir {
            continue;
        }
        if meta.id.0 != dev {
            tree.unreadable.push(format!(
                "{}: on another filesystem (mount point)",
                child_path.display()
            ));
            continue;
        }
        let child = match dir.child(&name) {
            Ok(c) if c.meta().is_ok_and(|m| m.id == meta.id) => c,
            Ok(_) => {
                tree.unreadable.push(format!(
                    "{}: changed while being read",
                    child_path.display()
                ));
                continue;
            }
            Err(e) => {
                tree.unreadable
                    .push(format!("{}: {e}", child_path.display()));
                continue;
            }
        };
        if tree.evidence.is_none() && tree_evidence(&child, &name.to_string_lossy(), scope) {
            tree.evidence = Some(child_path.clone());
        }
        survey_into(&child, &child_path, scope, dev, depth + 1, tree);
    }
}

/// Delete the contents of `dir` (not `dir` itself), through descriptors only.
/// Refuses at the first evidence directory, symlink swap or mount point.
fn remove_contents(dir: &Dir, path: &Path, scope: Scope, dev: u64, depth: usize) -> Result<()> {
    ensure!(
        depth < MAX_DEPTH,
        "{}: nested deeper than {MAX_DEPTH}",
        path.display()
    );
    // The directory being emptied may itself have become a capture layout
    // (a `manifest.json` beside a `site/`) since it was surveyed: that layout
    // spans two sibling entries, so the per-child evidence check below never
    // sees it. Re-check it here, at every level, so a capture moved onto a
    // candidate root (or any directory on the delete path) between survey and
    // deletion is refused instead of emptied.
    ensure!(
        !is_capture(dir),
        "evidence (capture layout) appeared at {}",
        path.display()
    );
    for name in dir
        .names()
        .with_context(|| format!("reading {}", path.display()))?
    {
        let child_path = path.join(&name);
        let meta = dir
            .child_meta(&name)
            .with_context(|| format!("stat {}", child_path.display()))?;
        if meta.kind == Kind::Dir {
            ensure!(
                meta.id.0 == dev,
                "{} is on another filesystem",
                child_path.display()
            );
            let child = dir
                .child(&name)
                .with_context(|| format!("opening {}", child_path.display()))?;
            ensure!(
                child.meta()?.id == meta.id,
                "{} changed while being removed",
                child_path.display()
            );
            ensure!(
                !tree_evidence(&child, &name.to_string_lossy(), scope),
                "evidence appeared at {}",
                child_path.display()
            );
            remove_contents(&child, &child_path, scope, dev, depth + 1)?;
            dir.rmdir(&name)
                .with_context(|| format!("removing {}", child_path.display()))?;
        } else {
            dir.unlink(&name)
                .with_context(|| format!("removing {}", child_path.display()))?;
        }
    }
    Ok(())
}

struct Project {
    root: PathBuf,
    family: String,
    /// Newest modification time among existing build outputs (flag independent).
    built: Option<SystemTime>,
    candidates: Vec<Candidate>,
}

/// Site family of a project, used only by `--keep-latest`.
///
/// The `input_sha256` in `<project>/.mochi/export.json`: every export of the same
/// saved capture model shares it, whatever the directory is called. A project
/// without one is its own family and is never superseded: directory names are not
/// trusted to say that two projects are the same site.
pub fn family_of(project: &Path) -> String {
    let hash = Dir::open(project)
        .ok()
        .and_then(|d| d.child(OsStr::new(".mochi")).ok())
        .and_then(|d| d.read_file(OsStr::new("export.json"), 64 << 20))
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|v| v.get("input_sha256")?.as_str().map(str::to_owned))
        .filter(|h| !h.is_empty());
    match hash {
        Some(h) => format!("capture:{h}"),
        None => format!("path:{}", project.display()),
    }
}

/// Follow `rel` below `start`, requiring every existing component to be a real
/// directory stored under exactly that spelling. `Ok(None)`: not present (a
/// case-variant spelling counts as not present). `Err`: a symlink or odd entry.
fn resolve(
    start: &Dir,
    start_path: &Path,
    rel: &[&str],
) -> std::result::Result<Option<(Dir, PathBuf)>, String> {
    let mut path = start_path.to_path_buf();
    let mut current: Option<Dir> = None;
    for part in rel {
        let dir = current.as_ref().unwrap_or(start);
        path.push(part);
        let name = OsStr::new(part);
        let names = dir
            .names()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if !names.iter().any(|n| n == name) {
            return Ok(None);
        }
        let meta = match dir.child_meta(name) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        match meta.kind {
            Kind::Symlink => return Err(format!("{} is a symlink", path.display())),
            Kind::Dir => {}
            _ => return Ok(None),
        }
        let child = dir
            .child(name)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if child.meta().map_err(|e| e.to_string())?.id != meta.id {
            return Err(format!("{} changed while being opened", path.display()));
        }
        current = Some(child);
    }
    Ok(current.map(|d| (d, path)))
}

fn inspect_candidate(
    project: &Dir,
    project_path: &Path,
    rel: &[&str],
    class: Class,
    scope: Scope,
    notes: &mut Vec<String>,
) -> Option<(Candidate, Option<SystemTime>)> {
    let refused = |why: String| Candidate {
        path: rel
            .iter()
            .fold(project_path.to_path_buf(), |p, c| p.join(c)),
        class,
        bytes: 0,
        status: Status::Refused(why),
        project: Some(project_path.to_path_buf()),
        id: None,
    };
    let (dir, path) = match resolve(project, project_path, rel) {
        Ok(Some(found)) => found,
        Ok(None) => return None,
        Err(why) => return Some((refused(why), None)),
    };
    let meta = match dir.meta() {
        Ok(m) => m,
        Err(e) => return Some((refused(format!("{}: {e}", path.display())), None)),
    };
    let tree = survey(&dir, &path, scope);
    let same_device = project.meta().is_ok_and(|p| p.id.0 == meta.id.0);
    let status = if !same_device {
        Status::Refused("on another filesystem than its project (mount point)".into())
    } else if let Some(evidence) = tree.evidence {
        Status::Refused(format!(
            "contains evidence directory {}",
            evidence.display()
        ))
    } else if let Some(first) = tree.unreadable.first() {
        notes.extend(tree.unreadable.iter().map(|u| format!("unreadable: {u}")));
        Status::Refused(format!("unreadable entry, contents not verified ({first})"))
    } else {
        Status::Remove
    };
    Some((
        Candidate {
            path,
            class,
            bytes: tree.bytes,
            status,
            project: Some(project_path.to_path_buf()),
            id: Some(meta.id),
        },
        meta.modified,
    ))
}

fn inspect_project(dir: &Dir, root: &Path, options: &Options, notes: &mut Vec<String>) -> Project {
    let mut candidates = Vec::new();
    let mut built: Option<SystemTime> = None;
    for rel in BUILD_PATHS {
        if options.deps && rel[0] == "node_modules" {
            // With --deps the whole node_modules is listed instead of its cache
            // subdirs, but they still count toward "when was this last built", so
            // `--keep-latest` chooses the same project with and without --deps.
            if let Ok(Some((d, _))) = resolve(dir, root, rel) {
                built = built.max(d.meta().ok().and_then(|m| m.modified));
            }
            continue;
        }
        let scope = if rel[0] == "node_modules" {
            Scope::Vendored
        } else {
            Scope::Build
        };
        if let Some((c, mtime)) = inspect_candidate(dir, root, rel, Class::Build, scope, notes) {
            built = built.max(mtime);
            candidates.push(c);
        }
    }
    if let Some((mut c, _)) = inspect_candidate(
        dir,
        root,
        &["node_modules"],
        Class::Deps,
        Scope::Vendored,
        notes,
    ) {
        if !options.deps && c.status == Status::Remove {
            c.status = Status::OptIn;
        }
        candidates.push(c);
    }
    Project {
        root: root.to_path_buf(),
        family: family_of(root),
        built,
        candidates,
    }
}

struct Found {
    projects: Vec<Project>,
    cargo: Vec<Candidate>,
    notes: Vec<String>,
}
fn scan(dir: &Dir, path: &Path, options: &Options, found: &mut Found, depth: usize) {
    if depth >= MAX_DEPTH {
        found
            .notes
            .push(format!("unreadable: {}: nested too deeply", path.display()));
        return;
    }
    let names = match dir.names() {
        Ok(n) => n,
        Err(e) => {
            found
                .notes
                .push(format!("unreadable: {}: {e}", path.display()));
            return;
        }
    };
    let mut dirs: Vec<OsString> = Vec::new();
    for name in names {
        match dir.child_meta(&name) {
            Ok(m) if m.kind == Kind::Symlink => found
                .notes
                .push(format!("skipped symlink: {}", path.join(&name).display())),
            Ok(m) if m.kind == Kind::Dir => dirs.push(name),
            Ok(_) => {}
            Err(e) => found
                .notes
                .push(format!("unreadable: {}: {e}", path.join(&name).display())),
        }
    }
    dirs.sort();
    for name in dirs {
        let child_path = path.join(&name);
        let text = name.to_string_lossy().into_owned();
        let child = match dir.child(&name) {
            Ok(c) => c,
            Err(e) => {
                found
                    .notes
                    .push(format!("unreadable: {}: {e}", child_path.display()));
                continue;
            }
        };
        if is_evidence_name(&text) || is_capture(&child) {
            found
                .notes
                .push(format!("{PROTECTED}{}", child_path.display()));
            continue;
        }
        if text == "node_modules" || text.starts_with(".mochi-export-") {
            continue;
        }
        if is_cargo_target(&child, &name) {
            let tree = survey(&child, &child_path, Scope::Cargo);
            let status = if let Some(e) = tree.evidence {
                Status::Refused(format!("contains evidence directory {}", e.display()))
            } else if let Some(first) = tree.unreadable.first() {
                found
                    .notes
                    .extend(tree.unreadable.iter().map(|u| format!("unreadable: {u}")));
                Status::Refused(format!("unreadable entry, contents not verified ({first})"))
            } else if options.cargo && cargo_in_use(&child) {
                Status::Refused("a cargo build is using this target (.cargo-lock is held)".into())
            } else if options.cargo {
                Status::Remove
            } else {
                Status::OptIn
            };
            found.cargo.push(Candidate {
                id: child.meta().ok().map(|m| m.id),
                path: child_path,
                class: Class::Cargo,
                bytes: tree.bytes,
                status,
                project: None,
            });
            continue;
        }
        // Build outputs are removed as a unit, never scanned for projects.
        let in_dot_mochi = path.file_name().is_some_and(|n| n == ".mochi");
        if text == ".output" || text == ".tanstack" || (in_dot_mochi && text == "dist") {
            continue;
        }
        if is_project_root(&child) {
            let project = inspect_project(&child, &child_path, options, &mut found.notes);
            found.projects.push(project);
        }
        scan(&child, &child_path, options, found, depth + 1);
    }
}

fn path_id(path: &Path) -> Option<FileId> {
    Dir::open(path).ok()?.meta().ok().map(|m| m.id)
}
/// A kept path: the canonical directory and its identity.
type Keep = (PathBuf, Option<FileId>);
/// `path` is the kept directory or lies inside it (by name or by identity, so a
/// case-insensitive filesystem cannot alias it away).
fn is_kept(keep: &Keep, path: &Path) -> bool {
    path.starts_with(&keep.0)
        || keep
            .1
            .is_some_and(|id| path.ancestors().any(|a| path_id(a) == Some(id)))
}
/// Project `root` is protected by `keep`: the kept path is the project, contains
/// it, or lies anywhere inside it (keeping `site/.output` keeps all of `site`).
fn keeps_project(keep: &Keep, root: &Path) -> bool {
    is_kept(keep, root) || keep.0.starts_with(root)
}

/// Make `path` absolute and drop `.` and trailing separators, without touching the
/// filesystem, except that `..` is refused when it would leave a symlink (it
/// climbs from the link's target, not from the link).
fn lexical_absolute(path: &Path) -> Result<PathBuf> {
    let base = if path.is_absolute() {
        PathBuf::new()
    } else {
        std::env::current_dir()?
    };
    let mut out = base;
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                ensure!(
                    !std::fs::symlink_metadata(&out).is_ok_and(|m| m.file_type().is_symlink()),
                    "refusing a path that leaves a symlink with `..`: {}",
                    path.display()
                );
                out.pop();
            }
            other => out.push(other),
        }
    }
    Ok(out)
}

/// Resolve a `--keep` argument. It must exist, must lie in (or contain) the
/// workspace, and must not reach its target through a symlink: protecting what a
/// link points at, rather than the project the path names, is a silent surprise.
fn resolve_keep(path: &Path, root: &Path) -> Result<Keep> {
    let mut found = Vec::new();
    for base in [None, Some(root)] {
        let candidate = match base {
            None => path.to_path_buf(),
            Some(b) if path.is_relative() => b.join(path),
            Some(_) => continue,
        };
        let lexical = lexical_absolute(&candidate)?;
        if let Ok(canon) = lexical.canonicalize() {
            found.push((lexical, canon));
        }
    }
    ensure!(
        !found.is_empty(),
        "--keep path does not exist: {}",
        path.display()
    );
    let (lexical, canon) = found
        .iter()
        .find(|(_, k)| k.starts_with(root) || root.starts_with(k))
        .cloned()
        .with_context(|| {
            format!(
                "--keep path is outside the workspace {} and protects nothing: {}",
                root.display(),
                found[0].1.display()
            )
        })?;
    // A symlink on the way is allowed only when it resolves to a directory that
    // still contains the workspace (for example /tmp -> /private/tmp above it).
    let mut prefix = PathBuf::new();
    for component in lexical.components() {
        prefix.push(component);
        let is_link = std::fs::symlink_metadata(&prefix).is_ok_and(|m| m.file_type().is_symlink());
        if is_link && !prefix.canonicalize().is_ok_and(|t| root.starts_with(t)) {
            bail!(
                "--keep path goes through the symlink {}; pass the real directory instead",
                prefix.display()
            );
        }
    }
    let id = path_id(&canon);
    ensure!(
        id.is_some(),
        "--keep must name a directory: {}",
        path.display()
    );
    Ok((canon, id))
}

/// Check that `workspace` is a plain, non-evidence directory and return it canonical.
/// Only the final component must not be a symlink: a parent link is resolved and the
/// canonical directory is what is cleaned (and printed).
pub fn canonical_workspace(workspace: &Path) -> Result<PathBuf> {
    let lexical = lexical_absolute(workspace)?;
    let meta = std::fs::symlink_metadata(&lexical)
        .with_context(|| format!("workspace not found: {}", lexical.display()))?;
    ensure!(
        !meta.file_type().is_symlink(),
        "refusing symlinked workspace: {}",
        lexical.display()
    );
    ensure!(
        meta.is_dir(),
        "workspace is not a directory: {}",
        lexical.display()
    );
    let root = lexical.canonicalize()?;
    ensure!(
        root.parent().is_some(),
        "refusing to clean the filesystem root"
    );
    let home = std::env::home_dir()
        .context("cannot determine the home directory, so cannot rule out cleaning it")?;
    let home = home.canonicalize().unwrap_or(home);
    ensure!(
        !home.starts_with(&root),
        "refusing to clean the home directory or a directory containing it: {}",
        root.display()
    );
    ensure!(
        root.file_name()
            .is_none_or(|n| !is_evidence_name(&n.to_string_lossy())),
        "refusing a workspace that is an evidence directory: {}",
        root.display()
    );
    for component in root.components() {
        if let Component::Normal(name) = component {
            ensure!(
                !is_core_evidence_name(&name.to_string_lossy()),
                "refusing a workspace inside an evidence directory: {}",
                root.display()
            );
        }
    }
    // A capture is recognised by its layout whatever it is called, so check the
    // workspace and every ancestor (a workspace at `capture/site` is evidence).
    if let Some(capture) = root
        .ancestors()
        .find(|a| Dir::open(a).is_ok_and(|d| is_capture(&d)))
    {
        bail!(
            "refusing a workspace that is, or lies inside, a capture directory: {}",
            capture.display()
        );
    }
    Ok(root)
}

pub fn plan(workspace: &Path, options: &Options) -> Result<Plan> {
    let root = canonical_workspace(workspace)?;
    let root_dir = Dir::open(&root).with_context(|| format!("opening {}", root.display()))?;
    let root_id = root_dir.meta()?.id;
    let mut keep: Vec<Keep> = Vec::new();
    for k in &options.keep {
        keep.push(resolve_keep(k, &root)?);
    }
    let mut found = Found {
        projects: Vec::new(),
        cargo: Vec::new(),
        notes: Vec::new(),
    };
    if is_project_root(&root_dir) {
        let project = inspect_project(&root_dir, &root, options, &mut found.notes);
        found.projects.push(project);
    }
    scan(&root_dir, &root, options, &mut found, 0);
    let Found {
        projects,
        cargo,
        mut notes,
    } = found;
    let repos = notes
        .iter()
        .filter(|n| n.starts_with(PROTECTED) && n.ends_with("/.git"))
        .count();
    if repos > 0 {
        notes.push(format!(
            "WARNING: {repos} git repositor{} inside the workspace; this looks like a source tree, not a .mochi workspace. Check the table before using --apply",
            if repos == 1 { "y" } else { "ies" }
        ));
    }
    // --keep-latest: the newest built project of each family stays whole.
    let mut latest: BTreeMap<String, (SystemTime, PathBuf)> = BTreeMap::new();
    if options.keep_latest {
        for p in &projects {
            if let Some(built) = p.built {
                let slot = latest
                    .entry(p.family.clone())
                    .or_insert((built, p.root.clone()));
                if (built, &p.root) > (slot.0, &slot.1) {
                    *slot = (built, p.root.clone());
                }
            }
        }
    }
    let mut candidates = Vec::new();
    for project in projects {
        let kept = keep.iter().any(|k| keeps_project(k, &project.root));
        let latest_here = latest
            .get(&project.family)
            .is_some_and(|(_, r)| *r == project.root);
        for mut c in project.candidates {
            if matches!(c.status, Status::Remove | Status::OptIn) {
                if kept {
                    c.status = Status::Kept;
                } else if latest_here {
                    c.status = Status::KeptLatest;
                }
            }
            if c.class == Class::Deps && c.status == Status::Remove && !options.deps {
                c.status = Status::OptIn;
            }
            candidates.push(c);
        }
    }
    for mut c in cargo {
        if keep
            .iter()
            .any(|k| is_kept(k, &c.path) || k.0.starts_with(&c.path))
        {
            c.status = Status::Kept;
        }
        candidates.push(c);
    }
    candidates.sort_by(|a, b| a.path.cmp(&b.path));
    for c in &candidates {
        ensure!(
            c.path.starts_with(&root),
            "internal error: candidate outside workspace: {}",
            c.path.display()
        );
    }
    Ok(Plan {
        root,
        root_id,
        candidates,
        notes,
    })
}

fn human(bytes: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < units.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", units[unit])
    }
}
/// Make text safe to print: control characters (including newlines and escape
/// sequences in directory names) are shown as `\u{..}`.
fn show(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_control() {
                format!("\\u{{{:x}}}", c as u32)
            } else {
                c.to_string()
            }
        })
        .collect()
}
fn show_path(path: &Path) -> String {
    show(&path.to_string_lossy())
}

pub fn print_table(plan: &Plan, out: &mut dyn Write) -> Result<()> {
    writeln!(out, "workspace: {}", show_path(&plan.root))?;
    let rel = |p: &Path| show_path(p.strip_prefix(&plan.root).unwrap_or(p));
    let rows: Vec<[String; 4]> = plan
        .candidates
        .iter()
        .map(|c| {
            [
                rel(&c.path),
                c.class.label().to_string(),
                human(c.bytes),
                show(&c.status.label()),
            ]
        })
        .collect();
    let width = |i: usize, head: &str| {
        rows.iter()
            .map(|r| r[i].chars().count())
            .chain([head.len()])
            .max()
            .unwrap_or(0)
    };
    let (w0, w1, w2) = (width(0, "PATH"), width(1, "CLASS"), width(2, "SIZE"));
    writeln!(
        out,
        "{:<w0$}  {:<w1$}  {:>w2$}  STATUS",
        "PATH", "CLASS", "SIZE"
    )?;
    for r in &rows {
        writeln!(out, "{:<w0$}  {:<w1$}  {:>w2$}  {}", r[0], r[1], r[2], r[3])?;
    }
    let protected = plan
        .notes
        .iter()
        .filter(|n| n.starts_with(PROTECTED))
        .count();
    for note in plan.notes.iter().filter(|n| !n.starts_with(PROTECTED)) {
        writeln!(out, "note: {}", show(note))?;
    }
    writeln!(
        out,
        "{protected} evidence director{} protected and not scanned (captures, provenance, source packs, ...)",
        if protected == 1 { "y" } else { "ies" }
    )?;
    Ok(())
}

/// Re-verify a candidate through held directory descriptors, then remove it.
///
/// The workspace root is opened once and checked against the identity recorded when
/// planning. Every component down to the candidate is then opened relative to the
/// previous descriptor with `O_NOFOLLOW`, so replacing a component with a symlink
/// (or anything else) mid-run cannot redirect the deletion. The candidate must be
/// the directory that was planned (same device and inode), on the workspace's
/// filesystem, and contain no evidence; its contents are removed with `unlinkat`
/// relative to descriptors, and the result is checked before it counts as removed.
fn remove_verified(plan: &Plan, c: &Candidate) -> Result<()> {
    let project = c.project.as_deref();
    let rel: Vec<String> = match project {
        Some(p) => c
            .path
            .strip_prefix(p)
            .context("candidate left its project root")?
            .components()
            .map(|x| x.as_os_str().to_string_lossy().into_owned())
            .collect(),
        None => vec![],
    };
    // Exact-name allow-list, re-checked at the point of deletion.
    match c.class {
        Class::Build => ensure!(
            BUILD_PATHS
                .iter()
                .any(|b| b.len() == rel.len() && b.iter().zip(&rel).all(|(a, b)| a == b)),
            "not an allowed build path: {}",
            c.path.display()
        ),
        Class::Deps => ensure!(
            rel == ["node_modules"],
            "not node_modules: {}",
            c.path.display()
        ),
        Class::Cargo => ensure!(
            project.is_none(),
            "cargo target has a project: {}",
            c.path.display()
        ),
    }
    let below_root: Vec<OsString> = c
        .path
        .strip_prefix(&plan.root)
        .context("candidate is outside the workspace")?
        .components()
        .map(|x| match x {
            Component::Normal(n) => Ok(n.to_os_string()),
            _ => bail!("unexpected path component in {}", c.path.display()),
        })
        .collect::<Result<_>>()?;
    ensure!(!below_root.is_empty(), "candidate is the workspace itself");
    let project_depth = match project {
        Some(p) => Some(p.strip_prefix(&plan.root)?.components().count()),
        None => None,
    };

    let root = Dir::open(&plan.root).context("opening the workspace")?;
    ensure!(
        root.meta()?.id == plan.root_id,
        "the workspace was replaced since it was planned"
    );
    let dev = plan.root_id.0;
    let mut current = root;
    let mut parent: Option<Dir> = None;
    if project_depth == Some(0) {
        ensure!(
            is_project_root(&current),
            "no longer a project root: {}",
            plan.root.display()
        );
    }
    for (i, name) in below_root.iter().enumerate() {
        ensure!(
            current.names()?.iter().any(|n| n == name),
            "{} is not stored under exactly that name (or is gone)",
            c.path.display()
        );
        let meta = current.child_meta(name)?;
        ensure!(
            meta.kind == Kind::Dir,
            "{} is not a plain directory",
            c.path.display()
        );
        ensure!(
            meta.id.0 == dev,
            "{} is on another filesystem",
            c.path.display()
        );
        let next = current.child(name)?;
        ensure!(
            next.meta()?.id == meta.id,
            "{} changed while being opened",
            c.path.display()
        );
        if project_depth == Some(i + 1) {
            ensure!(
                is_project_root(&next),
                "no longer a project root: {}",
                c.path.display()
            );
        }
        parent = Some(std::mem::replace(&mut current, next));
    }
    let parent = parent.context("candidate has no parent")?;
    let last = below_root.last().context("candidate has no name")?;
    let meta = current.meta()?;
    if let Some(planned) = c.id {
        ensure!(
            meta.id == planned,
            "{} is not the directory that was planned",
            c.path.display()
        );
    }
    let scope = match c.class {
        Class::Build if rel.first().is_none_or(|r| r != "node_modules") => Scope::Build,
        Class::Cargo => Scope::Cargo,
        _ => Scope::Vendored,
    };
    if c.class == Class::Cargo {
        ensure!(
            is_cargo_target(&current, last),
            "not a cargo target: {}",
            c.path.display()
        );
        ensure!(
            !cargo_in_use(&current),
            "a cargo build is using this target (.cargo-lock is held)"
        );
    }
    let tree = survey(&current, &c.path, scope);
    ensure!(
        tree.evidence.is_none(),
        "contains evidence directory {}",
        tree.evidence.unwrap_or_default().display()
    );
    ensure!(
        tree.unreadable.is_empty(),
        "unreadable entries: {}",
        tree.unreadable.join("; ")
    );
    remove_contents(&current, &c.path, scope, dev, 0)?;
    parent
        .rmdir(last)
        .with_context(|| format!("removing {}", c.path.display()))?;
    ensure!(
        matches!(parent.child_meta(last), Err(e) if e.kind() == std::io::ErrorKind::NotFound),
        "{} is still present after removal",
        c.path.display()
    );
    Ok(())
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod toctou_tests {
    use super::*;
    use std::fs;

    #[test]
    fn capture_layout_at_candidate_root_is_refused_during_removal() {
        let tmp = std::env::temp_dir().join(format!(
            "rv2-unit-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let cand = tmp.join(".output");
        fs::create_dir_all(cand.join("site")).unwrap();
        fs::write(cand.join("site/index.html"), "captured evidence").unwrap();
        fs::write(cand.join("manifest.json"), "{}").unwrap();
        fs::write(cand.join("bundle.js"), "build").unwrap();
        let dir = Dir::open(&cand).unwrap();
        let dev = dir.meta().unwrap().id.0;
        let res = remove_contents(&dir, &cand, Scope::Build, dev, 0);
        let survived = cand.join("site/index.html").exists();
        fs::remove_dir_all(&tmp).ok();
        assert!(
            res.is_err(),
            "a capture layout appearing at the candidate root must be refused"
        );
        assert!(survived, "the captured site must survive");
    }
}

pub fn apply(plan: &Plan, out: &mut dyn Write) -> Result<Report> {
    let mut report = Report::default();
    for c in plan
        .candidates
        .iter()
        .filter(|c| c.status == Status::Remove)
    {
        writeln!(out, "removing {} ({})", show_path(&c.path), human(c.bytes))?;
        out.flush()?;
        match remove_verified(plan, c) {
            Ok(()) => {
                report.bytes += c.bytes;
                report.removed.push(c.path.clone());
            }
            Err(e) => {
                writeln!(out, "  failed: {}", show(&format!("{e:#}")))?;
                report.failed.push((c.path.clone(), format!("{e:#}")));
            }
        }
    }
    writeln!(
        out,
        "removed {} director{} totalling {} (apparent size); {} failed",
        report.removed.len(),
        if report.removed.len() == 1 {
            "y"
        } else {
            "ies"
        },
        human(report.bytes),
        report.failed.len()
    )?;
    Ok(report)
}

/// Entry point for the CLI: plan, print, and delete only with `options.apply`.
/// With `--apply` the result is an error (non-zero exit) when any removal failed or
/// any row was REFUSED, after everything removable has been removed.
pub fn run(workspace: &Path, options: &Options, out: &mut dyn Write) -> Result<Report> {
    let plan = plan(workspace, options)?;
    print_table(&plan, out)?;
    if !options.apply {
        writeln!(
            out,
            "dry run: {} reclaimable (apparent size). Nothing deleted; pass --apply to delete the 'remove' rows.",
            human(plan.removable_bytes())
        )?;
        return Ok(Report::default());
    }
    let report = apply(&plan, out)?;
    ensure!(
        report.failed.is_empty(),
        "{} removal(s) failed",
        report.failed.len()
    );
    ensure!(
        plan.refused() == 0,
        "{} row(s) were REFUSED and left in place; see the table",
        plan.refused()
    );
    Ok(report)
}
