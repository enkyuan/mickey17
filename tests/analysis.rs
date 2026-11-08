use mochi::analysis;
use serde_json::{Value, json};
use std::path::Path;

fn snapshot(title: &str) -> Value {
    json!({
        "schema_version":1,"url":format!("https://example.com/{title}"),
        "viewport":{"width":1440,"height":900},"truncated":false,
        "styles":[{"display":"block","color":"rgb(0, 0, 0)"}],
        "nodes":[
            {"parent":null,"tag":"body","attributes":{},"text":"","style":0},
            {"parent":0,"tag":"main","attributes":{},"text":"","style":0},
            {"parent":1,"tag":"article","attributes":{"class":title,"id":title},"text":"","style":0},
            {"parent":2,"tag":"h2","attributes":{},"text":title,"style":0},
            {"parent":2,"tag":"img","attributes":{"src":format!("/{title}.png"),"alt":title},"text":"","style":0},
            {"parent":2,"tag":"a","attributes":{"href":format!("/{title}/details")},"text":"Details","style":0}
        ]
    })
}

fn save(root: &Path, name: &str, snapshot: &Value) {
    let dir = root.join("analysis/pages");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(format!("{name}.json")),
        serde_json::to_vec(snapshot).unwrap(),
    )
    .unwrap();
}

#[test]
fn templates_ignore_content_but_preserve_style_semantics_and_child_order() {
    let root = tempfile::tempdir().unwrap();
    save(root.path(), "01", &snapshot("first"));
    save(root.path(), "02", &snapshot("second"));
    let mut semantic = snapshot("semantic");
    semantic["nodes"][5]["attributes"]["role"] = json!("button");
    save(root.path(), "03", &semantic);
    let mut style = snapshot("style");
    style["styles"][0]["color"] = json!("rgb(255, 0, 0)");
    save(root.path(), "04", &style);
    let mut ordered = snapshot("ordered");
    ordered["nodes"].as_array_mut().unwrap().swap(3, 4);
    save(root.path(), "05", &ordered);
    let mut viewport = snapshot("viewport");
    viewport["viewport"]["width"] = json!(375);
    save(root.path(), "06", &viewport);
    let report = analysis::analyze(root.path()).unwrap();
    assert_eq!(report.templates.len(), 5);
    assert_eq!(
        report.templates[0].pages,
        ["https://example.com/first", "https://example.com/second"]
    );
    let candidate = report
        .component_candidates
        .iter()
        .find(|candidate| candidate.tag == "article" && candidate.occurrences == 3)
        .unwrap();
    assert_eq!(candidate.subtree_nodes, 4);
    assert_eq!(candidate.pages, 3);
    assert_eq!(candidate.examples[0].node, 2);
    // Analysis never edits the captured evidence, and reruns produce identical output.
    let original = std::fs::read(root.path().join("analysis/pages/01.json")).unwrap();
    let first = std::fs::read(root.path().join("analysis/report.json")).unwrap();
    analysis::analyze(root.path()).unwrap();
    assert_eq!(
        first,
        std::fs::read(root.path().join("analysis/report.json")).unwrap()
    );
    assert_eq!(
        original,
        std::fs::read(root.path().join("analysis/pages/01.json")).unwrap()
    );
}

#[test]
fn truncated_snapshots_are_not_evidence_for_template_or_component_equality() {
    let root = tempfile::tempdir().unwrap();
    save(root.path(), "01", &snapshot("complete"));
    let mut incomplete = snapshot("incomplete");
    incomplete["truncated"] = json!(true);
    save(root.path(), "02", &incomplete);
    let report = analysis::analyze(root.path()).unwrap();
    assert_eq!(report.templates.len(), 1);
    assert_eq!(report.pages[1].status, "capture_truncated");
    assert!(report.pages[1].template.is_none());
    assert!(report.component_candidates.is_empty());
}

// Value: protects=invalid snapshot evidence cannot replace valid reports; fails_when=schema style root depth or text slots are accepted; why_new=prior test covered parent size and URL only; seam=none; no_claim=capture JavaScript limits
#[test]
fn invalid_graphs_and_oversized_inputs_do_not_replace_a_valid_report() {
    let root = tempfile::tempdir().unwrap();
    save(root.path(), "01", &snapshot("valid"));
    analysis::analyze(root.path()).unwrap();
    let original = std::fs::read(root.path().join("analysis/report.json")).unwrap();
    for case in 0..7 {
        let mut bad = snapshot("bad");
        match case {
            0 => bad["schema_version"] = json!(2),
            1 => bad["nodes"][1]["style"] = json!(99),
            2 => bad["nodes"][1]["parent"] = Value::Null,
            3 => bad["nodes"][2]["text_slots"] = json!([4]),
            4 => bad["nodes"][2]["text_slots"] = json!([1, 1]),
            5 => bad["nodes"][2]["text"] = json!("mixed without slots"),
            _ => {
                let nodes = bad["nodes"].as_array_mut().unwrap();
                nodes.clear();
                for i in 0..258 {
                    nodes.push(json!({"parent":if i==0 {Value::Null} else {json!(i-1)},"tag":"div","attributes":{},"text":"","style":0}));
                }
            }
        }
        save(root.path(), "02", &bad);
        assert!(analysis::analyze(root.path()).is_err(), "case {case}");
        assert_eq!(
            std::fs::read(root.path().join("analysis/report.json")).unwrap(),
            original
        );
    }
    let mut invalid = snapshot("invalid");
    invalid["nodes"][2]["parent"] = json!(2);
    save(root.path(), "02", &invalid);
    assert!(
        analysis::analyze(root.path())
            .unwrap_err()
            .to_string()
            .contains("parent")
    );
    assert_eq!(
        original,
        std::fs::read(root.path().join("analysis/report.json")).unwrap()
    );
    std::fs::write(
        root.path().join("analysis/pages/02.json"),
        vec![b' '; analysis::MAX_SNAPSHOT_BYTES as usize + 1],
    )
    .unwrap();
    assert!(
        analysis::analyze(root.path())
            .unwrap_err()
            .to_string()
            .contains("8 MiB")
    );
    let mut long_url = snapshot("long");
    long_url["url"] = json!(format!("https://example.com/{}", "x".repeat(8192)));
    save(root.path(), "02", &long_url);
    assert!(
        analysis::analyze(root.path())
            .unwrap_err()
            .to_string()
            .contains("URL")
    );
    assert_eq!(
        original,
        std::fs::read(root.path().join("analysis/report.json")).unwrap()
    );
}

#[cfg(unix)]
#[test]
fn analysis_refuses_symlinked_inputs_and_reports() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    save(root.path(), "01", &snapshot("safe"));
    std::fs::write(outside.path().join("report.json"), "keep").unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("report.json"),
        root.path().join("analysis/report.json"),
    )
    .unwrap();
    assert!(analysis::analyze(root.path()).is_err());
    assert_eq!(
        std::fs::read_to_string(outside.path().join("report.json")).unwrap(),
        "keep"
    );
}

#[test]
fn namespace_and_mixed_text_positions_are_structural_evidence() {
    let root = tempfile::tempdir().unwrap();
    let mut first = snapshot("first");
    first["nodes"][2]["text"] = json!("before after");
    first["nodes"][2]["text_slots"] = json!([0, 3]);
    save(root.path(), "01", &first);
    let mut moved = first.clone();
    moved["url"] = json!("https://example.com/moved");
    moved["nodes"][2]["text_slots"] = json!([3]);
    save(root.path(), "02", &moved);
    let mut svg = first.clone();
    svg["url"] = json!("https://example.com/svg");
    svg["nodes"][2]["namespace"] = json!("http://www.w3.org/2000/svg");
    save(root.path(), "03", &svg);
    assert_eq!(analysis::analyze(root.path()).unwrap().templates.len(), 3);
}

#[test]
fn total_node_budget_excludes_whole_pages_without_claiming_they_were_analyzed() {
    let root = tempfile::tempdir().unwrap();
    let mut large = snapshot("large");
    let leaf = json!({"parent":0,"tag":"span","attributes":{},"text":"","style":0});
    let mut nodes = vec![json!({"parent":null,"tag":"body","attributes":{},"text":"","style":0})];
    nodes.extend(std::iter::repeat_n(leaf, 19_999));
    large["nodes"] = json!(nodes);
    for index in 0..13 {
        large["url"] = json!(format!("https://example.com/{index}"));
        save(root.path(), &format!("{index:02}"), &large);
    }
    let report = analysis::analyze(root.path()).unwrap();
    assert_eq!(report.nodes_analyzed, 240_000);
    assert_eq!(report.templates.len(), 1);
    assert_eq!(report.templates[0].pages.len(), 12);
    assert_eq!(report.pages[12].status, "node_budget_exceeded");
    assert!(report.pages[12].template.is_none());
}

#[test]
fn analyzed_clone_rejects_long_start_urls_before_creating_output() {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("clone");
    let url = format!("http://127.0.0.1:1/{}", "x".repeat(8192));
    let error = mochi::crawl::clone_with_options(
        &url,
        &output,
        &mochi::crawl::Options {
            max_pages: 1,
            extract: false,
            keep_trackers: false,
            ignore_robots: false,
        },
        &mochi::crawl::PipelineOptions {
            analyze: true,
            jobs: 1,
            cache_dir: None,
        },
    )
    .err()
    .expect("oversized URL must be rejected");
    assert!(error.to_string().contains("URL"));
    assert!(!output.exists());
}

// Value: protects=candidate output is bounded and excludes trivial subtrees; fails_when=top100 example8 or minimum-size filters vanish; why_new=prior fixtures never reach caps; seam=none; no_claim=semantic component equivalence
#[test]
fn candidate_output_caps_examples_and_excludes_small_subtrees() {
    let root = tempfile::tempdir().unwrap();
    let mut page = snapshot("caps");
    let mut nodes = vec![json!({"parent":null,"tag":"body","attributes":{},"text":"","style":0})];
    for group in 0..110 {
        for _ in 0..10 {
            let parent = nodes.len();
            nodes.push(json!({"parent":0,"tag":"section","attributes":{"role":format!("group-{group}")},"text":"","style":0}));
            for _ in 0..2 {
                nodes.push(
                    json!({"parent":parent,"tag":"span","attributes":{},"text":"x","style":0}),
                );
            }
        }
    }
    page["nodes"] = json!(nodes);
    save(root.path(), "01", &page);
    let report = analysis::analyze(root.path()).unwrap();
    assert_eq!(report.component_candidates.len(), 100);
    for candidate in &report.component_candidates {
        assert_eq!(candidate.tag, "section");
        assert_eq!(candidate.subtree_nodes, 3);
        assert_eq!(candidate.occurrences, 10);
        assert_eq!(candidate.examples.len(), 8);
    }
    assert!(
        report
            .component_candidates
            .windows(2)
            .all(|p| p[0].fingerprint < p[1].fingerprint)
    );
}
