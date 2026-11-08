use mochi::{
    edit::{self, Replacer},
    flight, mirror, trackers, util,
};
use url::Url;
fn page(stream: &str, split: usize) -> String {
    format!(
        "<h1>Acme</h1><script nonce=\"abc\">self.__next_f.push({})</script><script>self.__next_f.push({})</script>",
        serde_json::json!([1, &stream[..split]]),
        serde_json::json!([1, &stream[split..]])
    )
}
#[test]
fn flight_split_unicode_and_injection() {
    let stream = "0:[\"Acme\"]\n1:T8,Acme —2:null\n";
    let html = page(stream, 6);
    let edited = flight::edit_flight(&html, &|s| s.replace("Acme", "Globex")).unwrap();
    assert!(edited.contains("nonce=\"abc\""));
    assert!(edited.contains("1:Ta,Globex —"));
    assert_eq!(edited.matches("self.__next_f.push").count(), 1);
    let escaped = flight::edit_flight(&html, &|s| s.replace("Acme", "</script>\u{2028}")).unwrap();
    assert!(escaped.contains("\\u003c/script>\\u2028"));
    assert_eq!(flight::edit_flight(&html, &str::to_string).unwrap(), html);
    assert!(flight::map_rows("1:T99,short", &str::to_string).is_err());
    assert!(flight::map_rows("1:T1,é", &str::to_string).is_err());
    let outside = flight::outside_flight(&html, &|s| s.replace("Acme", "X"));
    assert!(outside.starts_with("<h1>X</h1>"));
    assert!(outside.contains("Acme"));
}
#[test]
fn replacements_preserve_urls_paths_and_literal_dollars() {
    let r = Replacer::new("Acme", "Globex", true, true).unwrap();
    assert_eq!(
        r.replace("Acme ACME acme Acmes heyAcme https://acme.com/Acme '/acme/logo.png' tel:Acme"),
        "Globex GLOBEX globex Acmes heyAcme https://acme.com/Acme '/acme/logo.png' tel:Acme"
    );
    assert_eq!(
        Replacer::new("Acme", "$1", false, false)
            .unwrap()
            .replace("heyAcme"),
        "hey$1"
    );
    assert!(Replacer::new("", "x", true, true).is_err());
}
#[test]
fn editing_updates_disk_html_and_rsc() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path();
    std::fs::write(p.join("index.html"), page("0:[\"Acme\"]\n", 5)).unwrap();
    std::fs::write(p.join("index.rsc"), "1:T4,Acme").unwrap();
    let r = Replacer::new("Acme", "Globex", true, true).unwrap();
    assert_eq!(
        edit::apply(p, &|s| r.replace(s), &|s| r.replace(s)).unwrap(),
        2
    );
    assert_eq!(
        std::fs::read_to_string(p.join("index.rsc")).unwrap(),
        "1:T6,Globex"
    );
    std::fs::write(
        p.join("metadata.html"),
        "https://www.acme.com/x http://acme.com https://acme.company.com",
    )
    .unwrap();
    edit::rehost(p, "https://acme.com", "https://globex.com").unwrap();
    assert_eq!(
        std::fs::read_to_string(p.join("metadata.html")).unwrap(),
        "https://globex.com/x https://globex.com https://acme.company.com"
    );
}
#[test]
fn localization_keeps_metadata() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("index.html");
    std::fs::write(&p,r#"<link rel="canonical" href="https://www.acme.com/"><link rel="stylesheet" href="https://acme.com/app.css"><style>a{background:url(https://acme.com/bg.png)}</style><img src="https://acme.com/logo.png"><a href="https://acme.com/about">About</a>"#).unwrap();
    edit::localize(tmp.path(), &Url::parse("https://acme.com").unwrap()).unwrap();
    let s = std::fs::read_to_string(p).unwrap();
    assert!(s.contains("href=\"https://www.acme.com/\""));
    assert!(s.contains("src=\"/logo.png\""));
    assert!(s.contains("href=\"/app.css\""));
    assert!(s.contains("url(/bg.png)"));
    assert!(s.contains("https://acme.com/about"));
}
#[test]
fn refs_cover_html_css_chunks_and_images() {
    let base = Url::parse("https://acme.com/").unwrap();
    let text = r#"<a href="/pricing">Pricing</a><a href="/essay.html">Essay</a><link href="/app.css"><img src="/logo.png" srcset="/logo2.png 2x, /logo3.png 3x"><div style="background:url(&quot;/bg.webp&quot;)"></div><img src="/_next/image?url=%2Fhero.png&w=640"><img src="https://cdn.test/img.png"><script>var a="/_next/static/chunks/942.js",b="vendor/rive/rive.wasm?v=1",c="/chunks/${id}.js",d="node_modules/foo/bar.js";</script>"#;
    let found = mirror::refs(text, &base);
    let paths: Vec<_> = found.iter().map(|u| u.path()).collect();
    assert_eq!(
        paths,
        vec![
            "/_next/static/chunks/942.js",
            "/app.css",
            "/bg.webp",
            "/hero.png",
            "/logo.png",
            "/logo2.png",
            "/logo3.png",
            "/vendor/rive/rive.wasm"
        ]
    );
    assert!(mirror::refs(r#"<img srcset="data:image/png;base64,abc 1x">"#, &base).is_empty());
}
#[test]
fn path_containment() {
    let tmp = tempfile::tempdir().unwrap();
    for bad in [
        "/..%2f..%2fevil",
        "/%2e%2e%2fevil",
        "/a/../evil",
        "/a%5c..%5cevil",
        "/%00",
    ] {
        assert!(util::contained(tmp.path(), bad).is_err(), "{bad}");
    }
    assert_eq!(
        mirror::output_path(
            tmp.path(),
            &Url::parse("https://acme.com/blog").unwrap(),
            true
        )
        .unwrap(),
        tmp.path().join("blog/index.html")
    );
}
#[cfg(unix)]
#[test]
fn symlinks_cannot_escape() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();
    assert!(util::contained(root.path(), "escape/secret").is_err());
}
#[test]
fn tracker_html_payload_and_runtime_loader() {
    assert_eq!(
        trackers::strip_html(
            r#"<script src="https://googletagmanager.com/gtag/js"></script><script src="/app.js"></script><noscript><img src="https://facebook.com/tr"></noscript><script type="application/ld+json">{}</script>"#
        ),
        r#"<script src="/app.js"></script><noscript></noscript><script type="application/ld+json">{}</script>"#
    );
    let row =
        "0:[\"$\",\"$L5\",null,{\"strategy\":\"afterInteractive\",\"children\":\"fbq('init')\"}]\n";
    assert_eq!(trackers::strip_row(row), "0:null\n");
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("app.js");
    std::fs::write(
        &p,
        "j.src='https://www.googletagmanager.com/gtm.js?id='+i;fetch('https://api.test/x')",
    )
    .unwrap();
    trackers::strip(tmp.path()).unwrap();
    assert_eq!(
        std::fs::read_to_string(p).unwrap(),
        "j.src='data:text/javascript,//www.googletagmanager.com/gtm.js?id='+i;fetch('https://api.test/x')"
    );
}
