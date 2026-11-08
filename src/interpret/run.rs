use super::{
    MAX_FILE, MAX_RESPONSE, Plan, Record, Report, TOKEN_RESERVATION, current_model, decisions,
    plan,
    provider::{Provider, Reply, TypeSafe},
    read, save,
};
use crate::util;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub max_requests: u32,
    pub max_input_tokens: u64,
    pub max_cost_microusd: u64,
    pub input_price_microusd_per_million: u64,
    pub deadline_seconds: u64,
    pub max_retries: u8,
}
impl Limits {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=1024).contains(&self.max_requests)
                && self.max_input_tokens > 0
                && self.max_cost_microusd > 0
                && self.input_price_microusd_per_million > 0
                && (1..=600).contains(&self.deadline_seconds)
                && self.max_retries <= 2,
            "live interpretation requires positive request/token/cost/rate/deadline limits; retries must be 0..2"
        );
        Ok(())
    }
    fn cost(&self, tokens: u64) -> Result<u64> {
        let cost = u128::from(tokens)
            .checked_mul(u128::from(self.input_price_microusd_per_million))
            .context("cost overflow")?
            .checked_add(999_999)
            .context("cost overflow")?
            / 1_000_000;
        u64::try_from(cost).context("cost overflow")
    }
}
/// Exact decimal USD parsing. Money/rates use micro-USD; no floating point rounding.
pub fn usd_micros(value: &str) -> Result<u64> {
    ensure!(
        !value.is_empty() && value.len() <= 32 && value.trim() == value,
        "invalid decimal USD amount"
    );
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    ensure!(
        !whole.is_empty()
            && whole.bytes().all(|b| b.is_ascii_digit())
            && fraction.len() <= 6
            && fraction.bytes().all(|b| b.is_ascii_digit()),
        "USD amounts need nonnegative decimal notation with at most six fractional digits"
    );
    let whole = whole.parse::<u64>().context("USD amount overflow")?;
    let mut fraction = fraction.to_owned();
    while fraction.len() < 6 {
        fraction.push('0');
    }
    whole
        .checked_mul(1_000_000)
        .and_then(|w| fraction.parse::<u64>().ok().and_then(|f| w.checked_add(f)))
        .context("USD amount overflow")
}
#[derive(Debug, Default, Clone, Serialize)]
pub struct Summary {
    pub attempts: u32,
    pub cache_hits: usize,
    pub accounted_input_tokens: u64,
    pub accounted_cost_microusd: u64,
    pub reported_output_tokens: u64,
    pub unknown_charge_attempts: u32,
}
#[derive(Debug, Serialize)]
struct Attempt {
    request_sha256: String,
    outcome: String,
    reserved_input_tokens: u64,
    reserved_cost_microusd: u64,
    reported_usage: Option<super::Usage>,
}
#[derive(Serialize)]
struct Journal {
    schema_version: u32,
    plan_sha256: String,
    mode: String,
    status: String,
    limits: Option<Limits>,
    summary: Summary,
    attempts: Vec<Attempt>,
}
pub struct Outcome {
    pub report: Report,
    pub summary: Summary,
    pub journal: PathBuf,
}
pub(crate) struct ProjectLock(File);
impl Drop for ProjectLock {
    fn drop(&mut self) {
        // A subprocess spawned by another thread can briefly inherit the open
        // file description before exec. Closing only this handle would leave
        // flock held until that child closes its copy. Release at scope exit.
        let _ = self.0.unlock();
    }
}
pub(crate) fn lock(root: &Path) -> Result<ProjectLock> {
    let path = util::checked_path(root, &root.join("reconstruction/interpretation/.lock"))?;
    std::fs::create_dir_all(path.parent().context("missing lock parent")?)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    file.try_lock()
        .map_err(|_| anyhow::anyhow!("another interpretation operation holds the project lock"))?;
    Ok(ProjectLock(file))
}
fn cache_path(entry: &plan::Entry) -> PathBuf {
    PathBuf::from(format!(
        "reconstruction/interpretation/cache/{}.json",
        entry.sha256
    ))
}
fn cached(root: &Path, entry: &plan::Entry) -> Result<Option<Record>> {
    let relative = cache_path(entry);
    let path = util::checked_path(root, &root.join(&relative))?;
    match path.symlink_metadata() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
        Ok(_) => {
            let bytes = read(root, &relative, MAX_RESPONSE)?;
            let record: Record = serde_json::from_slice(&bytes)
                .map_err(|_| anyhow::anyhow!("invalid interpretation cache JSON/schema"))?;
            record.validate(entry)?;
            Ok(Some(record))
        }
    }
}
fn reserve(journal: &mut Journal, entry: &plan::Entry, limits: &Limits) -> Result<()> {
    let cost = limits.cost(TOKEN_RESERVATION)?;
    let tokens = journal
        .summary
        .accounted_input_tokens
        .checked_add(TOKEN_RESERVATION)
        .context("token accounting overflow")?;
    let total_cost = journal
        .summary
        .accounted_cost_microusd
        .checked_add(cost)
        .context("cost accounting overflow")?;
    ensure!(
        journal.summary.attempts < limits.max_requests
            && tokens <= limits.max_input_tokens
            && total_cost <= limits.max_cost_microusd,
        "interpretation attempt exceeds remaining request/token/cost reservation budget"
    );
    journal.summary.attempts += 1;
    journal.summary.accounted_input_tokens = tokens;
    journal.summary.accounted_cost_microusd = total_cost;
    journal.summary.unknown_charge_attempts += 1;
    journal.attempts.push(Attempt {
        request_sha256: entry.sha256.clone(),
        outcome: "reserved_before_dispatch".into(),
        reserved_input_tokens: TOKEN_RESERVATION,
        reserved_cost_microusd: cost,
        reported_usage: None,
    });
    Ok(())
}
fn execute(
    root: &Path,
    provider: Option<&dyn Provider>,
    limits: Option<Limits>,
) -> Result<Outcome> {
    if let Some(limits) = &limits {
        limits.validate()?;
    }
    let root = root.canonicalize()?;
    let _lock = lock(&root)?;
    let model = current_model(&root)?;
    let bytes = read(
        &root,
        Path::new("reconstruction/interpretation/plan.json"),
        MAX_FILE,
    )
    .context("run plan-interpretation first")?;
    let plan: Plan = serde_json::from_slice(&bytes).context("invalid interpretation plan JSON")?;
    plan.validate(&model)?;
    let cache: Vec<_> = plan
        .entries
        .iter()
        .map(|entry| cached(&root, entry))
        .collect::<Result<_>>()?;
    let missing = cache.iter().filter(|entry| entry.is_none()).count();
    if let Some(limits) = &limits {
        let attempts = u32::try_from(missing)?;
        let tokens = u64::from(attempts)
            .checked_mul(TOKEN_RESERVATION)
            .context("preflight token overflow")?;
        let cost = limits
            .cost(TOKEN_RESERVATION)?
            .checked_mul(u64::from(attempts))
            .context("preflight cost overflow")?;
        ensure!(
            attempts <= limits.max_requests
                && tokens <= limits.max_input_tokens
                && cost <= limits.max_cost_microusd,
            "budget cannot reserve one worst-case attempt for every uncached request; reduce the plan or increase explicit limits"
        );
    } else {
        ensure!(
            missing == 0,
            "interpretation replay cache is incomplete; no network request was made"
        );
    }
    let directory = util::checked_path(&root, &root.join("reconstruction/interpretation/runs"))?;
    std::fs::create_dir_all(&directory)?;
    let file = tempfile::Builder::new()
        .prefix("run-")
        .suffix(".json")
        .tempfile_in(directory)?;
    let (file, path) = file.keep()?;
    drop(file);
    let relative = path.strip_prefix(&root)?.to_path_buf();
    let mut journal = Journal {
        schema_version: 1,
        plan_sha256: plan.sha256.clone(),
        mode: if provider.is_some() {
            "provider"
        } else {
            "replay"
        }
        .into(),
        status: "running".into(),
        limits: limits.clone(),
        summary: Summary::default(),
        attempts: Vec::new(),
    };
    save(&root, &relative, &journal, true)?;
    let deadline = Instant::now()
        + Duration::from_secs(limits.as_ref().map_or(60, |limits| limits.deadline_seconds));
    let result = (|| -> Result<Report> {
        let mut records = Vec::new();
        for (entry, cached) in plan.entries.iter().zip(cache) {
            ensure!(
                Instant::now() < deadline,
                "interpretation network/replay deadline exceeded"
            );
            if let Some(record) = cached {
                journal.summary.cache_hits += 1;
                records.push(record);
                continue;
            }
            let provider = provider.context("replay cannot call a provider")?;
            let limits = limits
                .as_ref()
                .context("provider calls require live limits")?;
            let mut completed = None;
            for retry in 0..=limits.max_retries {
                ensure!(
                    Instant::now() < deadline,
                    "interpretation network deadline exceeded"
                );
                reserve(&mut journal, entry, limits)?;
                // A crash/timeout after this durable reservation retains the
                // full possible charge; it never gets assumed free on retry.
                save(&root, &relative, &journal, true)?;
                let reply = provider.ask(
                    &entry.request,
                    deadline.saturating_duration_since(Instant::now()),
                );
                match reply {
                    Err(_) => {
                        journal
                            .attempts
                            .last_mut()
                            .context("missing reserved attempt")?
                            .outcome = "transport_failure_unknown_charge".into();
                        anyhow::bail!("provider request failed; full reservation retained");
                    }
                    Ok(Reply::Response(response)) => {
                        let attempt = journal
                            .attempts
                            .last_mut()
                            .context("missing reserved attempt")?;
                        attempt.reported_usage = Some(response.usage);
                        if let Err(error) = response.validate(&entry.request) {
                            attempt.outcome = "invalid_response_or_accounting_contract".into();
                            return Err(error);
                        }
                        attempt.outcome = "validated_reported_usage".into();
                        journal.summary.accounted_input_tokens -= TOKEN_RESERVATION;
                        journal.summary.accounted_input_tokens += response.usage.input_tokens;
                        journal.summary.accounted_cost_microusd -= attempt.reserved_cost_microusd;
                        journal.summary.accounted_cost_microusd +=
                            limits.cost(response.usage.input_tokens)?;
                        journal.summary.reported_output_tokens += response.usage.output_tokens;
                        journal.summary.unknown_charge_attempts -= 1;
                        save(&root, &relative, &journal, true)?;
                        let record =
                            Record::new(entry.sha256.clone(), provider.origin(), response)?;
                        save(&root, &cache_path(entry), &record, false)?;
                        completed = Some(record);
                        break;
                    }
                    Ok(Reply::Failure(failure)) => {
                        journal
                            .attempts
                            .last_mut()
                            .context("missing reserved attempt")?
                            .outcome = format!("http_{}_unknown_charge", failure.status);
                        save(&root, &relative, &journal, true)?;
                        ensure!(
                            failure.retryable && retry < limits.max_retries,
                            "provider HTTP failure; retry stopped and reservation retained"
                        );
                        let backoff = Duration::from_millis(250 * (1u64 << retry));
                        let delay = failure.retry_after.unwrap_or(backoff).max(backoff);
                        ensure!(
                            delay <= Duration::from_secs(30)
                                && delay < deadline.saturating_duration_since(Instant::now()),
                            "retry delay exceeds the remaining deadline; retry later"
                        );
                        std::thread::sleep(delay);
                    }
                }
            }
            records.push(completed.context("provider did not complete request")?);
        }
        ensure!(
            Instant::now() < deadline,
            "interpretation network/replay deadline exceeded"
        );
        ensure!(
            read(
                &root,
                Path::new("reconstruction/interpretation/plan.json"),
                MAX_FILE
            )? == bytes,
            "request plan changed during interpretation"
        );
        ensure!(
            current_model(&root)?.input_sha256 == model.input_sha256,
            "source changed during interpretation"
        );
        let report = Report {
            schema_version: 1,
            decisions: decisions(&plan, &records, &model)?,
            plan: plan.clone(),
            records,
            calibration: "unqualified".into(),
            auto_applied: false,
        };
        report.validate(&model)?;
        save(
            &root,
            Path::new("reconstruction/interpretation/report.json"),
            &report,
            true,
        )?;
        Ok(report)
    })();
    journal.status = if result.is_ok() { "complete" } else { "failed" }.into();
    save(&root, &relative, &journal, true)?;
    match result {
        Ok(report) => Ok(Outcome {
            report,
            summary: journal.summary,
            journal: path,
        }),
        Err(error) => Err(error.context(format!(
            "interpretation failed; previous complete report preserved; journal: {}",
            path.display()
        ))),
    }
}
/// Live transport is an explicit operation with caller-supplied positive limits.
pub fn interpret_live(root: &Path, client: &TypeSafe, limits: Limits) -> Result<Outcome> {
    execute(root, Some(client), Some(limits))
}
/// Replay validated cache records without credentials or network calls.
pub fn replay(root: &Path) -> Result<Outcome> {
    execute(root, None, None)
}
/// Controlled providers support deterministic integration tests. Their origin is
/// recorded and must not be represented as live-provider qualification.
pub fn with_provider(root: &Path, provider: &dyn Provider, limits: Limits) -> Result<Outcome> {
    execute(root, Some(provider), Some(limits))
}

#[cfg(all(test, unix))]
mod lock_tests {
    #[test]
    fn completed_operation_releases_lock_even_with_a_cloned_handle() {
        let root = tempfile::tempdir().unwrap();
        let held = super::lock(root.path()).unwrap();
        let inherited = held.0.try_clone().unwrap();
        assert!(super::lock(root.path()).is_err());
        drop(held);
        let next = super::lock(root.path()).unwrap();
        assert!(super::lock(root.path()).is_err());
        drop(inherited);
        assert!(super::lock(root.path()).is_err());
        drop(next);
        assert!(super::lock(root.path()).is_ok());
    }
}
