mod app_server;
mod commands;
mod git_capture;
pub mod models;
mod notify;
mod notify_install;
mod skill_installer;
mod store;
mod synthesis;
mod tray;
mod utilities;

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use store::Store;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[derive(Clone)]
pub struct AppState {
    store: Arc<Store>,
    blob_root: PathBuf,
    data_root: PathBuf,
}

impl AppState {
    pub fn open_default() -> Result<Self> {
        let data_root = std::env::var_os("CODEX_REVIEW_DATA_DIR")
            .map(PathBuf::from)
            .or_else(|| dirs::data_local_dir().map(|path| path.join("codex-review")))
            .context("could not resolve the local application data directory")?;
        std::fs::create_dir_all(&data_root)?;
        let store = Arc::new(Store::open(&data_root.join("review.sqlite3"))?);
        let blob_root = data_root.join("patches");
        std::fs::create_dir_all(&blob_root)?;
        Ok(Self {
            store,
            blob_root,
            data_root,
        })
    }
}

pub fn ingest(payload_json: &str) -> Result<Option<String>> {
    let state = AppState::open_default()?;
    commands::ingest_payload(&state, payload_json)
}

pub fn install_notify_for_cli(
    state: &AppState,
    executable: &std::path::Path,
) -> Result<notify_install::NotifyInstallStatus> {
    notify_install::install(&state.data_root, executable)
}

pub fn uninstall_notify_for_cli(state: &AppState) -> Result<notify_install::NotifyInstallStatus> {
    notify_install::uninstall(&state.data_root)
}

/// Starts the notify command that preceded Codex Review in a chained wrapper.
/// The child is deliberately not waited on so Codex completion remains fire-and-forget.
pub fn spawn_chained_notify(payload_json: &str) -> Result<()> {
    notify::CompletionPayload::parse(payload_json)?;
    let state = AppState::open_default()?;
    let Some(command_line) = notify_install::chained_command(&state.data_root)? else {
        return Ok(());
    };
    let Some((program, arguments)) = command_line.split_first() else {
        return Ok(());
    };
    if program.to_ascii_lowercase().contains("codex-review") {
        anyhow::bail!("refusing a recursive Codex Review notify chain");
    }
    let mut command = Command::new(program);
    command.args(arguments).arg(payload_json);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    command.spawn()?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init()
        .ok();
    let state = AppState::open_default().expect("failed to initialize Codex Review storage");
    start_synthesis_worker(state.clone());
    start_reconciliation_worker(state.clone());
    tauri::Builder::default()
        .manage(state)
        .setup(|app| {
            tray::install(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_artifacts,
            commands::get_artifact,
            commands::capture_repository,
            commands::set_repository_base,
            commands::queue_feedback,
            commands::list_feedback,
            commands::install_notify,
            commands::uninstall_notify,
            commands::notify_status,
            commands::prepare_feedback_handoff,
            commands::exit_application,
            utilities::utility_status,
            utilities::run_utility,
            skill_installer::skill_installer_defaults,
            skill_installer::inspect_skill_source,
            skill_installer::install_skills,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Codex Review");
}

fn start_synthesis_worker(state: AppState) {
    thread::spawn(move || {
        loop {
            match state.store.claim_synthesis_job() {
                Ok(Some((mut artifact, payload_json))) => {
                    let payload = notify::CompletionPayload::parse(&payload_json).ok();
                    let evidence = if artifact.thread_id.starts_with("manual:") {
                        None
                    } else {
                        match app_server::read_desktop_turn(
                            &app_server::AppServerConfig::default(),
                            &artifact.thread_id,
                            &artifact.turn_id,
                        ) {
                            Ok(evidence) if evidence.status == "completed" => Some(evidence),
                            Ok(evidence) => {
                                artifact.status = models::ArtifactStatus::Unavailable;
                                let run = synthesis::SynthesisRun {
                                    artifact,
                                    attempts: 0,
                                    error: Some(format!(
                                        "originating turn ended with status `{}`",
                                        evidence.status
                                    )),
                                };
                                if let Err(error) = state.store.finish_synthesis(&run) {
                                    tracing::error!(%error, "failed to reject incomplete turn");
                                }
                                continue;
                            }
                            Err(error) => {
                                if let Err(store_error) = state
                                    .store
                                    .defer_synthesis(&artifact.artifact_id, &error.to_string())
                                {
                                    tracing::error!(%store_error, "failed to defer turn verification");
                                }
                                thread::sleep(Duration::from_secs(5));
                                continue;
                            }
                        }
                    };
                    if let Some(evidence) = &evidence {
                        artifact.validations = evidence.validations.clone();
                    }
                    let patch_workspace = PathBuf::from(&artifact.patch_set.blob_path)
                        .parent()
                        .map(PathBuf::from)
                        .unwrap_or_else(|| state.blob_root.clone());
                    let context = synthesis::SynthesisContext {
                        patch_workspace,
                        original_request: evidence
                            .as_ref()
                            .map(|evidence| evidence.original_request.clone())
                            .filter(|value| !value.is_empty())
                            .or_else(|| {
                                payload
                                    .as_ref()
                                    .map(|payload| payload.input_messages.join("\n\n"))
                            })
                            .unwrap_or_default(),
                        final_response: evidence
                            .as_ref()
                            .map(|evidence| evidence.final_response.clone())
                            .filter(|value| !value.is_empty())
                            .or_else(|| {
                                payload
                                    .as_ref()
                                    .and_then(|payload| payload.last_assistant_message.clone())
                            })
                            .unwrap_or_default(),
                        repository_instructions: load_repository_instructions(&artifact),
                    };
                    let result = synthesis::synthesize_artifact(
                        artifact,
                        &context,
                        &synthesis::SynthesisConfig::default(),
                    );
                    if let Err(error) = state.store.finish_synthesis(&result) {
                        tracing::error!(%error, "failed to persist synthesis result");
                    }
                }
                Ok(None) => thread::sleep(Duration::from_secs(2)),
                Err(error) => {
                    tracing::error!(%error, "synthesis worker failed to claim a job");
                    thread::sleep(Duration::from_secs(5));
                }
            }
        }
    });
}

fn load_repository_instructions(artifact: &models::ReviewArtifact) -> String {
    use std::collections::BTreeSet;

    const CAP: usize = 20_000;
    let root = PathBuf::from(&artifact.repository.root);
    let mut candidates = BTreeSet::from([root.join("AGENTS.md")]);
    for file in &artifact.files {
        let mut directory = root.join(&file.path).parent().map(PathBuf::from);
        while let Some(path) = directory {
            if !path.starts_with(&root) {
                break;
            }
            candidates.insert(path.join("AGENTS.md"));
            if path == root {
                break;
            }
            directory = path.parent().map(PathBuf::from);
        }
    }
    let mut output = String::new();
    for path in candidates {
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let remaining = CAP.saturating_sub(output.len());
        if remaining == 0 {
            break;
        }
        output.push_str(&format!("\n# {}\n", path.display()));
        output.extend(content.chars().take(remaining));
    }
    output
}

fn start_reconciliation_worker(state: AppState) {
    thread::spawn(move || {
        loop {
            let completed_since = chrono::Utc::now().timestamp() - 15 * 60;
            match app_server::recent_desktop_completions(
                &app_server::AppServerConfig::default(),
                completed_since,
            ) {
                Ok(completions) => {
                    for completion in completions {
                        let payload = serde_json::json!({
                            "type": "agent-turn-complete",
                            "thread-id": completion.thread_id,
                            "turn-id": completion.turn_id,
                            "cwd": completion.cwd,
                            "client": "codex-desktop",
                            "status": "completed",
                            "input-messages": completion.input_messages,
                            "last-assistant-message": completion.last_assistant_message,
                            "completed-at": completion.completed_at
                        });
                        if let Err(error) = commands::ingest_payload(&state, &payload.to_string()) {
                            tracing::warn!(%error, "could not reconcile completed Codex turn");
                        }
                    }
                }
                Err(error) => tracing::warn!(%error, "Codex turn reconciliation failed"),
            }
            thread::sleep(Duration::from_secs(60));
        }
    });
}
