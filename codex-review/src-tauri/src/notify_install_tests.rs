use std::fs;

use super::*;
use pretty_assertions::assert_eq;
use tempfile::TempDir;

#[test]
fn install_state_round_trips_previous_command() {
    let temporary = TempDir::new().unwrap();
    let data_root = temporary.path().join("data");
    fs::create_dir_all(&data_root).unwrap();
    let state = NotifyInstallState {
        installed_executable: r"C:\apps\codex-review.exe".to_string(),
        previous_notify: Some(vec!["old-notify.exe".to_string(), "turn-ended".to_string()]),
        installed_at: "2026-07-11T00:00:00Z".to_string(),
    };
    write_json_atomic(&data_root.join(INSTALL_STATE_FILE), &state).unwrap();
    assert_eq!(read_state(&data_root).unwrap(), Some(state));
}

#[test]
fn recognizes_only_the_owned_ingest_command() {
    let executable = r"C:\apps\codex-review.exe";
    assert!(is_ours(
        &[executable.to_string(), "ingest".to_string()],
        executable
    ));
    assert!(!is_ours(
        &[executable.to_string(), "other".to_string()],
        executable
    ));
}

#[test]
fn install_and_uninstall_restore_the_previous_notify() {
    let temporary = TempDir::new().unwrap();
    let data_root = temporary.path().join("data");
    let config_path = temporary.path().join("config.toml");
    let executable = temporary.path().join("codex-review.exe");
    fs::write(
        &config_path,
        "model = \"gpt-test\"\nnotify = [\"old-notify.exe\", \"turn-ended\"]\n",
    )
    .unwrap();
    fs::write(&executable, b"test executable").unwrap();

    let installed = install_at(&data_root, &executable, &config_path).unwrap();
    assert!(installed.installed);
    assert_eq!(
        installed.chained_command,
        Some(vec!["old-notify.exe".to_string(), "turn-ended".to_string()])
    );
    let installed_config = fs::read_to_string(&config_path).unwrap();
    assert!(installed_config.contains("codex-review.exe"));
    assert!(installed_config.contains("\"ingest\""));

    let uninstalled = uninstall_at(&data_root, &config_path).unwrap();
    assert!(!uninstalled.installed);
    let restored: DocumentMut = fs::read_to_string(&config_path).unwrap().parse().unwrap();
    assert_eq!(
        notify_argv(&restored).unwrap(),
        Some(vec!["old-notify.exe".to_string(), "turn-ended".to_string()])
    );
    assert_eq!(restored["model"].as_str(), Some("gpt-test"));
}
