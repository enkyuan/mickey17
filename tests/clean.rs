use mochi::clean::{self, Class, Options, Status};
use std::{
    collections::BTreeSet,
    fs,
    path::Path,
    time::{Duration, SystemTime},
};
use walkdir::WalkDir;

fn write(path: impl AsRef<Path>, text: &str) {
    let path = path.as_ref();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}
fn project(dir: &Path) {
    write(dir.join("package.json"), "{}");
    write(dir.join("bun.lock"), "");
    write(dir.join("src/app.tsx"), "export {}");
}
/// A project with every regenerable class present.
fn built(dir: &Path) {
    project(dir);
    for rel in [
        ".output/server/index.mjs",
        ".tanstack/tmp/route.txt",
        ".mochi/dist/index.html",
        "node_modules/.vite/deps/a.js",
        "node_modules/.nitro/cache.json",
        "node_modules/.vite-temp/t.js",
        "node_modules/react/index.js",
    ] {
        write(dir.join(rel), "x");
    }
    write(dir.join(".mochi/export.json"), "{\"input_sha256\":\"aa\"}");
    write(
        dir.join(".mochi/provenance/compat/controller.js"),
        "keep me",
    );
}
/// Every entry (symlinks included, never followed) with its kind.
fn snapshot(root: &Path) -> BTreeSet<String> {
    WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .map(|e| {
            let e = e.unwrap();
            let kind = if e.path_is_symlink() {
                "link"
            } else if e.file_type().is_dir() {
                "dir"
            } else {
                "file"
            };
            format!("{kind} {}", e.path().strip_prefix(root).unwrap().display())
        })
        .collect()
}
fn apply(workspace: &Path, options: Options) -> anyhow::Result<(String, clean::Report)> {
    let mut out = Vec::new();
    let report = clean::run(
        workspace,
        &Options {
            apply: true,
            ..options
        },
        &mut out,
    )?;
    Ok((String::from_utf8(out).unwrap(), report))
}
/// Plan and apply without turning REFUSED rows into an error (library level).
fn apply_lenient(workspace: &Path, options: Options) -> clean::Report {
    let plan = clean::plan(workspace, &options).unwrap();
    let mut out = Vec::new();
    clean::apply(&plan, &mut out).unwrap()
}
fn set_age(dir: &Path, seconds_ago: u64) {
    let when = SystemTime::now() - Duration::from_secs(seconds_ago);
    fs::File::open(dir).unwrap().set_modified(when).unwrap();
}
/// The tag cargo writes into its target directory.
const CARGO_TAG: &str = "Signature: 8a477f597d28d172789f06886806bc55\n# This file is a cache directory tag created by cargo.\n";
fn mochi_clean(cwd: &Path, home: &Path, args: &[&std::ffi::OsStr]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_mochi"))
        .arg("clean")
        .args(args)
        .current_dir(cwd)
        .env("HOME", home)
        .output()
        .unwrap()
}
fn workspace() -> tempfile::TempDir {
    tempfile::Builder::new().prefix("ws").tempdir().unwrap()
}
fn find<'a>(plan: &'a clean::Plan, suffix: &str) -> &'a clean::Candidate {
    plan.candidates
        .iter()
        .find(|c| c.path.ends_with(suffix))
        .unwrap_or_else(|| panic!("no candidate {suffix}: {:?}", plan.candidates))
}

// Value: protects=a dry run, the default, never changes the workspace; fails_when=any path, file or mtime-visible entry disappears without --apply; why_new=first clean tests; seam=real CLI binary; no_claim=apply behavior
#[test]
fn dry_run_is_default_and_deletes_nothing() {
    let ws = workspace();
    built(&ws.path().join("site-a"));
    let before = snapshot(ws.path());
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_mochi"))
        .args(["clean", "--deps"])
        .arg(ws.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("dry run"), "{stdout}");
    assert!(stdout.contains("site-a/.output"), "{stdout}");
    assert!(stdout.contains("node_modules"), "{stdout}");
    assert_eq!(before, snapshot(ws.path()));
    // Library dry run too, with every opt-in class selected.
    let plan = clean::plan(
        ws.path(),
        &Options {
            deps: true,
            cargo: true,
            ..Options::default()
        },
    )
    .unwrap();
    assert!(plan.removable_bytes() > 0);
    assert_eq!(before, snapshot(ws.path()));
}

// Value: protects=--apply removes exactly the allow-listed build directories and leaves sources, metadata and evidence in place; fails_when=a neighbouring file, node_modules (without --deps) or .mochi/provenance is removed; why_new=first clean tests; seam=library run; no_claim=disk savings
#[test]
fn apply_removes_only_exact_build_directories() {
    let ws = workspace();
    let p = ws.path().join("site-a");
    built(&p);
    let (stdout, report) = apply(ws.path(), Options::default()).unwrap();
    assert_eq!(report.removed.len(), 6);
    assert!(stdout.contains("removing "), "{stdout}");
    for gone in [
        ".output",
        ".tanstack",
        ".mochi/dist",
        "node_modules/.vite",
        "node_modules/.nitro",
        "node_modules/.vite-temp",
    ] {
        assert!(!p.join(gone).exists(), "{gone} should be gone");
    }
    for kept in [
        "package.json",
        "bun.lock",
        "src/app.tsx",
        ".mochi/export.json",
        ".mochi/provenance/compat/controller.js",
        "node_modules/react/index.js",
    ] {
        assert!(p.join(kept).exists(), "{kept} must remain");
    }
    // Idempotent: nothing left, still success.
    let (_, again) = apply(ws.path(), Options::default()).unwrap();
    assert!(again.removed.is_empty() && again.failed.is_empty());
}

// Value: protects=node_modules is removed only when --deps is given, and then whole; fails_when=deps are removed by default or the dependency cache subdirs are double counted; why_new=first clean tests; seam=library; no_claim=APFS clone accounting
#[test]
fn deps_are_opt_in() {
    let ws = workspace();
    let p = ws.path().join("site-a");
    built(&p);
    let plan = clean::plan(ws.path(), &Options::default()).unwrap();
    assert_eq!(find(&plan, "node_modules").status, Status::OptIn);
    apply(ws.path(), Options::default()).unwrap();
    assert!(p.join("node_modules/react/index.js").exists());
    let plan = clean::plan(
        ws.path(),
        &Options {
            deps: true,
            ..Options::default()
        },
    )
    .unwrap();
    assert!(plan.candidates.iter().all(|c| !c.path.ends_with(".vite")));
    apply(
        ws.path(),
        Options {
            deps: true,
            ..Options::default()
        },
    )
    .unwrap();
    assert!(!p.join("node_modules").exists());
    assert!(p.join("package.json").exists());
}

// Value: protects=evidence directories are never scanned or deleted even when they contain project roots and build-looking names; fails_when=any capture, provenance, source-pack, semantic-conversion, reconstruction, previous-projects or .git path loses a file; why_new=first clean tests; seam=library apply over a mixed workspace; no_claim=exhaustive evidence taxonomy
#[test]
fn evidence_directories_are_never_removed() {
    let ws = workspace();
    let r = ws.path();
    built(&r.join("site-a"));
    // Capture layout recognised by content, with an arbitrary name.
    write(r.join("trials/t/natural/zzz/manifest.json"), "{}");
    write(r.join("trials/t/natural/zzz/site/index.html"), "<html>");
    write(
        r.join("trials/t/natural/zzz/.output/precious.bin"),
        "evidence",
    );
    write(
        r.join("trials/t/natural/zzz/site/.output/p.bin"),
        "evidence",
    );
    // Capture layout by name, one holding a full project.
    built(&r.join("trials/t/natural/capture/project"));
    built(&r.join("trials/t/natural/capture-modules"));
    // Every other evidence name, each with a project and build outputs inside.
    for name in [
        "provenance",
        "source-packs",
        "semantic-conversion",
        "reconstruction",
        "previous-projects",
        "Capture",
        "PROVENANCE",
        ".git",
    ] {
        built(&r.join(name).join("proj"));
        write(r.join(name).join(".output/x"), "evidence");
    }
    // A project whose own provenance sits next to its build.
    write(r.join("site-b/.mochi/provenance/.output/keep"), "evidence");
    project(&r.join("site-b"));
    write(r.join("site-b/.output/a"), "build");
    let before = snapshot(r);
    let (_, report) = apply(
        r,
        Options {
            deps: true,
            cargo: true,
            ..Options::default()
        },
    )
    .unwrap();
    let after = snapshot(r);
    for lost in before.difference(&after) {
        assert!(
            lost.contains("site-a/") || lost.contains("site-b/.output"),
            "evidence removed: {lost}"
        );
        assert!(
            !lost.contains("provenance") && !lost.contains("capture"),
            "evidence removed: {lost}"
        );
    }
    assert!(report.removed.iter().all(|p| {
        let s = p.to_string_lossy();
        s.contains("site-a") || s.contains("site-b")
    }));
    assert!(r.join("site-b/.mochi/provenance/.output/keep").exists());
    assert!(
        r.join("site-a/.mochi/provenance/compat/controller.js")
            .exists()
    );
}

// Value: protects=a build directory that itself contains evidence is refused and survives; fails_when=remove_dir_all runs on .output holding a capture, provenance or .git; why_new=first clean tests; seam=library; no_claim=nothing
#[test]
fn candidate_containing_evidence_is_refused() {
    let ws = workspace();
    let p = ws.path().join("site");
    project(&p);
    write(
        p.join(".output/bundle/provenance/controller.js"),
        "evidence",
    );
    write(p.join(".tanstack/manifest.json"), "{}");
    write(p.join(".tanstack/site/index.html"), "<html>");
    write(p.join(".mochi/dist/.git/HEAD"), "ref");
    write(
        p.join("node_modules/pkg/.git/HEAD"),
        "ref: dep checkouts are fine",
    );
    let plan = clean::plan(
        ws.path(),
        &Options {
            deps: true,
            ..Options::default()
        },
    )
    .unwrap();
    for rel in [".output", ".tanstack", ".mochi/dist"] {
        assert!(
            matches!(find(&plan, rel).status, Status::Refused(_)),
            "{rel}: {:?}",
            find(&plan, rel).status
        );
    }
    assert_eq!(find(&plan, "node_modules").status, Status::Remove);
    let before = snapshot(ws.path());
    let report = apply_lenient(
        ws.path(),
        Options {
            deps: true,
            ..Options::default()
        },
    );
    assert_eq!(report.removed.len(), 1);
    let after = snapshot(ws.path());
    let lost: Vec<_> = before.difference(&after).collect();
    assert!(lost.iter().all(|l| l.contains("node_modules")), "{lost:?}");
}

// Value: protects=symlinks are never followed or deleted through; fails_when=a link target outside the workspace loses data, or a symlinked .output/.mochi/node_modules/project root is treated as a candidate; why_new=first clean tests; seam=library apply; no_claim=Windows junctions
#[cfg(unix)]
#[test]
fn symlinks_are_refused_and_targets_survive() {
    use std::os::unix::fs::symlink;
    let ws = workspace();
    let outside = workspace();
    let o = outside.path();
    write(o.join("precious/file"), "outside");
    write(o.join("dist/file"), "outside");
    write(o.join("modules/.vite/file"), "outside");
    write(o.join("proj/.output/file"), "outside");
    write(o.join("proj/package.json"), "{}");
    write(o.join("proj/bun.lock"), "");

    let p = ws.path().join("site");
    project(&p);
    symlink(o.join("precious"), p.join(".output")).unwrap();
    fs::create_dir_all(p.join("node_modules")).unwrap();
    symlink(o.join("modules/.vite"), p.join("node_modules/.vite")).unwrap();
    // .mochi itself is a link, so .mochi/dist resolves outside the project.
    symlink(o, p.join(".mochi")).unwrap();
    // A linked project root and a linked cargo target inside the workspace.
    symlink(o.join("proj"), ws.path().join("linked-project")).unwrap();
    symlink(o, ws.path().join("target")).unwrap();
    // A link nested inside a legitimately removable directory.
    write(p.join(".tanstack/real"), "x");
    symlink(o.join("precious"), p.join(".tanstack/link")).unwrap();

    let outside_before = snapshot(o);
    let plan = clean::plan(
        ws.path(),
        &Options {
            deps: true,
            cargo: true,
            ..Options::default()
        },
    )
    .unwrap();
    for rel in ["site/.output", "site/.mochi/dist"] {
        assert!(
            plan.candidates
                .iter()
                .all(|c| !(c.path.ends_with(rel) && c.status == Status::Remove)),
            "{rel} must not be removable"
        );
    }
    assert!(
        plan.candidates
            .iter()
            .all(|c| !c.path.starts_with(ws.path().join("linked-project")))
    );
    apply_lenient(
        ws.path(),
        Options {
            deps: true,
            cargo: true,
            ..Options::default()
        },
    );
    assert_eq!(outside_before, snapshot(o), "outside data changed");
    assert!(
        fs::symlink_metadata(p.join(".output"))
            .unwrap()
            .is_symlink()
    );
    assert!(fs::symlink_metadata(p.join(".mochi")).unwrap().is_symlink());
    assert!(
        fs::symlink_metadata(ws.path().join("linked-project"))
            .unwrap()
            .is_symlink()
    );
    // The nested link was removed as a link; its target is untouched.
    assert!(!p.join(".tanstack").exists());
    assert!(o.join("precious/file").exists());
}

// Value: protects=workspace argument validation; fails_when=a symlinked, evidence, file or missing workspace is accepted; why_new=first clean tests; seam=library; no_claim=nothing
#[cfg(unix)]
#[test]
fn unsafe_workspaces_are_refused() {
    use std::os::unix::fs::symlink;
    let base = workspace();
    let b = base.path();
    built(&b.join("real/site"));
    symlink(b.join("real"), b.join("link")).unwrap();
    assert!(clean::plan(&b.join("link"), &Options::default()).is_err());
    assert!(clean::plan(&b.join("missing"), &Options::default()).is_err());
    assert!(clean::plan(&b.join("real/site/package.json"), &Options::default()).is_err());
    for name in ["capture", "Provenance", "source-packs", "capture-final"] {
        built(&b.join(name).join("inner"));
        assert!(
            clean::plan(&b.join(name).join("inner"), &Options::default()).is_err(),
            "{name}"
        );
        assert!(
            clean::plan(&b.join(name), &Options::default()).is_err(),
            "{name}"
        );
    }
    write(b.join("cap/manifest.json"), "{}");
    write(b.join("cap/site/index.html"), "x");
    assert!(clean::plan(&b.join("cap"), &Options::default()).is_err());
    assert!(clean::plan(Path::new("/"), &Options::default()).is_err());
}

// Value: protects=only directories under a project root are candidates; fails_when=look-alike names, non-project .output, nested node_modules projects or outside siblings are removed; why_new=first clean tests; seam=library; no_claim=globbing is unsupported by design
#[test]
fn only_exact_names_under_project_roots_match() {
    let base = workspace();
    let ws = base.path().join("ws");
    // No bun.lock: not a project root.
    write(ws.join("loose/package.json"), "{}");
    write(ws.join("loose/.output/a"), "x");
    write(ws.join("notproject/.output/a"), "x");
    // Look-alikes inside a real project.
    let p = ws.join("site");
    project(&p);
    for rel in [
        ".output.bak/a",
        ".outputs/a",
        ".Output2/a",
        ".tanstack-old/a",
        "dist/a",
        ".mochi/distribution/a",
        "node_modules/.vite-cache/a",
        "node_modules/.nitro2/a",
        "node_modules/pkg/.output/a",
        "src/.output/a",
        "target/a",
    ] {
        write(p.join(rel), "x");
    }
    // A project nested inside node_modules is not scanned.
    project(&p.join("node_modules/dep"));
    write(p.join("node_modules/dep/.output/a"), "x");
    // A sibling outside the workspace.
    built(&base.path().join("sibling"));
    let before = snapshot(base.path());
    let (_, report) = apply(
        &ws,
        Options {
            deps: true,
            cargo: true,
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!(
        report.removed,
        vec![p.canonicalize().unwrap().join("node_modules")]
    );
    assert_eq!(
        before
            .difference(&snapshot(base.path()))
            .filter(|l| !l.contains("site/node_modules"))
            .count(),
        0
    );
    assert!(p.join("src/.output/a").exists() && p.join("target/a").exists());
}

// Value: protects=cargo target directories are removed only with --cargo and only when marked; fails_when=an unmarked "target" directory is removed or --cargo is implied; why_new=first clean tests; seam=library; no_claim=cargo's own cache semantics
#[test]
fn cargo_targets_require_flag_and_marker() {
    let ws = workspace();
    let r = ws.path();
    write(r.join("target/CACHEDIR.TAG"), CARGO_TAG);
    write(r.join("target/debug/big"), "xxxx");
    write(
        r.join("other/target/.rustc_info.json"),
        "{\"rustc_fingerprint\":1,\"outputs\":{}}",
    );
    write(r.join("other/target/release/x"), "x");
    write(r.join("unmarked/target/data"), "user data named target");
    let (_, none) = apply(r, Options::default()).unwrap();
    assert!(none.removed.is_empty());
    assert!(r.join("target/debug/big").exists());
    let plan = clean::plan(
        r,
        &Options {
            cargo: true,
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!(
        plan.candidates
            .iter()
            .filter(|c| c.class == Class::Cargo)
            .count(),
        2
    );
    apply(
        r,
        Options {
            cargo: true,
            ..Options::default()
        },
    )
    .unwrap();
    assert!(!r.join("target").exists() && !r.join("other/target").exists());
    assert!(r.join("unmarked/target/data").exists());
    assert!(r.join("other").exists());
}

// Value: protects=--keep and --keep-latest protection; fails_when=a kept project loses output, the newest of a family is removed, a mistyped --keep silently protects nothing, or families merge by accident; why_new=first clean tests; seam=library; no_claim=family grouping beyond the documented rule
#[test]
fn keep_and_keep_latest() {
    let ws = workspace();
    let r = ws.path();
    for name in ["natural-old", "natural-new", "natural-oldest", "garden"] {
        built(&r.join(name));
    }
    // Same capture hash => one family regardless of names; garden differs.
    write(
        r.join("garden/.mochi/export.json"),
        "{\"input_sha256\":\"bb\"}",
    );
    set_age(&r.join("natural-oldest/.output"), 300);
    set_age(&r.join("natural-old/.output"), 200);
    set_age(&r.join("natural-new/.output"), 10);
    for n in ["natural-oldest", "natural-old", "natural-new", "garden"] {
        for rel in [
            ".tanstack",
            ".mochi/dist",
            "node_modules/.vite",
            "node_modules/.nitro",
            "node_modules/.vite-temp",
        ] {
            set_age(&r.join(n).join(rel), 500);
        }
    }
    // Mistyped --keep fails before anything is planned or deleted.
    let before = snapshot(r);
    let err = apply(
        r,
        Options {
            keep: vec![r.join("nope")],
            ..Options::default()
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("--keep"), "{err}");
    assert_eq!(before, snapshot(r));

    let (_, report) = apply(
        r,
        Options {
            keep: vec![r.join("natural-oldest")],
            keep_latest: true,
            ..Options::default()
        },
    )
    .unwrap();
    assert!(r.join("natural-oldest/.output").exists(), "--keep");
    assert!(r.join("natural-new/.output").exists(), "newest of family");
    assert!(
        r.join("garden/.output").exists(),
        "only member of its family"
    );
    assert!(
        !r.join("natural-old/.output").exists(),
        "older sibling removed"
    );
    assert!(r.join("natural-old/src/app.tsx").exists());
    assert_eq!(report.removed.len(), 6);
    assert!(
        report
            .removed
            .iter()
            .all(|p| p.starts_with(r.canonicalize().unwrap().join("natural-old")))
    );
}

// Value: protects=documented family rule; fails_when=export.json hash or name-stem grouping changes without a doc update; why_new=first clean tests; seam=library; no_claim=semantic site identity
#[test]
fn family_rule() {
    let d = workspace();
    let a = d.path().join("head-natural");
    write(a.join(".mochi/export.json"), "{\"input_sha256\":\"abc\"}");
    assert_eq!(clean::family_of(&a), "capture:abc");
    // Without an input hash a project is its own family: names are never merged.
    let mut seen = BTreeSet::new();
    for name in [
        "natural",
        "runtime-natural-base",
        "natural-compat",
        "shop-v2",
        "shop-old",
    ] {
        let family = clean::family_of(&d.path().join(name));
        assert!(family.starts_with("path:"), "{name}: {family}");
        assert!(seen.insert(family));
    }
    // A symlinked export.json is not read.
    #[cfg(unix)]
    {
        let b = d.path().join("linked");
        write(
            d.path().join("elsewhere.json"),
            "{\"input_sha256\":\"zzz\"}",
        );
        fs::create_dir_all(b.join(".mochi")).unwrap();
        std::os::unix::fs::symlink(
            d.path().join("elsewhere.json"),
            b.join(".mochi/export.json"),
        )
        .unwrap();
        assert!(clean::family_of(&b).starts_with("path:"));
    }
}

// Value: protects=unreadable entries are reported and block deletion rather than being skipped silently; fails_when=a candidate with an unreadable subdirectory is removed or the problem goes unreported; why_new=first clean tests; seam=library with chmod; no_claim=root users (test is skipped)
#[cfg(unix)]
#[test]
fn unreadable_entries_are_reported_and_block_removal() {
    use std::os::unix::fs::PermissionsExt;
    let ws = workspace();
    let p = ws.path().join("site");
    project(&p);
    write(p.join(".output/locked/file"), "x");
    write(p.join(".tanstack/ok"), "x");
    write(p.join("hidden/inside"), "x");
    fs::set_permissions(p.join(".output/locked"), fs::Permissions::from_mode(0o000)).unwrap();
    fs::set_permissions(p.join("hidden"), fs::Permissions::from_mode(0o000)).unwrap();
    let restore = || {
        fs::set_permissions(p.join(".output/locked"), fs::Permissions::from_mode(0o755)).ok();
        fs::set_permissions(p.join("hidden"), fs::Permissions::from_mode(0o755)).ok();
    };
    if fs::read_dir(p.join(".output/locked")).is_ok() {
        restore();
        return; // running as root: permissions are not enforced
    }
    let plan = clean::plan(ws.path(), &Options::default());
    let result = plan.and_then(|plan| {
        let mut out = Vec::new();
        clean::print_table(&plan, &mut out)?;
        Ok((plan, String::from_utf8(out).unwrap()))
    });
    let (plan, table) = match result {
        Ok(v) => v,
        Err(e) => {
            restore();
            panic!("{e}");
        }
    };
    let output_status = find(&plan, ".output").status.clone();
    let tanstack_status = find(&plan, ".tanstack").status.clone();
    restore();
    assert!(
        matches!(output_status, Status::Refused(_)),
        "{output_status:?}"
    );
    assert_eq!(tanstack_status, Status::Remove);
    assert!(table.contains("unreadable"), "{table}");
    assert!(p.join(".output/locked/file").exists());
}

// Value: protects=the CLI prints each path before deleting and a final total, and exits non-zero for invalid workspaces; fails_when=apply is silent or a bad workspace exits 0; why_new=first clean tests; seam=real CLI binary; no_claim=output formatting
#[test]
fn cli_apply_prints_paths_and_total() {
    let ws = workspace();
    built(&ws.path().join("site"));
    let run = |args: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_mochi"))
            .arg("clean")
            .args(args)
            .output()
            .unwrap()
    };
    let output = run(&["--apply", ws.path().to_str().unwrap()]);
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("removing ") && stdout.contains(".output"),
        "{stdout}"
    );
    assert!(stdout.contains("removed 6 directories"), "{stdout}");
    assert!(!ws.path().join("site/.output").exists());
    let bad = run(&[ws.path().join("missing").to_str().unwrap()]);
    assert!(!bad.status.success());
    let bad_keep = run(&[
        "--keep",
        "/definitely/not/here",
        ws.path().to_str().unwrap(),
    ]);
    assert!(!bad_keep.status.success());
}

// Value: protects=a workspace inside a capture recognised only by its layout is refused; fails_when=`mochi clean capture/site` deletes build-looking directories inside captured evidence; why_new=review found only the workspace itself was layout-checked; seam=library apply; no_claim=nothing
#[test]
fn workspace_inside_capture_layout_is_refused() {
    let ws = workspace();
    let cap = ws.path().join("trials/zzz");
    write(cap.join("manifest.json"), "{}");
    built(&cap.join("site/proj"));
    let before = snapshot(ws.path());
    for inner in ["site", "site/proj", "site/proj/src"] {
        let err = apply(&cap.join(inner), Options::default()).unwrap_err();
        assert!(err.to_string().contains("capture"), "{inner}: {err}");
    }
    assert_eq!(before, snapshot(ws.path()));
}

// Value: protects=only the exact on-disk names are candidates, so a case-insensitive filesystem cannot make `.OUTPUT` a "remove" row that apply then refuses; fails_when=the dry run lists `.output` for a `.OUTPUT` directory or --apply fails on it; why_new=review saw a lying table and a non-idempotent failing --apply on APFS; seam=library; no_claim=behaviour on case-sensitive filesystems is trivially the same
#[test]
fn case_variants_of_build_names_are_not_candidates() {
    let ws = workspace();
    let p = ws.path().join("site");
    project(&p);
    write(p.join(".OUTPUT/f"), "not an allow-listed name");
    write(p.join(".Mochi/DIST/f"), "x");
    let plan = clean::plan(ws.path(), &Options::default()).unwrap();
    assert!(plan.candidates.is_empty(), "{:?}", plan.candidates);
    let (_, report) = apply(ws.path(), Options::default()).unwrap();
    assert!(report.removed.is_empty() && report.failed.is_empty());
    assert!(p.join(".OUTPUT/f").exists() && p.join(".Mochi/DIST/f").exists());
}

// Value: protects=--keep always protects what it names: a path inside a project keeps that project, and a path that protects nothing in the workspace (including a relative path that resolves in the current directory instead) is an error, never silently ignored; fails_when=--keep site/.output still deletes site/.output, or --keep site resolves to ./site outside the workspace and deletes ws/site/.output; why_new=both were reproduced in review; seam=library and real CLI with its own working directory; no_claim=nothing
#[test]
fn keep_is_never_silently_ignored() {
    let base = workspace();
    let b = base.path();
    let ws = b.join("ws");
    built(&ws.join("site"));
    built(&ws.join("other"));
    // A path inside a project keeps that whole project.
    let plan = clean::plan(
        &ws,
        &Options {
            keep: vec![ws.join("site/.output")],
            ..Options::default()
        },
    )
    .unwrap();
    for c in plan
        .candidates
        .iter()
        .filter(|c| c.path.starts_with(ws.canonicalize().unwrap().join("site")))
    {
        assert!(matches!(c.status, Status::Kept | Status::OptIn), "{c:?}");
        assert_ne!(c.status, Status::Remove, "{c:?}");
    }
    // An existing path outside the workspace protects nothing: refused.
    fs::create_dir_all(b.join("elsewhere")).unwrap();
    let err = clean::plan(
        &ws,
        &Options {
            keep: vec![b.join("elsewhere")],
            ..Options::default()
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("outside the workspace"), "{err}");
    // A relative --keep that exists both in the cwd and in the workspace means
    // the workspace one.
    let cwd = b.join("cwd");
    fs::create_dir_all(cwd.join("site")).unwrap();
    let out = mochi_clean(
        &cwd,
        &b.join("home"),
        &[
            "--apply".as_ref(),
            "--keep".as_ref(),
            "site".as_ref(),
            ws.as_os_str(),
        ],
    );
    assert!(out.status.success(), "{out:?}");
    assert!(ws.join("site/.output").exists(), "kept project lost output");
    assert!(!ws.join("other/.output").exists());
    // A relative --keep that exists only in the cwd is refused.
    fs::create_dir_all(cwd.join("only-here")).unwrap();
    let out = mochi_clean(
        &cwd,
        &b.join("home"),
        &["--keep".as_ref(), "only-here".as_ref(), ws.as_os_str()],
    );
    assert!(!out.status.success(), "{out:?}");
}

// Value: protects=only a genuine cargo target is a cargo candidate, and `.git` inside one is evidence; fails_when=a user directory named target with a foreign or empty CACHEDIR.TAG / .rustc_info.json, a project root named target, or a marked target holding a .git repository is deleted with --cargo; why_new=all three were deleted in review; seam=library apply; no_claim=a deliberately forged cargo tag
#[test]
fn spoofed_cargo_markers_are_not_targets() {
    let ws = workspace();
    let r = ws.path();
    write(r.join("notes/target/CACHEDIR.TAG"), "x");
    write(r.join("notes/target/capture-notes/n.txt"), "evidence");
    write(
        r.join("pycache/target/CACHEDIR.TAG"),
        "Signature: 8a477f597d28d172789f06886806bc55\n# created by pytest\n",
    );
    write(r.join("pycache/target/data"), "user data");
    write(r.join("empty/target/.rustc_info.json"), "");
    write(r.join("empty/target/data"), "user data");
    project(&r.join("projects/target"));
    write(r.join("projects/target/CACHEDIR.TAG"), CARGO_TAG);
    write(r.join("repo/target/CACHEDIR.TAG"), CARGO_TAG);
    write(r.join("repo/target/.git/HEAD"), "ref: refs/heads/main");
    let before = snapshot(r);
    let options = Options {
        cargo: true,
        deps: true,
        ..Options::default()
    };
    let plan = clean::plan(r, &options).unwrap();
    let cargo: Vec<_> = plan
        .candidates
        .iter()
        .filter(|c| c.class == Class::Cargo)
        .collect();
    assert_eq!(cargo.len(), 1, "{cargo:?}");
    assert!(matches!(cargo[0].status, Status::Refused(_)), "{cargo:?}");
    let report = apply_lenient(r, options);
    assert!(report.removed.is_empty(), "{:?}", report.removed);
    assert_eq!(before, snapshot(r));
}

// Value: protects=a symlinked workspace is refused however it is spelled, and the HOME guard covers directories containing HOME; fails_when=`link/`, `link/.` or `link/..` is accepted, or a parent of HOME is cleaned; why_new=review showed lstat follows `link/` and only HOME itself was refused; seam=library and CLI with HOME set; no_claim=symlinks in ancestors of the workspace (allowed, e.g. /tmp on macOS)
#[cfg(unix)]
#[test]
fn symlinked_workspace_spellings_and_home_ancestors_are_refused() {
    use std::os::unix::fs::symlink;
    let base = workspace();
    let b = base.path();
    built(&b.join("real/site"));
    fs::create_dir_all(b.join("real/sub")).unwrap();
    symlink(b.join("real/sub"), b.join("link")).unwrap();
    let link = b.join("link").into_os_string().into_string().unwrap();
    for spelling in [
        format!("{link}/"),
        format!("{link}/."),
        format!("{link}//"),
        format!("{link}/.."),
    ] {
        assert!(
            clean::plan(Path::new(&spelling), &Options::default()).is_err(),
            "{spelling}"
        );
    }
    let home = b.join("home");
    fs::create_dir_all(&home).unwrap();
    for target in [b.to_path_buf(), home.clone()] {
        let out = mochi_clean(b, &home, &["--apply".as_ref(), target.as_os_str()]);
        assert!(!out.status.success(), "{target:?}: {out:?}");
    }
    assert!(b.join("real/site/.output").exists());
}

// Value: protects=deletion goes through held descriptors, so swapping a parent directory for a symlink while --apply runs cannot delete outside the workspace; fails_when=any file in the mirror tree outside the workspace is removed; why_new=a path-based remove_dir_all let a swapped parent redirect deletions; seam=real renameat(RENAME_SWAP) racing apply; no_claim=every interleaving was exercised
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn swapping_a_parent_for_a_symlink_during_apply_cannot_delete_outside() {
    use rustix::fs::{CWD, RenameFlags, renameat_with};
    use std::sync::atomic::{AtomicBool, Ordering};
    let base = workspace();
    let ws = base.path().join("ws");
    let outside = base.path().join("outside");
    const PROJECTS: usize = 400;
    for n in 0..PROJECTS {
        let name = format!("p{n:04}");
        let p = ws.join("grp").join(&name);
        project(&p);
        for i in 0..20 {
            write(p.join(format!(".output/chunk/f{i}.js")), "build");
        }
        let mirror = outside.join("v").join(&name);
        project(&mirror);
        write(mirror.join(".output/precious"), "outside evidence");
    }
    std::os::unix::fs::symlink(outside.join("v"), ws.join("lnk")).unwrap();
    let plan = clean::plan(&ws, &Options::default()).unwrap();
    assert_eq!(plan.candidates.len(), PROJECTS);
    let before = snapshot(&outside);
    let stop = AtomicBool::new(false);
    let (grp, lnk) = (ws.join("grp"), ws.join("lnk"));
    let report = std::thread::scope(|scope| {
        scope.spawn(|| {
            while !stop.load(Ordering::Relaxed) {
                let _ = renameat_with(CWD, &grp, CWD, &lnk, RenameFlags::EXCHANGE);
            }
        });
        let mut out = Vec::new();
        let report = clean::apply(&plan, &mut out).unwrap();
        stop.store(true, Ordering::Relaxed);
        report
    });
    assert_eq!(
        before,
        snapshot(&outside),
        "files outside the workspace changed"
    );
    assert_eq!(report.removed.len() + report.failed.len(), PROJECTS);
}

// Value: protects=a project directory replaced by a symlink (or another directory) between plan and apply is refused; fails_when=the link target outside the workspace loses data or the replacement is deleted as if it were the planned directory; why_new=deterministic form of the swap race; seam=library plan then apply; no_claim=nothing
#[cfg(unix)]
#[test]
fn directories_replaced_after_planning_are_refused() {
    use std::os::unix::fs::symlink;
    let base = workspace();
    let ws = base.path().join("ws");
    let outside = base.path().join("outside");
    project(&ws.join("site"));
    write(ws.join("site/.output/a"), "build");
    project(&ws.join("other"));
    write(ws.join("other/.output/a"), "build");
    project(&outside);
    write(outside.join(".output/precious"), "outside");
    let plan = clean::plan(&ws, &Options::default()).unwrap();
    // `site` becomes a symlink to a real project outside; `other/.output` is
    // replaced by a different directory.
    fs::rename(ws.join("site"), ws.join("site.moved")).unwrap();
    symlink(&outside, ws.join("site")).unwrap();
    fs::rename(ws.join("other/.output"), ws.join("other/.output.old")).unwrap();
    write(ws.join("other/.output/new"), "not planned");
    let before = snapshot(&outside);
    let mut out = Vec::new();
    let report = clean::apply(&plan, &mut out).unwrap();
    assert!(report.removed.is_empty(), "{report:?}");
    assert_eq!(report.failed.len(), 2, "{report:?}");
    assert_eq!(before, snapshot(&outside));
    assert!(ws.join("other/.output/new").exists());
    assert!(ws.join("site.moved/.output/a").exists());
}

// Value: protects=capture detection also matches captures whose manifest.json or site/ is a symlink; fails_when=a build directory holding such a capture is removed, or such a directory is scanned as ordinary; why_new=lstat-based predicate missed them; seam=library; no_claim=every capture shape
#[cfg(unix)]
#[test]
fn captures_with_symlinked_manifest_or_site_are_evidence() {
    use std::os::unix::fs::symlink;
    let ws = workspace();
    let outside = workspace();
    write(outside.path().join("manifest.json"), "{}");
    fs::create_dir_all(outside.path().join("site-real")).unwrap();
    let p = ws.path().join("site");
    project(&p);
    // .output/frozen: manifest.json is a link, site/ a real directory.
    write(p.join(".output/frozen/site/index.html"), "<html>");
    symlink(
        outside.path().join("manifest.json"),
        p.join(".output/frozen/manifest.json"),
    )
    .unwrap();
    // .tanstack/frozen: manifest.json real, site is a link (even a dangling one).
    write(p.join(".tanstack/frozen/manifest.json"), "{}");
    symlink(
        outside.path().join("nowhere"),
        p.join(".tanstack/frozen/site"),
    )
    .unwrap();
    // A scanned directory of the same shapes is protected rather than entered.
    write(ws.path().join("loose/manifest.json"), "{}");
    symlink(
        outside.path().join("site-real"),
        ws.path().join("loose/site"),
    )
    .unwrap();
    project(&ws.path().join("loose/inner"));
    write(ws.path().join("loose/inner/.output/x"), "evidence");
    let plan = clean::plan(ws.path(), &Options::default()).unwrap();
    for rel in [".output", ".tanstack"] {
        assert!(
            matches!(find(&plan, rel).status, Status::Refused(_)),
            "{rel}: {:?}",
            find(&plan, rel).status
        );
    }
    apply_lenient(ws.path(), Options::default());
    assert!(p.join(".output/frozen/site/index.html").exists());
    assert!(p.join(".tanstack/frozen/manifest.json").exists());
    assert!(ws.path().join("loose/inner/.output/x").exists());
}

// Value: protects=--keep-latest chooses the same project with and without --deps, so a dry run predicts the --deps run; fails_when=node_modules cache directories stop counting as build time when --deps is given; why_new=built time was computed from a flag-dependent path set; seam=library; no_claim=nothing
#[test]
fn keep_latest_choice_does_not_depend_on_deps_flag() {
    let ws = workspace();
    let r = ws.path();
    for name in ["a", "b"] {
        built(&r.join(name));
        write(
            r.join(name).join(".mochi/export.json"),
            "{\"input_sha256\":\"same\"}",
        );
        for rel in [".output", ".tanstack", ".mochi/dist"] {
            set_age(&r.join(name).join(rel), 1000);
        }
        for rel in [
            "node_modules/.vite",
            "node_modules/.nitro",
            "node_modules/.vite-temp",
        ] {
            set_age(&r.join(name).join(rel), 1000);
        }
    }
    // Only `a` has a recent build, and it is in node_modules/.vite.
    set_age(&r.join("a/node_modules/.vite"), 5);
    let latest = |deps: bool| -> Vec<String> {
        let plan = clean::plan(
            r,
            &Options {
                deps,
                keep_latest: true,
                ..Options::default()
            },
        )
        .unwrap();
        plan.candidates
            .iter()
            .filter(|c| c.status == Status::KeptLatest)
            .map(|c| {
                c.path
                    .strip_prefix(&plan.root)
                    .unwrap()
                    .display()
                    .to_string()
            })
            .filter(|p| p.starts_with("a/") || p.starts_with("b/"))
            .map(|p| p[..1].to_string())
            .collect()
    };
    let (without, with) = (latest(false), latest(true));
    assert!(
        !without.is_empty() && without.iter().all(|p| p == "a"),
        "{without:?}"
    );
    assert!(
        !with.is_empty() && with.iter().all(|p| p == "a"),
        "{with:?}"
    );
}

// Value: protects=--keep never protects a different project than the one named; fails_when=a --keep that goes through a symlink inside the workspace is accepted; why_new=the link target was kept instead of the named project; seam=library; no_claim=nothing
#[cfg(unix)]
#[test]
fn keep_through_a_symlink_is_refused() {
    use std::os::unix::fs::symlink;
    let base = workspace();
    let ws = base.path().join("ws");
    built(&ws.join("x/p"));
    built(&ws.join("y"));
    symlink(ws.join("y"), ws.join("x/p/ylink")).unwrap();
    symlink(ws.join("x"), ws.join("xlink")).unwrap();
    for keep in [ws.join("x/p/ylink"), ws.join("xlink/p")] {
        let err = clean::plan(
            &ws,
            &Options {
                keep: vec![keep.clone()],
                ..Options::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("symlink"), "{keep:?}: {err}");
    }
    // A real path still works, even though the temp directory itself may sit
    // behind a symlink (/tmp on macOS).
    let plan = clean::plan(
        &ws,
        &Options {
            keep: vec![ws.join("x/p")],
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!(find(&plan, "x/p/.output").status, Status::Kept);
    assert_eq!(find(&plan, "y/.output").status, Status::Remove);
}

// Value: protects=--apply exits non-zero when any row was REFUSED, after removing what it can; fails_when=a refused row still exits 0; why_new=silent partial success; seam=real CLI binary; no_claim=nothing
#[test]
fn apply_exits_nonzero_when_a_row_was_refused() {
    let ws = workspace();
    let p = ws.path().join("site");
    project(&p);
    write(p.join(".output/provenance/x"), "evidence");
    write(p.join(".tanstack/ok"), "x");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_mochi"))
        .args(["clean", "--apply"])
        .arg(ws.path())
        .output()
        .unwrap();
    assert!(!out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("REFUSED"));
    assert!(!p.join(".tanstack").exists());
    assert!(p.join(".output/provenance/x").exists());
    // A dry run with the same refusal still exits 0.
    let dry = std::process::Command::new(env!("CARGO_BIN_EXE_mochi"))
        .arg("clean")
        .arg(ws.path())
        .output()
        .unwrap();
    assert!(dry.status.success());
}

// Value: protects=control characters in directory names cannot forge table rows or terminal escapes; fails_when=a newline or ESC from a file name reaches the output raw; why_new=names were printed unescaped; seam=library print_table and apply; no_claim=bidi or width tricks
#[cfg(unix)]
#[test]
fn control_characters_in_names_are_escaped() {
    let ws = workspace();
    let p = ws.path().join("evil\nremove  forged\u{1b}[2J");
    project(&p);
    write(p.join(".output/a"), "x");
    let plan = clean::plan(ws.path(), &Options::default()).unwrap();
    let mut out = Vec::new();
    clean::print_table(&plan, &mut out).unwrap();
    clean::apply(&plan, &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(!text.contains('\u{1b}'), "{text:?}");
    assert!(
        text.contains("\\u{a}") && text.contains("\\u{1b}"),
        "{text}"
    );
    assert!(
        !text.lines().any(|l| l.starts_with("remove  forged")),
        "{text}"
    );
}

// Value: protects=--cargo refuses a target while a build holds its .cargo-lock; fails_when=a locked target is removed; why_new=deleting under a running build; seam=library with a held flock; no_claim=cargo's locking protocol beyond the lock file
#[cfg(unix)]
#[test]
fn cargo_target_with_a_held_build_lock_is_refused() {
    let ws = workspace();
    let r = ws.path();
    write(r.join("target/CACHEDIR.TAG"), CARGO_TAG);
    write(r.join("target/debug/.cargo-lock"), "");
    write(r.join("target/debug/big"), "xxxx");
    let options = Options {
        cargo: true,
        ..Options::default()
    };
    let lock = fs::File::open(r.join("target/debug/.cargo-lock")).unwrap();
    lock.lock().unwrap();
    let plan = clean::plan(r, &options).unwrap();
    assert!(matches!(find(&plan, "target").status, Status::Refused(_)));
    let mut out = Vec::new();
    assert!(clean::apply(&plan, &mut out).unwrap().removed.is_empty());
    assert!(r.join("target/debug/big").exists());
    drop(lock);
    // Another test's child process may briefly inherit the descriptor between
    // fork and exec, keeping the lock alive; wait that out.
    let mut status = Status::OptIn;
    for _ in 0..100 {
        status = find(&clean::plan(r, &options).unwrap(), "target")
            .status
            .clone();
        if status == Status::Remove {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(status, Status::Remove);
}

// Value: protects=handoff, references, qualification and verification* are evidence for the scan and a workspace cannot be one; fails_when=a project inside them loses its build output; why_new=names added after review; seam=library; no_claim=exhaustive taxonomy
#[test]
fn extra_workspace_evidence_names_are_protected() {
    let ws = workspace();
    let r = ws.path();
    for name in ["handoff", "references", "qualification", "verification-gpu"] {
        built(&r.join(name).join("proj"));
    }
    built(&r.join("site"));
    let (_, report) = apply(r, Options::default()).unwrap();
    assert!(
        report
            .removed
            .iter()
            .all(|p| p.starts_with(r.canonicalize().unwrap().join("site")))
    );
    for name in ["handoff", "references", "qualification", "verification-gpu"] {
        assert!(r.join(name).join("proj/.output").exists(), "{name}");
        assert!(
            clean::plan(&r.join(name), &Options::default()).is_err(),
            "{name}"
        );
    }
}

// Value: protects=a workspace that contains git repositories is flagged loudly; fails_when=cleaning a source tree gives no warning; why_new=the parent-of-repos case; seam=library; no_claim=nothing
#[test]
fn workspace_containing_repositories_warns() {
    let ws = workspace();
    built(&ws.path().join("proj"));
    fs::create_dir_all(ws.path().join("repo/.git")).unwrap();
    let plan = clean::plan(ws.path(), &Options::default()).unwrap();
    let mut out = Vec::new();
    clean::print_table(&plan, &mut out).unwrap();
    assert!(String::from_utf8(out).unwrap().contains("WARNING"));
}
