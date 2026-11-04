use mosha::{edit, mirror, serve, util};
use url::Url;

#[test]
fn default_port_upgrade_stays_on_site_but_other_ports_do_not() {
    let http = Url::parse("http://example.com/").unwrap();
    assert!(mirror::same_site(
        &http,
        &Url::parse("https://example.com/").unwrap()
    ));
    assert!(!mirror::same_site(
        &http,
        &Url::parse("https://example.com:8443/").unwrap()
    ));
    assert!(!mirror::same_site(
        &http,
        &Url::parse("https://other.com/").unwrap()
    ));
}
#[test]
fn rehost_nondefault_port_does_not_change_default_origin() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("index.html");
    std::fs::write(&p, "https://acme.com/default https://acme.com:8443/private").unwrap();
    edit::rehost(t.path(), "https://acme.com:8443", "https://globex.com").unwrap();
    assert_eq!(
        std::fs::read_to_string(p).unwrap(),
        "https://acme.com/default https://globex.com/private"
    );
}
#[test]
fn cli_replacement_preserves_json_escaping_and_text_row_lengths() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("index.rsc");
    std::fs::write(
        &p,
        "0:[\"$\",\"h1\",null,{\"children\":\"Acme\"}]\n1:T4,Acme",
    )
    .unwrap();
    let new = "A\"B\\C\n雪";
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_mosha"))
        .args([
            "replace",
            t.path().to_str().unwrap(),
            "Acme",
            new,
            "--exact-case",
        ])
        .status()
        .unwrap();
    assert!(status.success());
    let saved = std::fs::read_to_string(p).unwrap();
    let (row, text) = saved.split_once('\n').unwrap();
    let tree: serde_json::Value = serde_json::from_str(row.strip_prefix("0:").unwrap()).unwrap();
    assert_eq!(tree[3]["children"], new);
    assert_eq!(text, format!("1:T{:x},{new}", new.len()));
}
#[cfg(unix)]
#[tokio::test]
async fn percent_named_directory_checks_exact_index_symlink() {
    use tower::ServiceExt;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let dir = root.path().join("a%2fb");
    std::fs::create_dir(&dir).unwrap();
    let secret = outside.path().join("marker");
    std::fs::write(&secret, "outside").unwrap();
    std::os::unix::fs::symlink(secret, dir.join("index.html")).unwrap();
    let response = serve::router(root.path().into())
        .oneshot(
            axum::http::Request::builder()
                .uri("/a%252fb")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
    assert!(util::write(root.path(), &dir.join("index.html"), b"overwritten").is_err());
}
#[cfg(unix)]
#[test]
fn interrupted_edit_keeps_original_file() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("index.html");
    let original = "Acme ".repeat(600);
    std::fs::write(&p, &original).unwrap();
    let output = std::process::Command::new("sh")
        .args([
            "-c",
            "ulimit -f 1; exec \"$1\" replace \"$2\" Acme Globex --exact-case",
            "sh",
            env!("CARGO_BIN_EXE_mosha"),
            t.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(std::fs::read_to_string(p).unwrap(), original);
}

#[test]
fn replacement_treats_json_looking_long_text_as_plain_text() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("index.rsc");
    let text = r#"a:{"Acme":"Acme"}"#;
    std::fs::write(&p, format!("1:T{:x},{text}", text.len())).unwrap();
    let r = edit::Replacer::new("Acme", "A\"B", false, true).unwrap();
    edit::replace(t.path(), &r).unwrap();
    let replaced = text.replace("Acme", "A\"B");
    assert_eq!(
        std::fs::read_to_string(p).unwrap(),
        format!("1:T{:x},{replaced}", replaced.len())
    );
}

#[test]
fn failed_mirror_write_does_not_overwrite_existing_bytes() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("index.rsc");
    util::write(t.path(), &p, b"first").unwrap();
    assert!(util::write(t.path(), &p, b"second").is_err());
    assert_eq!(std::fs::read(p).unwrap(), b"first");
}

#[test]
fn json_replacement_preserves_decoded_asset_paths_and_matches_html() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("index.html");
    let stream = "0:[\"$\",\"img\",null,{\"src\":\"/Acme/logo.png\",\"alt\":\"Acme\"}]\n";
    let html = format!(
        "<img src=\"/Acme/logo.png\" alt=\"Acme\"><script>self.__next_f.push({})</script>",
        serde_json::json!([1, stream])
    );
    std::fs::write(&p, html).unwrap();
    let r = edit::Replacer::new("Acme", "Globex", false, true).unwrap();
    edit::replace(t.path(), &r).unwrap();
    let saved = std::fs::read_to_string(p).unwrap();
    assert!(saved.contains("src=\"/Acme/logo.png\" alt=\"Globex\""));
    mosha::flight::edit_flight(&saved, &|row| {
        let tree: serde_json::Value =
            serde_json::from_str(row.strip_prefix("0:").unwrap()).unwrap();
        assert_eq!(tree[3]["src"], "/Acme/logo.png");
        assert_eq!(tree[3]["alt"], "Globex");
        row.to_string()
    })
    .unwrap();
}
