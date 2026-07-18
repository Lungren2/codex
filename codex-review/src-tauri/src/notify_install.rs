use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use toml_edit::{Array, DocumentMut, Item, Value};

const INSTALL_STATE_FILE: &str = "notify-install.json";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NotifyInstallState {
    pub installed_executable: String,
    pub previous_notify: Option<Vec<String>>,
    pub installed_at: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NotifyInstallStatus {
    pub installed: bool,
    pub config_path: String,
    pub chained_command: Option<Vec<String>>,
}

pub fn install(data_root: &Path, executable: &Path) -> Result<NotifyInstallStatus> {
    let config_path = codex_config_path()?;
    install_at(data_root, executable, &config_path)
}

fn install_at(
    data_root: &Path,
    executable: &Path,
    config_path: &Path,
) -> Result<NotifyInstallStatus> {
    let source = fs::read_to_string(&config_path).unwrap_or_default();
    let mut document = source
        .parse::<DocumentMut>()
        .context("Codex config.toml could not be parsed safely")?;
    let executable = executable
        .canonicalize()
        .unwrap_or_else(|_| executable.to_path_buf())
        .to_string_lossy()
        .into_owned();
    let current = notify_argv(&document)?;
    if current
        .as_ref()
        .is_some_and(|argv| is_ours(argv, &executable))
    {
        return status_at(data_root, config_path);
    }

    fs::create_dir_all(data_root)?;
    let state = NotifyInstallState {
        installed_executable: executable.clone(),
        previous_notify: current,
        installed_at: chrono::Utc::now().to_rfc3339(),
    };
    write_json_atomic(&data_root.join(INSTALL_STATE_FILE), &state)?;
    document["notify"] = Item::Value(Value::Array(string_array(&[
        executable,
        "ingest".to_string(),
    ])));
    write_config_with_backup(&config_path, &source, document.to_string().as_bytes())?;
    status_at(data_root, config_path)
}

pub fn uninstall(data_root: &Path) -> Result<NotifyInstallStatus> {
    let config_path = codex_config_path()?;
    uninstall_at(data_root, &config_path)
}

fn uninstall_at(data_root: &Path, config_path: &Path) -> Result<NotifyInstallStatus> {
    let source = fs::read_to_string(&config_path).unwrap_or_default();
    let mut document = source
        .parse::<DocumentMut>()
        .context("Codex config.toml could not be parsed safely")?;
    let state =
        read_state(data_root)?.context("Codex Review notify installation is not recorded")?;
    let current = notify_argv(&document)?;
    if !current
        .as_ref()
        .is_some_and(|argv| is_ours(argv, &state.installed_executable))
    {
        bail!("Codex notify changed after installation; refusing to overwrite the newer command");
    }
    match state.previous_notify {
        Some(argv) => document["notify"] = Item::Value(Value::Array(string_array(&argv))),
        None => {
            document.remove("notify");
        }
    }
    write_config_with_backup(&config_path, &source, document.to_string().as_bytes())?;
    fs::remove_file(data_root.join(INSTALL_STATE_FILE)).ok();
    status_at(data_root, config_path)
}

pub fn status(data_root: &Path) -> Result<NotifyInstallStatus> {
    let config_path = codex_config_path()?;
    status_at(data_root, &config_path)
}

fn status_at(data_root: &Path, config_path: &Path) -> Result<NotifyInstallStatus> {
    let document = fs::read_to_string(&config_path)
        .unwrap_or_default()
        .parse::<DocumentMut>()
        .context("Codex config.toml could not be parsed safely")?;
    let state = read_state(data_root)?;
    let current = notify_argv(&document)?;
    let installed = state.as_ref().is_some_and(|state| {
        current
            .as_ref()
            .is_some_and(|argv| is_ours(argv, &state.installed_executable))
    });
    Ok(NotifyInstallStatus {
        installed,
        config_path: config_path.to_string_lossy().into_owned(),
        chained_command: state.and_then(|state| state.previous_notify),
    })
}

pub fn chained_command(data_root: &Path) -> Result<Option<Vec<String>>> {
    Ok(read_state(data_root)?.and_then(|state| state.previous_notify))
}

fn codex_config_path() -> Result<PathBuf> {
    let home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|path| path.join(".codex")))
        .context("could not resolve CODEX_HOME")?;
    fs::create_dir_all(&home)?;
    Ok(home.join("config.toml"))
}

fn notify_argv(document: &DocumentMut) -> Result<Option<Vec<String>>> {
    let Some(item) = document.get("notify") else {
        return Ok(None);
    };
    let array = item
        .as_array()
        .context("top-level Codex `notify` must be an array of strings")?;
    array
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .context("top-level Codex `notify` must contain only strings")
        })
        .collect::<Result<Vec<_>>>()
        .map(Some)
}

fn string_array(values: &[String]) -> Array {
    let mut array = Array::new();
    for value in values {
        array.push(value.as_str());
    }
    array
}

fn is_ours(argv: &[String], executable: &str) -> bool {
    argv.len() >= 2
        && argv[1] == "ingest"
        && paths_equal(Path::new(&argv[0]), Path::new(executable))
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    let left = left.canonicalize().unwrap_or_else(|_| left.to_path_buf());
    let right = right.canonicalize().unwrap_or_else(|_| right.to_path_buf());
    left.to_string_lossy()
        .eq_ignore_ascii_case(&right.to_string_lossy())
}

fn read_state(data_root: &Path) -> Result<Option<NotifyInstallState>> {
    let path = data_root.join(INSTALL_STATE_FILE);
    if !path.exists() {
        return Ok(None);
    }
    serde_json::from_slice(&fs::read(&path)?)
        .context("Codex Review notify install state is invalid")
        .map(Some)
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<()> {
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(value)?)?;
    replace_file(&temporary, path)
}

fn write_config_with_backup(path: &Path, previous: &str, bytes: &[u8]) -> Result<()> {
    let backup = path.with_extension("toml.codex-review.bak");
    fs::write(&backup, previous)?;
    let temporary = path.with_extension("toml.codex-review.tmp");
    fs::write(&temporary, bytes)?;
    replace_file(&temporary, path)
}

fn replace_file(source: &Path, destination: &Path) -> Result<()> {
    if destination.exists() {
        fs::remove_file(destination)?;
    }
    fs::rename(source, destination)?;
    Ok(())
}

#[cfg(test)]
#[path = "notify_install_tests.rs"]
mod tests;
