//! Controlled responses exercise transport/contracts only. FAKE FOR TESTS ONLY:
//! none of these fixtures establishes Jev access, accuracy, calibration or billing.
use mochi::{
    export,
    interpret::{
        self, Answer, Failure, Limits, MODEL, Origin, PlanConfig, Provider, Question, Reply,
        Request, Response, Status, Usage,
    },
    project,
};
use serde_json::{Value, json};
use std::{cell::Cell, collections::BTreeMap, path::Path, time::Duration};
#[path = "support/source_fixture.rs"]
mod source_fixture;

#[derive(Clone, Copy)]
enum Behavior {
    Good,
    Unknown,
    Invalid,
    Retry,
    Auth,
    FailSecond,
    ChangeSource,
}
struct Controlled<'a> {
    calls: Cell<usize>,
    behavior: Behavior,
    root: &'a Path,
}
impl<'a> Controlled<'a> {
    fn new(root: &'a Path, behavior: Behavior) -> Self {
        Self {
            calls: Cell::new(0),
            behavior,
            root,
        }
    }
}
impl Provider for Controlled<'_> {
    fn origin(&self) -> Origin {
        Origin::ControlledFixture
    }
    fn ask(&self, request: &Request, _timeout: Duration) -> anyhow::Result<Reply> {
        let call = self.calls.get() + 1;
        self.calls.set(call);
        let journals = std::fs::read_dir(self.root.join("reconstruction/interpretation/runs"))?
            .map(|e| e.map(|e| e.path()))
            .collect::<std::io::Result<Vec<_>>>()?;
        assert!(
            journals.iter().any(|p| {
                let v: Value = serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap();
                v["attempts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|a| a["outcome"] == "reserved_before_dispatch")
            }),
            "charge reservation must be durable before provider invocation"
        );
        if matches!(self.behavior, Behavior::FailSecond) && call == 2 {
            anyhow::bail!("controlled transport failure");
        }
        if matches!(self.behavior, Behavior::Retry) && call == 1 {
            return Ok(Reply::Failure(Failure {
                status: 429,
                retryable: true,
                retry_after: Some(Duration::ZERO),
            }));
        }
        if matches!(self.behavior, Behavior::Auth) {
            return Ok(Reply::Failure(Failure {
                status: 401,
                retryable: false,
                retry_after: None,
            }));
        }
        if matches!(self.behavior, Behavior::ChangeSource) {
            std::fs::write(
                self.root.join("site/brand.svg"),
                b"changed during provider call",
            )?;
        }
        let mut answers = BTreeMap::new();
        for (id, question) in &request.questions {
            let answer = match question {
                Question::Noul { .. } => Answer::Noul { noul: 0.95 },
                Question::Choice { criteria, .. } => {
                    let choice = if matches!(self.behavior, Behavior::Invalid) {
                        "../../pwn"
                    } else if matches!(self.behavior, Behavior::Unknown) {
                        "unknown"
                    } else if criteria.contains_key("product_card") {
                        "product_card"
                    } else {
                        "label"
                    };
                    let probabilities = criteria
                        .keys()
                        .map(|option| (option.clone(), if option == choice { 1.0 } else { 0.0 }))
                        .collect();
                    Answer::Choice {
                        choice: choice.into(),
                        confidence: 1.0,
                        probabilities,
                    }
                }
            };
            answers.insert(id.clone(), answer);
        }
        Ok(Reply::Response(Response {
            model: MODEL.into(),
            answers,
            usage: Usage {
                input_tokens: 123,
                output_tokens: 20,
            },
        }))
    }
}
fn fixture(root: &Path) -> interpret::Plan {
    source_fixture::fixture(root, 2, 2);
    project::build(root).unwrap();
    interpret::save_plan(
        root,
        PlanConfig {
            batch_candidates: 1,
            ..PlanConfig::default()
        },
    )
    .unwrap()
}
fn limits() -> Limits {
    Limits {
        max_requests: 100,
        max_input_tokens: 100 * interpret::TOKEN_RESERVATION,
        max_cost_microusd: 1_000_000,
        input_price_microusd_per_million: 42_000,
        deadline_seconds: 30,
        max_retries: 0,
    }
}
fn bytes(root: &Path, path: &str) -> Vec<u8> {
    std::fs::read(root.join(path)).unwrap()
}
fn alter(root: &Path, path: &str, edit: impl FnOnce(&mut Value)) {
    let path = root.join(path);
    let mut value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    edit(&mut value);
    std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
}
#[test]
fn plans_are_deterministic_bounded_and_omit_controls_query_values_and_handlers() {
    let root = tempfile::tempdir().unwrap();
    source_fixture::fixture(root.path(), 2, 2);
    for route in 0..2 {
        for view in 0..2 {
            alter(
                root.path(),
                &format!("reconstruction/pages/{route}-{view}.json"),
                |v| {
                    v["nodes"][7]["attributes"][0]["value"] =
                        json!(format!("/product/{route}?access_token=PRIVATE_SENTINEL"));
                    let mut input = source_fixture::element(
                        Some(6),
                        "input",
                        &[
                            ("type", "text"),
                            ("value", "PRIVATE_SENTINEL"),
                            ("onfocus", "PRIVATE_SENTINEL"),
                        ],
                    );
                    input["control"] = json!({"value":"PRIVATE_SENTINEL","checked":false,"selected":null,"open":null});
                    v["nodes"].as_array_mut().unwrap().push(input);
                },
            );
        }
    }
    project::build(root.path()).unwrap();
    let plan = interpret::save_plan(root.path(), PlanConfig::default()).unwrap();
    assert!(!plan.entries.is_empty());
    let wire = serde_json::to_string(&plan).unwrap();
    assert!(!wire.contains("PRIVATE_SENTINEL"));
    for entry in &plan.entries {
        assert!(serde_json::to_vec(&entry.request).unwrap().len() <= 16 * 1024);
        assert_eq!(entry.request.model, MODEL);
        assert!(
            entry
                .request
                .questions
                .values()
                .any(|q| matches!(q, Question::Noul { .. }))
        );
        for question in entry.request.questions.values() {
            if let Question::Choice { criteria, .. } = question {
                assert!(criteria.contains_key("unknown") && criteria.len() <= 255);
            }
        }
    }
    let old = bytes(root.path(), "reconstruction/interpretation/plan.json");
    interpret::save_plan(root.path(), PlanConfig::default()).unwrap();
    assert_eq!(
        old,
        bytes(root.path(), "reconstruction/interpretation/plan.json")
    );
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_mochi"))
        .arg("plan-interpretation")
        .arg(root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        old,
        bytes(root.path(), "reconstruction/interpretation/plan.json")
    );
    assert!(
        !root
            .path()
            .join("reconstruction/interpretation/report.json")
            .exists()
    );
}
#[test]
fn valid_controlled_results_replay_without_calls_and_export_only_review_annotations() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("capture");
    std::fs::create_dir(&root).unwrap();
    let plan = fixture(&root);
    let controlled = Controlled::new(&root, Behavior::Good);
    let outcome = interpret::with_provider(&root, &controlled, limits()).unwrap();
    assert_eq!(controlled.calls.get(), plan.entries.len());
    assert_eq!(outcome.summary.attempts as usize, plan.entries.len());
    assert_eq!(outcome.summary.unknown_charge_attempts, 0);
    assert!(outcome.report.decisions.iter().all(|d| d.review_required));
    assert!(!outcome.report.auto_applied);
    assert_eq!(outcome.report.calibration, "unqualified");
    assert!(
        outcome
            .report
            .records
            .iter()
            .all(|r| r.origin == Origin::ControlledFixture)
    );
    assert!(
        outcome
            .report
            .decisions
            .iter()
            .any(|d| d.status == Status::ObservedAttribute)
    );
    let saved = bytes(&root, "reconstruction/interpretation/report.json");
    let replay = interpret::replay(&root).unwrap();
    assert_eq!(replay.summary.attempts, 0);
    assert_eq!(replay.summary.accounted_cost_microusd, 0);
    assert_eq!(replay.summary.cache_hits, plan.entries.len());
    assert_eq!(
        saved,
        bytes(&root, "reconstruction/interpretation/report.json")
    );
    let cli = std::process::Command::new(env!("CARGO_BIN_EXE_mochi"))
        .arg("interpret")
        .arg(&root)
        .arg("--replay")
        .env("TYPESAFE_API_KEY", "TEST_ONLY_NEVER_WRITE_THIS_KEY")
        .output()
        .unwrap();
    assert!(
        cli.status.success(),
        "{}",
        String::from_utf8_lossy(&cli.stderr)
    );
    for entry in walkdir::WalkDir::new(root.join("reconstruction/interpretation")) {
        let entry = entry.unwrap();
        if entry.file_type().is_file() {
            assert!(
                !String::from_utf8_lossy(&std::fs::read(entry.path()).unwrap())
                    .contains("TEST_ONLY_NEVER_WRITE_THIS_KEY")
            );
        }
    }
    let plain = parent.path().join("plain");
    let annotated = parent.path().join("annotated");
    export::tanstack_start(&root, &plain).unwrap();
    let report = export::tanstack_start_with_interpretation(&root, &annotated).unwrap();
    let annotation = report.interpretation.unwrap();
    assert!(!annotation.auto_applied);
    assert_eq!(annotation.provider_origins, vec![Origin::ControlledFixture]);
    let jsx = std::fs::read_dir(annotated.join("src/components"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|s| s == "tsx"))
        .map(|p| std::fs::read_to_string(p).unwrap())
        .collect::<String>();
    assert!(
        jsx.contains("review required")
            && jsx.contains("controlled_fixture")
            && jsx.contains("export function Component")
    );
    assert_eq!(
        bytes(&plain, "src/routes/catalog/index.tsx"),
        bytes(&annotated, "src/routes/catalog/index.tsx")
    );
    let cli_out = parent.path().join("cli");
    let cli = std::process::Command::new(env!("CARGO_BIN_EXE_mochi"))
        .arg("export")
        .arg(&root)
        .arg("--interpretation")
        .arg("-o")
        .arg(cli_out)
        .output()
        .unwrap();
    assert!(
        cli.status.success(),
        "{}",
        String::from_utf8_lossy(&cli.stderr)
    );
}
#[test]
fn malformed_results_stale_sources_and_corrupt_cache_preserve_last_complete_report() {
    let root = tempfile::tempdir().unwrap();
    let plan = fixture(root.path());
    interpret::with_provider(
        root.path(),
        &Controlled::new(root.path(), Behavior::Good),
        limits(),
    )
    .unwrap();
    let old = bytes(root.path(), "reconstruction/interpretation/report.json");
    let cache = root.path().join(format!(
        "reconstruction/interpretation/cache/{}.json",
        plan.entries[0].sha256
    ));
    alter(
        root.path(),
        cache.strip_prefix(root.path()).unwrap().to_str().unwrap(),
        |v| v["sha256"] = json!("corrupt"),
    );
    assert!(interpret::replay(root.path()).is_err());
    assert_eq!(
        old,
        bytes(root.path(), "reconstruction/interpretation/report.json")
    );
    std::fs::remove_file(&cache).unwrap();
    let invalid = Controlled::new(root.path(), Behavior::Invalid);
    assert!(interpret::with_provider(root.path(), &invalid, limits()).is_err());
    assert_eq!(invalid.calls.get(), 1);
    assert!(!cache.exists());
    assert_eq!(
        old,
        bytes(root.path(), "reconstruction/interpretation/report.json")
    );
    let changed = Controlled::new(root.path(), Behavior::ChangeSource);
    assert!(interpret::with_provider(root.path(), &changed, limits()).is_err());
    assert_eq!(
        old,
        bytes(root.path(), "reconstruction/interpretation/report.json")
    );
    assert!(interpret::prepare(root.path(), PlanConfig::default()).is_err());
}
#[test]
fn reservations_retries_auth_and_budget_preflight_fail_closed() {
    let root = tempfile::tempdir().unwrap();
    let plan = fixture(root.path());
    assert!(plan.entries.len() > 1);
    let mut small = limits();
    small.max_cost_microusd = 1;
    let provider = Controlled::new(root.path(), Behavior::Good);
    assert!(interpret::with_provider(root.path(), &provider, small).is_err());
    assert_eq!(provider.calls.get(), 0);
    assert!(interpret::replay(root.path()).is_err());
    let provider = Controlled::new(root.path(), Behavior::Auth);
    let mut bounded = limits();
    bounded.max_retries = 2;
    assert!(interpret::with_provider(root.path(), &provider, bounded).is_err());
    assert_eq!(provider.calls.get(), 1);
    let provider = Controlled::new(root.path(), Behavior::Retry);
    let mut bounded = limits();
    bounded.max_retries = 1;
    let outcome = interpret::with_provider(root.path(), &provider, bounded).unwrap();
    assert_eq!(provider.calls.get(), plan.entries.len() + 1);
    assert_eq!(outcome.summary.unknown_charge_attempts, 1);
    assert_eq!(
        outcome.summary.accounted_input_tokens,
        interpret::TOKEN_RESERVATION + 123 * plan.entries.len() as u64
    );
    assert_eq!(
        outcome.summary.accounted_cost_microusd,
        2753 + 6 * plan.entries.len() as u64
    );
    let journal: Value = serde_json::from_slice(&std::fs::read(&outcome.journal).unwrap()).unwrap();
    assert_eq!(journal["attempts"][0]["outcome"], "http_429_unknown_charge");
    assert_eq!(journal["status"], "complete");
}
#[test]
fn unknown_answers_abstain_and_partial_runs_are_resumable_without_publishing_partial_reports() {
    let root = tempfile::tempdir().unwrap();
    let plan = fixture(root.path());
    let bad = Controlled::new(root.path(), Behavior::FailSecond);
    assert!(interpret::with_provider(root.path(), &bad, limits()).is_err());
    assert_eq!(bad.calls.get(), 2);
    assert!(
        !root
            .path()
            .join("reconstruction/interpretation/report.json")
            .exists()
    );
    let unknown = Controlled::new(root.path(), Behavior::Unknown);
    let result = interpret::with_provider(root.path(), &unknown, limits()).unwrap();
    assert_eq!(unknown.calls.get(), plan.entries.len() - 1);
    assert_eq!(result.summary.cache_hits, 1);
    assert!(
        result
            .report
            .decisions
            .iter()
            .any(|d| d.status == Status::Abstained)
    );
    let cli = std::process::Command::new(env!("CARGO_BIN_EXE_mochi"))
        .arg("interpret")
        .arg(root.path())
        .arg("--live")
        .output()
        .unwrap();
    assert!(!cli.status.success());
    assert!(String::from_utf8_lossy(&cli.stderr).contains("--max-requests"));
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        let cli = std::process::Command::new(env!("CARGO_BIN_EXE_mochi"))
            .arg("interpret")
            .arg(root.path())
            .args([
                "--live",
                "--max-requests",
                "1",
                "--max-input-tokens",
                "65536",
                "--max-cost-usd",
                "0.01",
                "--input-price-per-million-usd",
                "0.042",
                "--deadline-seconds",
                "1",
            ])
            .env(
                "TYPESAFE_API_KEY",
                std::ffi::OsString::from_vec(b"PRIVATE_KEY_SENTINEL\xff".to_vec()),
            )
            .output()
            .unwrap();
        assert!(!cli.status.success());
        let error = String::from_utf8_lossy(&cli.stderr);
        assert!(error.contains("configure TYPESAFE_API_KEY"));
        assert!(!error.contains("PRIVATE_KEY_SENTINEL"));
    }
}
#[test]
fn response_validation_checks_distribution_types_ids_models_duplicates_and_money() {
    let request = Request {
        model: MODEL.into(),
        state: json!("FAKE FOR TESTS ONLY"),
        questions: BTreeMap::from([(
            "role".into(),
            Question::Choice {
                instructions: json!("Choose a role"),
                criteria: BTreeMap::from([
                    ("label".into(), "label".into()),
                    ("unknown".into(), "unknown".into()),
                ]),
            },
        )]),
    };
    let good = json!({"model":MODEL,"answers":{"role":{"type":"choice","choice":"label","confidence":0.9,"probabilities":{"label":0.9,"unknown":0.1}}},"usage":{"input_tokens":100,"output_tokens":20}});
    serde_json::from_value::<Response>(good.clone())
        .unwrap()
        .validate(&request)
        .unwrap();
    for case in 0..8 {
        let mut bad = good.clone();
        match case {
            0 => bad["model"] = json!("jev-latest"),
            1 => bad["answers"]["role"]["choice"] = json!("../bad"),
            2 => bad["answers"]["role"]["probabilities"]["label"] = json!(-0.1),
            3 => bad["answers"]["role"]["probabilities"]["unknown"] = json!(0.9),
            4 => bad["answers"]["role"]["confidence"] = json!(2.0),
            5 => {
                bad["answers"]["role"] = json!({"type":"noul","noul":0.9});
            }
            6 => bad["usage"]["input_tokens"] = json!(interpret::TOKEN_RESERVATION + 1),
            _ => bad["answers"]["extra"] = bad["answers"]["role"].clone(),
        }
        assert!(
            serde_json::from_value::<Response>(bad)
                .unwrap()
                .validate(&request)
                .is_err(),
            "case {case}"
        );
    }
    let duplicate = format!(
        r#"{{"model":"{MODEL}","answers":{{"role":{{"type":"noul","noul":0.9}},"role":{{"type":"noul","noul":0.1}}}},"usage":{{"input_tokens":1,"output_tokens":1}}}}"#
    );
    assert!(serde_json::from_str::<Response>(&duplicate).is_err());
    assert_eq!(interpret::usd_micros("0.042").unwrap(), 42_000);
    assert_eq!(interpret::usd_micros("0.000001").unwrap(), 1);
    for value in ["NaN", "-1", "1e3", "0.0000001", " 1", "1.2.3"] {
        assert!(interpret::usd_micros(value).is_err());
    }
}

#[test]
fn projection_preserves_node_positions_qualifies_resources_and_skips_oversized_candidates() {
    let root = tempfile::tempdir().unwrap();
    source_fixture::fixture(root.path(), 2, 2);
    for route in 0..2 {
        for view in 0..2 {
            alter(
                root.path(),
                &format!("reconstruction/pages/{route}-{view}.json"),
                |v| {
                    for index in [7, 11] {
                        v["nodes"][index]["tag"] = json!("audio");
                        v["nodes"][index]["attributes"][0]["name"] = json!("src");
                    }
                },
            );
        }
    }
    project::build(root.path()).unwrap();
    let plan = interpret::prepare(root.path(), PlanConfig::default()).unwrap();
    assert!(
        plan.observed_props
            .iter()
            .any(|(_, _, role)| role == "resource_url")
    );
    assert!(
        !plan
            .observed_props
            .iter()
            .any(|(_, _, role)| role == "destination")
    );
    for entry in &plan.entries {
        for candidate in entry.request.state["candidates"]
            .as_object()
            .unwrap()
            .values()
        {
            for example in candidate["examples"].as_array().unwrap() {
                for (index, node) in example["nodes"].as_array().unwrap().iter().enumerate() {
                    assert_eq!(node["node"], index);
                }
            }
        }
    }
    for route in 0..2 {
        for view in 0..2 {
            alter(
                root.path(),
                &format!("reconstruction/pages/{route}-{view}.json"),
                |v| {
                    for index in [8, 12] {
                        v["nodes"][index]["value"] = json!("Large text 🙂".repeat(4000));
                    }
                },
            );
        }
    }
    project::build(root.path()).unwrap();
    let plan = interpret::prepare(root.path(), PlanConfig::default()).unwrap();
    assert!(
        plan.skipped
            .iter()
            .any(|s| s.reason == "candidate_payload_limit")
    );
    assert!(
        plan.entries
            .iter()
            .all(|e| serde_json::to_vec(&e.request).unwrap().len() <= 16 * 1024)
    );
}

#[test]
fn lock_retry_budget_and_tampered_annotations_stop_before_additional_work() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("capture");
    std::fs::create_dir(&root).unwrap();
    let plan = fixture(&root);
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(root.join("reconstruction/interpretation/.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    assert!(
        interpret::replay(&root)
            .err()
            .unwrap()
            .to_string()
            .contains("lock")
    );
    assert!(interpret::save_plan(&root, PlanConfig::default()).is_err());
    drop(lock);
    let mut bounded = limits();
    bounded.max_requests = plan.entries.len() as u32;
    bounded.max_retries = 1;
    let retry = Controlled::new(&root, Behavior::Retry);
    assert!(interpret::with_provider(&root, &retry, bounded).is_err());
    assert_eq!(retry.calls.get(), plan.entries.len());
    assert!(
        !root
            .join("reconstruction/interpretation/report.json")
            .exists()
    );
    interpret::with_provider(&root, &Controlled::new(&root, Behavior::Good), limits()).unwrap();
    alter(&root, "reconstruction/interpretation/report.json", |v| {
        v["decisions"][0]["role"] = json!("../../untrusted");
    });
    let out = parent.path().join("refused");
    assert!(export::tanstack_start_with_interpretation(&root, &out).is_err());
    assert!(!out.exists());
    export::tanstack_start(&root, &parent.path().join("mechanical")).unwrap();
}
