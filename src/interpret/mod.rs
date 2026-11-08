//! Typed semantic proposals. All proposals require review; interpretation cannot
//! create source strings, alter structural proofs, or certify reconstruction.
mod plan;
mod provider;
mod run;
use crate::{
    cache::fingerprint,
    project::{Project, SourceRef},
    util,
};
use anyhow::{Context, Result, ensure};
pub use plan::{Plan, PlanConfig, prepare, save_plan};
pub use provider::{Failure, Origin, Provider, Reply, TypeSafe};
pub use run::{Limits, Outcome, interpret_live, replay, usd_micros, with_provider};
use serde::{
    Deserialize, Serialize,
    de::{self, MapAccess, Visitor},
};
use std::{
    collections::BTreeMap,
    fmt,
    io::{Read, Write},
    marker::PhantomData,
    path::Path,
};

pub const MODEL: &str = "jev-1.14.0";
pub const VERSION: &str = "mochi-semantic-questions-v1";
pub const TOKEN_RESERVATION: u64 = 65_536;
const MAX_FILE: u64 = 16 * 1024 * 1024;
const MAX_RESPONSE: u64 = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub model: String,
    pub state: serde_json::Value,
    pub questions: BTreeMap<String, Question>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum Question {
    Choice {
        instructions: serde_json::Value,
        criteria: BTreeMap<String, String>,
    },
    Noul {
        instructions: serde_json::Value,
        criteria: TruthCriteria,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TruthCriteria {
    #[serde(rename = "true")]
    pub yes: String,
    #[serde(rename = "false")]
    pub no: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub model: String,
    #[serde(deserialize_with = "unique_map")]
    pub answers: BTreeMap<String, Answer>,
    pub usage: Usage,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum Answer {
    Choice {
        choice: String,
        confidence: f64,
        #[serde(deserialize_with = "unique_map")]
        probabilities: BTreeMap<String, f64>,
    },
    Noul {
        noul: f64,
    },
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}
fn unique_map<'de, D, T>(deserializer: D) -> std::result::Result<BTreeMap<String, T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Unique<T>(PhantomData<T>);
    impl<'de, T: Deserialize<'de>> Visitor<'de> for Unique<T> {
        type Value = BTreeMap<String, T>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a map with unique keys")
        }
        fn visit_map<M: MapAccess<'de>>(
            self,
            mut access: M,
        ) -> std::result::Result<Self::Value, M::Error> {
            let mut map = BTreeMap::new();
            while let Some((key, value)) = access.next_entry::<String, T>()? {
                if map.insert(key, value).is_some() {
                    return Err(de::Error::custom("duplicate map key"));
                }
            }
            Ok(map)
        }
    }
    deserializer.deserialize_map(Unique(PhantomData))
}
fn hash<T: Serialize>(value: &T) -> Result<String> {
    Ok(fingerprint(&serde_json::to_vec(value)?))
}
fn probability(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}
impl Response {
    pub fn validate(&self, request: &Request) -> Result<()> {
        ensure!(
            request.model == MODEL && self.model == MODEL,
            "model version mismatch"
        );
        ensure!(
            self.usage.input_tokens <= TOKEN_RESERVATION && self.usage.output_tokens <= 1_000_000,
            "provider usage exceeds the reserved contract"
        );
        ensure!(
            self.answers.keys().eq(request.questions.keys()),
            "response question IDs do not match request"
        );
        for (id, question) in &request.questions {
            match (question, &self.answers[id]) {
                (Question::Noul { .. }, Answer::Noul { noul }) => {
                    ensure!(probability(*noul), "invalid Noul value")
                }
                (
                    Question::Choice { criteria, .. },
                    Answer::Choice {
                        choice,
                        confidence,
                        probabilities,
                    },
                ) => {
                    ensure!(
                        probability(*confidence)
                            && criteria.contains_key(choice)
                            && probabilities.keys().eq(criteria.keys()),
                        "invalid Choice shape"
                    );
                    ensure!(
                        probabilities.values().all(|p| probability(*p)),
                        "invalid Choice probabilities"
                    );
                    let sum: f64 = probabilities.values().sum();
                    ensure!(
                        (sum - 1.0).abs() <= 0.000001,
                        "Choice probabilities do not sum to one"
                    );
                    let selected = probabilities[choice];
                    ensure!(
                        probabilities.values().all(|p| *p <= selected + 1e-9),
                        "Choice is not a maximum-probability option"
                    );
                }
                _ => anyhow::bail!("response answer type does not match question"),
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub schema_version: u32,
    pub request_sha256: String,
    pub origin: Origin,
    pub response: Response,
    pub sha256: String,
}
impl Record {
    fn new(request_sha256: String, origin: Origin, response: Response) -> Result<Self> {
        let sha256 = hash(&(&request_sha256, &origin, &response))?;
        Ok(Self {
            schema_version: 1,
            request_sha256,
            origin,
            response,
            sha256,
        })
    }
    fn validate(&self, entry: &plan::Entry) -> Result<()> {
        ensure!(
            self.schema_version == 1 && self.request_sha256 == entry.sha256,
            "cache/request identity mismatch"
        );
        ensure!(
            self.sha256 == hash(&(&self.request_sha256, &self.origin, &self.response))?,
            "cache content digest mismatch"
        );
        self.response.validate(&entry.request)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    ProposalNeedsReview,
    Abstained,
    ObservedAttribute,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    pub component: String,
    pub prop: Option<String>,
    pub source: SourceRef,
    pub role: String,
    pub status: Status,
    pub reason: String,
    pub probability: Option<f64>,
    pub confidence: Option<f64>,
    pub applicability: Option<f64>,
    pub review_required: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub schema_version: u32,
    pub plan: Plan,
    pub records: Vec<Record>,
    pub decisions: Vec<Decision>,
    pub calibration: String,
    pub auto_applied: bool,
}
impl Report {
    pub fn validate(&self, model: &Project) -> Result<()> {
        ensure!(
            self.schema_version == 1 && self.calibration == "unqualified" && !self.auto_applied,
            "unsupported interpretation policy"
        );
        self.plan.validate(model)?;
        ensure!(
            self.records.len() == self.plan.entries.len(),
            "incomplete interpretation response set"
        );
        for (record, entry) in self.records.iter().zip(&self.plan.entries) {
            record.validate(entry)?;
        }
        ensure!(
            hash(&self.decisions)? == hash(&decisions(&self.plan, &self.records, model)?)?,
            "invalid interpretation decisions"
        );
        Ok(())
    }
}
fn decisions(plan: &Plan, records: &[Record], model: &Project) -> Result<Vec<Decision>> {
    let mut result = Vec::new();
    for (entry, record) in plan.entries.iter().zip(records) {
        for (id, target) in &entry.targets {
            if target.kind == plan::Kind::Applicability {
                continue;
            }
            let Answer::Choice {
                choice,
                confidence,
                probabilities,
            } = &record.response.answers[id]
            else {
                anyhow::bail!("expected a Choice answer");
            };
            let applicability = entry
                .targets
                .iter()
                .find(|(_, t)| {
                    t.component == target.component && t.kind == plan::Kind::Applicability
                })
                .and_then(|(id, _)| match record.response.answers[id] {
                    Answer::Noul { noul } => Some(noul),
                    _ => None,
                })
                .context("missing applicability answer")?;
            let abstained = choice == "unknown" || applicability <= 0.5;
            result.push(Decision {
                component: target.component.clone(),
                prop: target.prop.clone(),
                source: model.components[&target.component].prototype.clone(),
                role: if abstained {
                    "unknown".into()
                } else {
                    choice.clone()
                },
                status: if abstained {
                    Status::Abstained
                } else {
                    Status::ProposalNeedsReview
                },
                reason: if choice == "unknown" {
                    "unknown_option"
                } else if applicability <= 0.5 {
                    "applicability_does_not_favor_yes"
                } else {
                    "uncalibrated_requires_review"
                }
                .into(),
                probability: Some(probabilities[choice]),
                confidence: Some(*confidence),
                applicability: Some(applicability),
                review_required: true,
            });
        }
    }
    for (component, prop, role) in &plan.observed_props {
        result.push(Decision {
            component: component.clone(),
            prop: Some(prop.clone()),
            source: model.components[component].prototype.clone(),
            role: role.clone(),
            status: Status::ObservedAttribute,
            reason: "observed_attribute_kind".into(),
            probability: None,
            confidence: None,
            applicability: None,
            review_required: true,
        });
    }
    Ok(result)
}
fn read(root: &Path, relative: &Path, limit: u64) -> Result<Vec<u8>> {
    let path = util::checked_path(root, &root.join(relative))?;
    let file = std::fs::File::open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= limit,
        "interpretation file byte limit exceeded or not regular"
    );
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "interpretation file byte limit exceeded"
    );
    Ok(bytes)
}
fn save<T: Serialize>(root: &Path, relative: &Path, value: &T, replace: bool) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    ensure!(
        bytes.len() as u64 <= MAX_FILE,
        "interpretation output byte limit exceeded"
    );
    let path = util::checked_path(root, &root.join(relative))?;
    std::fs::create_dir_all(path.parent().context("missing interpretation parent")?)?;
    if replace && path.exists() {
        util::atomic_replace(&path, &bytes)?;
    } else {
        let mut file = tempfile::NamedTempFile::new_in(
            path.parent().context("missing interpretation parent")?,
        )?;
        file.write_all(&bytes)?;
        file.as_file().sync_all()?;
        file.persist_noclobber(&path)?;
    }
    Ok(())
}
pub(crate) fn current_model(root: &Path) -> Result<Project> {
    let bytes = read(
        root,
        Path::new("reconstruction/project.json"),
        128 * 1024 * 1024,
    )?;
    let model: Project = serde_json::from_slice(&bytes).context("invalid saved project model")?;
    model.validate()?;
    ensure!(
        crate::project::derive(root)?.input_sha256 == model.input_sha256,
        "saved project model is stale; rerun model-project"
    );
    Ok(model)
}
/// Read a saved report without network calls; validate it against current model evidence.
pub fn load_report(root: &Path, model: &Project) -> Result<Report> {
    let root = root.canonicalize()?;
    let bytes = read(
        &root,
        Path::new("reconstruction/interpretation/report.json"),
        MAX_FILE,
    )?;
    let report: Report =
        serde_json::from_slice(&bytes).context("invalid saved interpretation report")?;
    report.validate(model)?;
    Ok(report)
}

#[derive(Debug, Serialize)]
pub struct Annotations {
    pub report_sha256: String,
    pub model: String,
    pub calibration: String,
    pub auto_applied: bool,
    pub provider_origins: Vec<Origin>,
    pub components: BTreeMap<String, Vec<Decision>>,
}
/// Explicit source annotations only. Labels remain review-required and never
/// become executable strings, filenames, routes, or changed primitive mappings.
pub fn annotations(root: &Path, model: &Project) -> Result<Annotations> {
    let report = load_report(root, model)?;
    let report_sha256 = hash(&report)?;
    let provider_origins = report
        .records
        .iter()
        .map(|r| r.origin)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut components: BTreeMap<String, Vec<Decision>> = BTreeMap::new();
    for decision in report.decisions {
        components
            .entry(model.components[&decision.component].name.clone())
            .or_default()
            .push(decision);
    }
    Ok(Annotations {
        report_sha256,
        model: MODEL.into(),
        calibration: "unqualified".into(),
        auto_applied: false,
        provider_origins,
        components,
    })
}
