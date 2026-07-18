use super::*;
use crate::benchmark_manifest::HarnessConfiguration;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::fs;

#[test]
fn discovers_manifest_and_aggregates_recorded_runs() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let suite = root.path().join("sample-suite");
    fs::create_dir_all(suite.join("results"))?;
    fs::create_dir_all(suite.join("review"))?;
    fs::write(suite.join("run-benchmark.ps1"), "param()")?;
    fs::write(suite.join("review").join("index.html"), "review")?;
    fs::write(
        suite.join("benchmark.json"),
        serde_json::to_vec(&json!({
            "schemaVersion": 3,
            "id": "sample-suite",
            "stages": [{ "id": "gate" }],
            "tasks": [{ "id": "first" }, { "id": "second" }]
        }))?,
    )?;
    fs::write(
        suite.join("benchmark-harness.json"),
        serde_json::to_vec(&json!({
            "supportsParallelism": true,
            "maxParallelism": 4,
            "arguments": {
                "stage": "Stage",
                "task": "TaskId",
                "maxNewRuns": "MaxNewRuns",
                "replicate": "Replicate",
                "parallelism": "Parallelism",
                "dryRun": "DryRun"
            }
        }))?,
    )?;
    fs::write(
        suite.join("results").join("scores.json"),
        serde_json::to_vec(&json!([
            { "valid": true, "inputTokens": 12, "outputTokens": 3, "durationMs": 40 },
            { "valid": false, "inputTokens": 8, "outputTokens": 2, "durationMs": 60 }
        ]))?,
    )?;

    assert_eq!(
        discover_benchmarks(root.path())?,
        vec![BenchmarkSummary {
            id: "sample-suite".to_string(),
            schema_version: Some(3),
            path: suite.display().to_string(),
            stages: vec!["gate".to_string()],
            tasks: vec!["first".to_string(), "second".to_string()],
            runner_available: true,
            review_available: true,
            supports_parallelism: true,
            max_parallelism: 4,
            supported_controls: vec![
                "stage",
                "task",
                "maxNewRuns",
                "replicate",
                "parallelism",
                "dryRun",
            ],
            run_count: 2,
            valid_run_count: 1,
            input_tokens: 20,
            output_tokens: 5,
            duration_ms: 100,
        }]
    );
    Ok(())
}

#[test]
fn validates_manifest_parallelism_and_known_selectors() {
    let configuration = json!({
        "stages": [{ "id": "gate" }],
        "tasks": [{ "id": "task" }]
    });
    let harness = HarnessConfiguration {
        supports_parallelism: true,
        max_parallelism: 3,
        arguments: serde_json::from_value(json!({
            "stage": "Stage",
            "task": "TaskId",
            "maxNewRuns": "MaxNewRuns",
            "replicate": "Replicate",
            "parallelism": "Parallelism",
            "dryRun": "DryRun"
        }))
        .expect("runner arguments"),
        ..HarnessConfiguration::default()
    };
    let valid = RunRequest {
        stage: Some("gate".to_string()),
        task_id: Some("task".to_string()),
        max_new_runs: Some(2),
        replicate: Some(1),
        parallelism: Some(3),
        override_stage_gate: Some(false),
        rerun_invalid: Some(false),
        dry_run: Some(true),
    };
    assert_eq!(
        validate_run_request(&configuration, &harness, &valid),
        Ok(())
    );

    let invalid = RunRequest {
        parallelism: Some(4),
        ..valid
    };
    assert_eq!(
        validate_run_request(&configuration, &harness, &invalid),
        Err(ApiError::bad_request("parallelism must be between 1 and 3"))
    );
}

#[test]
fn rejects_benchmark_path_traversal() {
    let root = tempfile::tempdir().expect("temp root");
    assert_eq!(
        benchmark_directory(root.path(), "../outside"),
        Err(ApiError::bad_request("invalid benchmark id"))
    );
}
