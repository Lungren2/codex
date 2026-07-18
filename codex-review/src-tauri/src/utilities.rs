use crate::AppState;
use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use tauri::{AppHandle, Manager, State};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UtilityResult {
    pub utility: String,
    pub mode: String,
    pub result: Value,
}

fn utility_script(name: &str) -> Result<&'static str> {
    match name {
        "session-retention" => Ok("session-retention/session-retention.mjs"),
        "daily-digest" => Ok("daily-digest/daily-digest.mjs"),
        _ => anyhow::bail!("unknown utility `{name}`"),
    }
}

fn utility_root(app: &AppHandle) -> Result<PathBuf> {
    let resource = app
        .path()
        .resource_dir()
        .context("could not resolve application resources")?;
    let bundled = resource.join("tools");
    if bundled.exists() {
        return Ok(bundled);
    }
    Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("could not resolve repository root")?
        .join("tools"))
}

fn runtime_config(state: &AppState, name: &str) -> Result<PathBuf> {
    let dir = state.data_root.join("utilities").join(name);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("config.json");
    if !path.exists() {
        let mut value = match name {
            "session-retention" => serde_json::json!({
                "codexHome": dirs::home_dir().context("could not resolve home directory")?.join(".codex"),
                "codexCommand": "codex", "codexCommandArgs": [], "nodeCommand": "node",
                "archive": { "scheduleTime": "08:00", "intervalDays": 1, "inactiveHours": 24 },
                "delete": { "scheduleTime": "08:00", "intervalDays": 1, "retentionDays": 2 },
                "lockStaleMinutes": 120, "schedulerName": "CodexSessionRetention"
            }),
            "daily-digest" => serde_json::json!({
                "codexHome": dirs::home_dir().context("could not resolve home directory")?.join(".codex"),
                "outputDir": "~/Desktop", "outputFilename": "Codex Daily Digest.html", "digestDirName": "Codex Daily Digests",
                "outputFormat": "html", "tmpDir": "tmp", "codexCommand": "codex", "codexCommandArgs": [], "nodeCommand": "node",
                "model": null, "profile": null, "scheduleTime": "08:30", "intervalDays": 1, "maxCatchUpDays": 14,
                "includeArchivedSessions": true, "includeRetentionLogs": true, "inferenceTimeoutMs": 180000,
                "lockStaleMinutes": 120, "schedulerName": "CodexDailyDigest"
            }),
            _ => anyhow::bail!("unknown utility `{name}`"),
        };
        std::fs::write(
            &path,
            format!("{}\n", serde_json::to_string_pretty(&mut value)?),
        )?;
    }
    Ok(path)
}

fn execute(app: &AppHandle, state: &AppState, name: &str, live: bool) -> Result<UtilityResult> {
    let script = utility_root(app)?.join(utility_script(name)?);
    let config = runtime_config(state, name)?;
    let mut args = vec![
        script.to_string_lossy().to_string(),
        if live { "run".into() } else { "status".into() },
    ];
    if !live {
        args.push("--dry-run".into());
    }
    args.extend([
        "--json".into(),
        "--config".into(),
        config.to_string_lossy().to_string(),
    ]);
    let output = Command::new("node")
        .args(&args)
        .output()
        .context("could not launch Node.js; install Node to run Codex utilities")?;
    if !output.status.success() {
        anyhow::bail!(
            "utility failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let result = serde_json::from_slice(&output.stdout).context("utility returned invalid JSON")?;
    Ok(UtilityResult {
        utility: name.into(),
        mode: if live { "run" } else { "status" }.into(),
        result,
    })
}

#[tauri::command]
pub fn utility_status(
    app: AppHandle,
    state: State<'_, AppState>,
    utility: String,
) -> Result<UtilityResult, String> {
    execute(&app, &state, &utility, false).map_err(|error| format!("{error:#}"))
}

#[tauri::command]
pub fn run_utility(
    app: AppHandle,
    state: State<'_, AppState>,
    utility: String,
) -> Result<UtilityResult, String> {
    execute(&app, &state, &utility, true).map_err(|error| format!("{error:#}"))
}
