use crate::benchmark_manifest::ApiError;
use crate::benchmark_manifest::BenchmarkSummary;
use crate::benchmark_manifest::RunRequest;
use crate::benchmark_manifest::benchmark_directory;
use crate::benchmark_manifest::discover_benchmarks;
use crate::benchmark_manifest::read_benchmark;
use crate::benchmark_manifest::read_harness;
use crate::benchmark_manifest::resolve_benchmark_root;
use crate::benchmark_manifest::summarize_benchmark;
use crate::benchmark_manifest::validate_run_request;
use anyhow::Context;
use axum::Json;
use axum::Router;
use axum::extract::Path as AxumPath;
use axum::extract::State;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::IntoResponse;
use axum::response::Redirect;
use axum::response::Response;
use axum::routing::get;
use axum::routing::post;
use clap::Parser;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::SystemTime;
use tokio::process::Command;
use tokio::sync::RwLock;

const APP_HTML: &str = include_str!("../assets/benchmark/index.html");
const APP_JS: &str = include_str!("../assets/benchmark/app.js");
const APP_CSS: &str = include_str!("../assets/benchmark/styles.css");

#[derive(Debug, Parser)]
pub struct BenchmarkCommand {
    /// Directory containing benchmark subdirectories with benchmark.json manifests.
    #[arg(long, env = "CODEX_BENCHMARK_ROOT", value_name = "DIRECTORY")]
    root: Option<PathBuf>,

    /// Loopback port for the benchmark web app. Use 0 to select an available port.
    #[arg(long, default_value_t = 4178)]
    port: u16,

    /// Print the URL without opening a browser.
    #[arg(long, default_value_t = false)]
    no_open: bool,
}

#[derive(Clone)]
struct AppState {
    benchmark_root: Arc<PathBuf>,
    jobs: Arc<RwLock<BTreeMap<String, BenchmarkJob>>>,
    next_job_id: Arc<AtomicU64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BenchmarkJob {
    id: String,
    benchmark_id: String,
    status: JobStatus,
    started_at_unix_ms: u128,
    finished_at_unix_ms: Option<u128>,
    exit_code: Option<i32>,
    output: String,
    request: RunRequest,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum JobStatus {
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BenchmarkDetail {
    summary: BenchmarkSummary,
    configuration: Value,
    runs: Vec<Value>,
    active_job: Option<BenchmarkJob>,
}

#[derive(Debug, serde::Deserialize)]
struct BenchmarkPath {
    benchmark_id: String,
}

#[derive(Debug, serde::Deserialize)]
struct ReviewPath {
    benchmark_id: String,
    path: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({ "error": self.message })),
        )
            .into_response()
    }
}

pub async fn run(command: BenchmarkCommand) -> anyhow::Result<()> {
    let root = resolve_benchmark_root(command.root)?;
    let state = AppState {
        benchmark_root: Arc::new(root),
        jobs: Arc::new(RwLock::new(BTreeMap::new())),
        next_job_id: Arc::new(AtomicU64::new(1)),
    };
    let app = Router::new()
        .route("/", get(index))
        .route("/app.js", get(app_js))
        .route("/styles.css", get(app_css))
        .route("/api/benchmarks", get(list_benchmarks))
        .route("/api/benchmarks/{benchmark_id}", get(get_benchmark))
        .route("/api/benchmarks/{benchmark_id}/run", post(start_benchmark))
        .route("/api/jobs", get(list_jobs))
        .route("/review/{benchmark_id}", get(review_redirect))
        .route("/review/{benchmark_id}/", get(review_index))
        .route("/review/{benchmark_id}/{*path}", get(review_asset))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", command.port))
        .await
        .context("failed to bind benchmark server")?;
    let address = listener.local_addr()?;
    let url = format!("http://{address}/");
    println!("Codex benchmark app: {url}");
    if !command.no_open
        && let Err(error) = webbrowser::open(&url)
    {
        eprintln!("Could not open the browser: {error}");
    }
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .context("benchmark server failed")
}

async fn index() -> impl IntoResponse {
    static_response("text/html; charset=utf-8", APP_HTML)
}

async fn app_js() -> impl IntoResponse {
    static_response("text/javascript; charset=utf-8", APP_JS)
}

async fn app_css() -> impl IntoResponse {
    static_response("text/css; charset=utf-8", APP_CSS)
}

fn static_response(content_type: &'static str, body: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-store"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data: blob:; frame-src 'self'; connect-src 'self'; form-action 'none'; object-src 'none'; base-uri 'none'",
            ),
        ],
        body,
    )
        .into_response()
}

async fn list_benchmarks(
    State(state): State<AppState>,
) -> Result<Json<Vec<BenchmarkSummary>>, ApiError> {
    discover_benchmarks(&state.benchmark_root)
        .map(Json)
        .map_err(ApiError::internal)
}

async fn get_benchmark(
    State(state): State<AppState>,
    AxumPath(BenchmarkPath { benchmark_id }): AxumPath<BenchmarkPath>,
) -> Result<Json<BenchmarkDetail>, ApiError> {
    let benchmark_dir = benchmark_directory(&state.benchmark_root, &benchmark_id)?;
    let (configuration, runs) = read_benchmark(&benchmark_dir).map_err(ApiError::internal)?;
    let harness = read_harness(&benchmark_dir, &configuration).map_err(ApiError::internal)?;
    let summary = summarize_benchmark(&benchmark_dir, &configuration, &harness, &runs);
    let active_job = state
        .jobs
        .read()
        .await
        .values()
        .find(|job| job.benchmark_id == benchmark_id && job.status == JobStatus::Running)
        .cloned();
    Ok(Json(BenchmarkDetail {
        summary,
        configuration,
        runs,
        active_job,
    }))
}

async fn list_jobs(State(state): State<AppState>) -> Json<Vec<BenchmarkJob>> {
    Json(state.jobs.read().await.values().cloned().collect())
}

async fn start_benchmark(
    State(state): State<AppState>,
    AxumPath(BenchmarkPath { benchmark_id }): AxumPath<BenchmarkPath>,
    Json(mut request): Json<RunRequest>,
) -> Result<(StatusCode, Json<BenchmarkJob>), ApiError> {
    let benchmark_dir = benchmark_directory(&state.benchmark_root, &benchmark_id)?;
    let (configuration, _) = read_benchmark(&benchmark_dir).map_err(ApiError::internal)?;
    let harness = read_harness(&benchmark_dir, &configuration).map_err(ApiError::internal)?;
    validate_run_request(&configuration, &harness, &request)?;
    if !harness.supports_parallelism {
        request.parallelism = None;
    }
    let runner = benchmark_dir.join(&harness.runner);
    if !runner.is_file() {
        return Err(ApiError::not_found(format!(
            "benchmark runner is missing: {}",
            runner.display()
        )));
    }
    let mut jobs = state.jobs.write().await;
    if jobs
        .values()
        .any(|job| job.benchmark_id == benchmark_id && job.status == JobStatus::Running)
    {
        return Err(ApiError::conflict(
            "this benchmark already has an active run",
        ));
    }
    let sequence = state.next_job_id.fetch_add(1, Ordering::Relaxed);
    let job_id = format!("job-{sequence:06}");
    let job = BenchmarkJob {
        id: job_id.clone(),
        benchmark_id: benchmark_id.clone(),
        status: JobStatus::Running,
        started_at_unix_ms: unix_millis(),
        finished_at_unix_ms: None,
        exit_code: None,
        output: String::new(),
        request: request.clone(),
    };
    jobs.insert(job_id.clone(), job.clone());
    drop(jobs);
    tokio::spawn(run_benchmark_job(
        state.clone(),
        job_id,
        benchmark_dir,
        request,
        runner,
        harness.arguments,
    ));
    Ok((StatusCode::ACCEPTED, Json(job)))
}

async fn run_benchmark_job(
    state: AppState,
    job_id: String,
    benchmark_dir: PathBuf,
    request: RunRequest,
    runner: PathBuf,
    arguments: crate::benchmark_manifest::RunnerArguments,
) {
    let result = execute_runner(&benchmark_dir, &runner, &arguments, &request).await;
    let mut jobs = state.jobs.write().await;
    let Some(job) = jobs.get_mut(&job_id) else {
        return;
    };
    job.finished_at_unix_ms = Some(unix_millis());
    match result {
        Ok(output) => {
            job.exit_code = output.status.code();
            job.status = if output.status.success() {
                JobStatus::Succeeded
            } else {
                JobStatus::Failed
            };
            job.output = truncate_output(format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        Err(error) => {
            job.status = JobStatus::Failed;
            job.output = error.to_string();
        }
    }
}

async fn execute_runner(
    benchmark_dir: &Path,
    runner: &Path,
    arguments: &crate::benchmark_manifest::RunnerArguments,
    request: &RunRequest,
) -> anyhow::Result<std::process::Output> {
    let shell = which::which("pwsh")
        .or_else(|_| which::which("powershell.exe"))
        .context("PowerShell 7 or Windows PowerShell is required to run this benchmark")?;
    let mut command = Command::new(shell);
    command
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-File")
        .arg(runner)
        .current_dir(benchmark_dir)
        .kill_on_drop(true);
    if let (Some(argument), Some(stage)) = (arguments.stage.as_deref(), request.stage.as_deref()) {
        command.arg(format!("-{argument}")).arg(stage);
    }
    if let (Some(argument), Some(task_id)) = (arguments.task.as_deref(), request.task_id.as_deref())
    {
        command.arg(format!("-{argument}")).arg(task_id);
    }
    if let (Some(argument), Some(max_new_runs)) =
        (arguments.max_new_runs.as_deref(), request.max_new_runs)
    {
        command
            .arg(format!("-{argument}"))
            .arg(max_new_runs.to_string());
    }
    if let (Some(argument), Some(replicate)) = (arguments.replicate.as_deref(), request.replicate) {
        command
            .arg(format!("-{argument}"))
            .arg(replicate.to_string());
    }
    if let (Some(argument), Some(parallelism)) =
        (arguments.parallelism.as_deref(), request.parallelism)
    {
        command
            .arg(format!("-{argument}"))
            .arg(parallelism.to_string());
    }
    if request.override_stage_gate == Some(true)
        && let Some(argument) = arguments.override_stage_gate.as_deref()
    {
        command.arg(format!("-{argument}"));
    }
    if request.rerun_invalid == Some(true)
        && let Some(argument) = arguments.rerun_invalid.as_deref()
    {
        command.arg(format!("-{argument}"));
    }
    if request.dry_run == Some(true)
        && let Some(argument) = arguments.dry_run.as_deref()
    {
        command.arg(format!("-{argument}"));
    }
    command
        .output()
        .await
        .context("benchmark runner failed to start")
}

async fn review_index(
    State(state): State<AppState>,
    AxumPath(BenchmarkPath { benchmark_id }): AxumPath<BenchmarkPath>,
) -> Result<Response, ApiError> {
    serve_review_file(&state.benchmark_root, &benchmark_id, "index.html").await
}

async fn review_redirect(
    State(state): State<AppState>,
    AxumPath(BenchmarkPath { benchmark_id }): AxumPath<BenchmarkPath>,
) -> Result<Redirect, ApiError> {
    benchmark_directory(&state.benchmark_root, &benchmark_id)?;
    let encoded: String = url::form_urlencoded::byte_serialize(benchmark_id.as_bytes()).collect();
    Ok(Redirect::permanent(&format!("/review/{encoded}/")))
}

async fn review_asset(
    State(state): State<AppState>,
    AxumPath(ReviewPath { benchmark_id, path }): AxumPath<ReviewPath>,
) -> Result<Response, ApiError> {
    serve_review_file(&state.benchmark_root, &benchmark_id, &path).await
}

async fn serve_review_file(
    root: &Path,
    benchmark_id: &str,
    relative: &str,
) -> Result<Response, ApiError> {
    let benchmark_dir = benchmark_directory(root, benchmark_id)?;
    let (configuration, _) = read_benchmark(&benchmark_dir).map_err(ApiError::internal)?;
    let harness = read_harness(&benchmark_dir, &configuration).map_err(ApiError::internal)?;
    let review_root = benchmark_dir
        .join(harness.review_root)
        .canonicalize()
        .map_err(|_| ApiError::not_found("blind review has not been built"))?;
    let canonical = review_root
        .join(relative)
        .canonicalize()
        .map_err(|_| ApiError::not_found("review asset not found"))?;
    if !canonical.starts_with(&review_root) || !canonical.is_file() {
        return Err(ApiError::not_found("review asset not found"));
    }
    let body = tokio::fs::read(&canonical)
        .await
        .map_err(ApiError::internal)?;
    let generated_case = relative.replace('\\', "/").starts_with("cases/");
    let content_security_policy = if generated_case {
        "default-src 'self' data: blob:; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'none'; form-action 'none'; object-src 'none'; base-uri 'none'"
    } else {
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; frame-src 'self'; connect-src 'self'; form-action 'none'; object-src 'none'; base-uri 'none'"
    };
    Ok((
        [
            (header::CONTENT_TYPE, content_type(&canonical)),
            (header::CACHE_CONTROL, "no-store"),
            (header::CONTENT_SECURITY_POLICY, content_security_policy),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        body,
    )
        .into_response())
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("css") => "text/css; charset=utf-8",
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        Some("webp") => "image/webp",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

fn truncate_output(output: String) -> String {
    const MAX_CHARS: usize = 40_000;
    if output.chars().count() <= MAX_CHARS {
        return output;
    }
    let suffix: String = output
        .chars()
        .rev()
        .take(MAX_CHARS)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("[earlier output truncated]\n{suffix}")
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
}

#[cfg(test)]
#[path = "benchmark_cmd_tests.rs"]
mod tests;
