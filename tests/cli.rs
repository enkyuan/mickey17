// Value: protects=CLI and library reject invalid jobs and overlapping cache storage; fails_when=unsafe options proceed or offline analyze dispatch breaks; why_new=no existing CLI pipeline tests; seam=none; no_claim=Chromium clone success
#[test]
fn pipeline_cli_and_library_validate_before_network_or_writes() {
    use mochi::{crawl, mirror};
    let root = tempfile::tempdir().unwrap();
    for jobs in ["0", "17"] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_mochi"))
            .args(["clone", "http://127.0.0.1:1", "--jobs", jobs])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stderr).contains("--jobs"));
    }
    let defaults = crawl::PipelineOptions::default();
    assert!(!defaults.analyze && defaults.cache_dir.is_none());
    assert_eq!(defaults.jobs, 4);
    let client = reqwest::blocking::Client::new();
    let url = url::Url::parse("http://127.0.0.1:1/").unwrap();
    for jobs in [0, 17] {
        let out = root.path().join(format!("jobs-{jobs}"));
        let error = mirror::mirror_with_options(
            &client,
            std::slice::from_ref(&url),
            Default::default(),
            &out,
            &mirror::DownloadOptions {
                jobs,
                cache_dir: None,
            },
        )
        .err()
        .unwrap();
        assert!(error.to_string().contains("jobs"));
        assert!(!out.exists());
        let error = crawl::clone_with_options(
            url.as_str(),
            &out,
            &crawl::Options {
                max_pages: 1,
                extract: false,
                keep_trackers: false,
                ignore_robots: false,
            },
            &crawl::PipelineOptions {
                jobs,
                analyze: false,
                cache_dir: None,
            },
        )
        .err()
        .unwrap();
        assert!(error.to_string().contains("jobs"));
        assert!(!out.exists());
    }
    for (output, cache) in [
        ("site", "site/cache"),
        ("cache/site", "cache"),
        ("same", "same"),
    ] {
        let error = mirror::mirror_with_options(
            &client,
            std::slice::from_ref(&url),
            Default::default(),
            &root.path().join(output),
            &mirror::DownloadOptions {
                jobs: 1,
                cache_dir: Some(root.path().join(cache)),
            },
        )
        .err()
        .unwrap();
        assert!(error.to_string().contains("overlap"));
    }
    let pages = root.path().join("offline/analysis/pages");
    std::fs::create_dir_all(&pages).unwrap();
    std::fs::write(pages.join("one.json"),r#"{"schema_version":1,"url":"http://127.0.0.1:1/unavailable","viewport":{"width":1,"height":1},"truncated":false,"styles":[{}],"nodes":[{"parent":null,"tag":"body","attributes":{},"text":"","style":0}]}"#).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_mochi"))
        .arg("analyze")
        .arg(root.path().join("offline"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Analyzed 1 nodes"));
    assert!(root.path().join("offline/analysis/report.json").exists());
}
