use axum::http::StatusCode;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct RunRequest {
    pub(super) stage: Option<String>,
    pub(super) task_id: Option<String>,
    pub(super) max_new_runs: Option<u32>,
    pub(super) replicate: Option<u32>,
    pub(super) parallelism: Option<u16>,
    pub(super) override_stage_gate: Option<bool>,
    pub(super) rerun_invalid: Option<bool>,
    pub(super) dry_run: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct HarnessConfiguration {
    #[serde(default = "default_runner")]
    pub(super) runner: String,
    #[serde(default = "default_review_root")]
    pub(super) review_root: String,
    #[serde(default)]
    pub(super) supports_parallelism: bool,
    #[serde(default = "default_parallelism")]
    pub(super) max_parallelism: u16,
    #[serde(default)]
    pub(super) arguments: RunnerArguments,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct RunnerArguments {
    pub(super) stage: Option<String>,
    pub(super) task: Option<String>,
    pub(super) max_new_runs: Option<String>,
    pub(super) replicate: Option<String>,
    pub(super) parallelism: Option<String>,
    pub(super) override_stage_gate: Option<String>,
    pub(super) rerun_invalid: Option<String>,
    pub(super) dry_run: Option<String>,
}

impl Default for HarnessConfiguration {
    fn default() -> Self {
        Self {
            runner: default_runner(),
            review_root: default_review_root(),
            supports_parallelism: false,
            max_parallelism: 1,
            arguments: RunnerArguments::default(),
        }
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct BenchmarkSummary {
    pub(super) id: String,
    pub(super) schema_version: Option<u64>,
    pub(super) path: String,
    pub(super) stages: Vec<String>,
    pub(super) tasks: Vec<String>,
    pub(super) runner_available: bool,
    pub(super) review_available: bool,
    pub(super) supports_parallelism: bool,
    pub(super) max_parallelism: u16,
    pub(super) supported_controls: Vec<&'static str>,
    pub(super) run_count: usize,
    pub(super) valid_run_count: usize,
    pub(super) input_tokens: u64,
    pub(super) output_tokens: u64,
    pub(super) duration_ms: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct ApiError {
    pub(super) status: StatusCode,
    pub(super) message: String,
}

impl ApiError {
    pub(super) fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    pub(super) fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    pub(super) fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
        }
    }

    pub(super) fn internal(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
        }
    }
}

pub(super) fn resolve_benchmark_root(root: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    let candidate = match root {
        Some(root) => root,
        None => {
            let cwd = std::env::current_dir()?;
            let child = cwd.join("benchmarks");
            if child.is_dir() { child } else { cwd }
        }
    };
    let canonical = dunce::canonicalize(&candidate).map_err(|error| {
        anyhow::anyhow!(
            "benchmark root does not exist: {}: {error}",
            candidate.display()
        )
    })?;
    anyhow::ensure!(
        canonical.is_dir(),
        "benchmark root is not a directory: {}",
        canonical.display()
    );
    Ok(canonical)
}

pub(super) fn discover_benchmarks(root: &Path) -> anyhow::Result<Vec<BenchmarkSummary>> {
    let mut benchmarks = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() || !entry.path().join("benchmark.json").is_file() {
            continue;
        }
        let (configuration, runs) = read_benchmark(&entry.path())?;
        let harness = read_harness(&entry.path(), &configuration)?;
        benchmarks.push(summarize_benchmark(
            &entry.path(),
            &configuration,
            &harness,
            &runs,
        ));
    }
    benchmarks.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(benchmarks)
}

pub(super) fn read_benchmark(directory: &Path) -> anyhow::Result<(Value, Vec<Value>)> {
    let configuration: Value =
        serde_json::from_slice(&std::fs::read(directory.join("benchmark.json"))?)?;
    let scores_path = directory.join("results").join("scores.json");
    let runs = if scores_path.is_file() {
        serde_json::from_slice::<Vec<Value>>(&std::fs::read(scores_path)?)?
    } else {
        Vec::new()
    };
    Ok((configuration, runs))
}

pub(super) fn read_harness(
    directory: &Path,
    configuration: &Value,
) -> anyhow::Result<HarnessConfiguration> {
    let path = directory.join("benchmark-harness.json");
    let harness = if path.is_file() {
        serde_json::from_slice(&std::fs::read(path)?)?
    } else if let Some(value) = configuration.get("harness") {
        serde_json::from_value(value.clone())?
    } else {
        HarnessConfiguration::default()
    };
    validate_relative_component(&harness.runner, "runner")?;
    validate_relative_component(&harness.review_root, "review root")?;
    for argument in harness.arguments.values().into_iter().flatten() {
        anyhow::ensure!(
            !argument.is_empty()
                && argument
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric()),
            "benchmark runner argument names must be ASCII alphanumeric"
        );
    }
    Ok(harness)
}

pub(super) fn summarize_benchmark(
    directory: &Path,
    configuration: &Value,
    harness: &HarnessConfiguration,
    runs: &[Value],
) -> BenchmarkSummary {
    let id = configuration
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| directory.file_name()?.to_str().map(str::to_owned))
        .unwrap_or_else(|| "benchmark".to_string());
    BenchmarkSummary {
        id,
        schema_version: configuration.get("schemaVersion").and_then(Value::as_u64),
        path: directory.display().to_string(),
        stages: string_ids(configuration.get("stages")),
        tasks: task_ids(configuration),
        runner_available: directory.join(&harness.runner).is_file(),
        review_available: directory
            .join(&harness.review_root)
            .join("index.html")
            .is_file(),
        supports_parallelism: harness.supports_parallelism,
        max_parallelism: harness.max_parallelism.max(1),
        supported_controls: harness.arguments.supported_controls(),
        run_count: runs.len(),
        valid_run_count: runs
            .iter()
            .filter(|run| run.get("valid").and_then(Value::as_bool) == Some(true))
            .count(),
        input_tokens: sum_u64(runs, "inputTokens"),
        output_tokens: sum_u64(runs, "outputTokens"),
        duration_ms: sum_u64(runs, "durationMs"),
    }
}

pub(super) fn validate_run_request(
    configuration: &Value,
    harness: &HarnessConfiguration,
    request: &RunRequest,
) -> Result<(), ApiError> {
    if let Some(stage) = request.stage.as_deref() {
        require_control(harness.arguments.stage.as_deref(), "stage")?;
        let stages = string_ids(configuration.get("stages"));
        if !stages.is_empty() && !stages.iter().any(|candidate| candidate == stage) {
            return Err(ApiError::bad_request(format!("unknown stage: {stage}")));
        }
    }
    if let Some(task_id) = request.task_id.as_deref() {
        require_control(harness.arguments.task.as_deref(), "task")?;
        let tasks = task_ids(configuration);
        if !tasks.iter().any(|candidate| candidate == task_id) {
            return Err(ApiError::bad_request(format!("unknown task: {task_id}")));
        }
    }
    if let Some(replicate) = request.replicate {
        require_control(harness.arguments.replicate.as_deref(), "replicate")?;
        if replicate == 0 {
            return Err(ApiError::bad_request("replicate must be at least 1"));
        }
    }
    if request.max_new_runs.is_some() {
        require_control(harness.arguments.max_new_runs.as_deref(), "new-run cap")?;
    }
    if request.override_stage_gate == Some(true) {
        require_control(
            harness.arguments.override_stage_gate.as_deref(),
            "stage override",
        )?;
    }
    if request.rerun_invalid == Some(true) {
        require_control(
            harness.arguments.rerun_invalid.as_deref(),
            "invalid-run retry",
        )?;
    }
    if request.dry_run == Some(true) {
        require_control(harness.arguments.dry_run.as_deref(), "dry run")?;
    }
    if let Some(parallelism) = request.parallelism {
        require_control(harness.arguments.parallelism.as_deref(), "parallelism")?;
        if !harness.supports_parallelism && parallelism > 1 {
            return Err(ApiError::bad_request(
                "this benchmark runner does not declare parallel execution support",
            ));
        }
        let maximum = harness.max_parallelism.max(1);
        if parallelism == 0 || parallelism > maximum {
            return Err(ApiError::bad_request(format!(
                "parallelism must be between 1 and {maximum}"
            )));
        }
    }
    Ok(())
}

pub(super) fn benchmark_directory(root: &Path, benchmark_id: &str) -> Result<PathBuf, ApiError> {
    if benchmark_id.is_empty()
        || benchmark_id == "."
        || benchmark_id == ".."
        || benchmark_id.contains('/')
        || benchmark_id.contains('\\')
    {
        return Err(ApiError::bad_request("invalid benchmark id"));
    }
    let canonical = dunce::canonicalize(root.join(benchmark_id))
        .map_err(|_| ApiError::not_found(format!("unknown benchmark: {benchmark_id}")))?;
    if !canonical.starts_with(root) || !canonical.join("benchmark.json").is_file() {
        return Err(ApiError::not_found(format!(
            "unknown benchmark: {benchmark_id}"
        )));
    }
    Ok(canonical)
}

fn task_ids(configuration: &Value) -> Vec<String> {
    let Some(tasks) = configuration
        .get("tasks")
        .or_else(|| configuration.get("prompts"))
    else {
        return Vec::new();
    };
    if tasks
        .as_array()
        .is_some_and(|items| items.iter().all(Value::is_string))
    {
        return tasks
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
    }
    string_ids(Some(tasks))
}

fn string_ids(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            item.as_str()
                .or_else(|| item.get("id").and_then(Value::as_str))
        })
        .map(str::to_owned)
        .collect()
}

fn sum_u64(runs: &[Value], field: &str) -> u64 {
    runs.iter()
        .filter_map(|run| run.get(field).and_then(Value::as_u64))
        .sum()
}

fn default_runner() -> String {
    "run-benchmark.ps1".to_string()
}

fn default_review_root() -> String {
    "review".to_string()
}

fn default_parallelism() -> u16 {
    1
}

impl RunnerArguments {
    pub(super) fn values(&self) -> [Option<&str>; 8] {
        [
            self.stage.as_deref(),
            self.task.as_deref(),
            self.max_new_runs.as_deref(),
            self.replicate.as_deref(),
            self.parallelism.as_deref(),
            self.override_stage_gate.as_deref(),
            self.rerun_invalid.as_deref(),
            self.dry_run.as_deref(),
        ]
    }

    fn supported_controls(&self) -> Vec<&'static str> {
        let controls = [
            ("stage", &self.stage),
            ("task", &self.task),
            ("maxNewRuns", &self.max_new_runs),
            ("replicate", &self.replicate),
            ("parallelism", &self.parallelism),
            ("overrideStageGate", &self.override_stage_gate),
            ("rerunInvalid", &self.rerun_invalid),
            ("dryRun", &self.dry_run),
        ];
        controls
            .into_iter()
            .filter_map(|(name, argument)| argument.as_ref().map(|_| name))
            .collect()
    }
}

fn require_control(argument: Option<&str>, control: &str) -> Result<(), ApiError> {
    if argument.is_none() {
        return Err(ApiError::bad_request(format!(
            "this benchmark runner does not support {control}"
        )));
    }
    Ok(())
}

fn validate_relative_component(value: &str, label: &str) -> anyhow::Result<()> {
    let path = Path::new(value);
    anyhow::ensure!(
        path.components().count() == 1 && path.file_name().is_some(),
        "benchmark {label} must be a single relative path component"
    );
    Ok(())
}
