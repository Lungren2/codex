use crate::AppState;
use anyhow::{Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::State;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillEntry {
    pub name: String,
    pub installed: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillSource {
    pub source: String,
    pub target_home: String,
    pub skills: Vec<SkillEntry>,
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallSkillsRequest {
    pub source: String,
    pub target_home: String,
    pub skills: Vec<String>,
    pub existing: String,
}

fn skill_dirs(source: &Path) -> Result<Vec<String>> {
    let mut names = std::fs::read_dir(source)
        .with_context(|| format!("could not read skill source {}", source.display()))?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(|kind| kind.is_dir())
                .map(|_| entry.file_name().to_string_lossy().into_owned())
        })
        .collect::<Vec<_>>();
    names.sort();
    Ok(names)
}

fn defaults() -> (PathBuf, PathBuf) {
    let ecosystem = PathBuf::from(r"D:\dev\codex-dev\ecosystem.json");
    let parsed = std::fs::read_to_string(ecosystem)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
    let source = parsed
        .as_ref()
        .and_then(|json| json.pointer("/roots/skillsWorkshop"))
        .and_then(|value| value.as_str())
        .map(PathBuf::from)
        .unwrap_or_default()
        .join("skills");
    let target = parsed
        .as_ref()
        .and_then(|json| json.pointer("/homes/primary"))
        .and_then(|value| value.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".codex"));
    (source, target)
}

#[tauri::command]
pub fn skill_installer_defaults() -> SkillSource {
    let (source, target_home) = defaults();
    SkillSource {
        source: source.to_string_lossy().into_owned(),
        target_home: target_home.to_string_lossy().into_owned(),
        skills: Vec::new(),
    }
}

#[tauri::command]
pub fn inspect_skill_source(source: String, target_home: String) -> Result<SkillSource, String> {
    let source_path = PathBuf::from(&source);
    let target = PathBuf::from(&target_home).join("skills");
    let skills = skill_dirs(&source_path)
        .map_err(|error| format!("{error:#}"))?
        .into_iter()
        .map(|name| SkillEntry {
            installed: target.join(&name).is_dir(),
            name,
        })
        .collect();
    Ok(SkillSource {
        source,
        target_home,
        skills,
    })
}

fn copy_dir(source: &Path, target: &Path) -> Result<()> {
    std::fs::create_dir_all(target)?;
    for item in std::fs::read_dir(source)? {
        let item = item?;
        let destination = target.join(item.file_name());
        if item.file_type()?.is_dir() {
            copy_dir(&item.path(), &destination)?;
        } else {
            std::fs::copy(item.path(), destination)?;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn install_skills(
    state: State<'_, AppState>,
    request: InstallSkillsRequest,
) -> Result<Vec<String>, String> {
    let source = PathBuf::from(&request.source);
    let target_root = PathBuf::from(&request.target_home).join("skills");
    let available = skill_dirs(&source).map_err(|error| format!("{error:#}"))?;
    if request.skills.is_empty() {
        return Err("select at least one skill".into());
    }
    let backup_root = state.data_root.join("skill-backups");
    let mut installed = Vec::new();
    for name in request.skills {
        if !available.contains(&name) || Path::new(&name).components().count() != 1 {
            return Err(format!("invalid skill selection `{name}`"));
        }
        let from = source.join(&name);
        let to = target_root.join(&name);
        if to.exists() {
            match request.existing.as_str() {
                "overwrite" => std::fs::remove_dir_all(&to).map_err(|error| error.to_string())?,
                "backup" => {
                    std::fs::create_dir_all(&backup_root).map_err(|error| error.to_string())?;
                    let backup = backup_root.join(format!(
                        "{}-{}",
                        name,
                        chrono::Utc::now().format("%Y%m%d%H%M%S%3f")
                    ));
                    std::fs::rename(&to, backup).map_err(|error| error.to_string())?;
                }
                "skip" => continue,
                _ => return Err("invalid existing-skill handling".into()),
            }
        }
        std::fs::create_dir_all(&target_root).map_err(|error| error.to_string())?;
        copy_dir(&from, &to).map_err(|error| format!("{error:#}"))?;
        installed.push(name);
    }
    Ok(installed)
}
