use crate::models::{
    ArtifactStatus, DiffFile, DiffFileKind, DiffHunk, DiffLine, DiffLineKind, EvidenceRef,
    HunkCategory, HunkClassification, PatchSetIdentity, RepositoryIdentity, ReviewArtifact,
};
use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use regex::Regex;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use uuid::Uuid;

pub const DEFAULT_FILE_CAP: usize = 256 * 1024;
pub const DEFAULT_PATCH_CAP: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct CaptureOptions {
    pub base_ref_override: Option<String>,
    pub file_cap: usize,
    pub patch_cap: usize,
}

impl Default for CaptureOptions {
    fn default() -> Self {
        Self {
            base_ref_override: None,
            file_cap: DEFAULT_FILE_CAP,
            patch_cap: DEFAULT_PATCH_CAP,
        }
    }
}

pub struct CaptureResult {
    pub artifact: ReviewArtifact,
    pub patch_bytes: Vec<u8>,
}

pub fn current_fingerprint(root: &Path, base_ref: &str) -> Result<String> {
    let head_sha = git(root, &["rev-parse", "HEAD"])?;
    let base_sha = git(root, &["merge-base", base_ref, "HEAD"])?;
    let mut patch = git_bytes(
        root,
        &[
            "-c",
            "core.quotepath=false",
            "diff",
            "--no-ext-diff",
            "--find-renames",
            "--no-color",
            &base_sha,
            "--",
        ],
        &[0],
    )?;
    patch.extend(untracked_patch(root)?);
    let diff_sha256 = digest(&canonicalize_patch(&patch));
    let index_tree = git(root, &["write-tree"])?;
    Ok(digest(
        format!("{base_sha}\0{head_sha}\0{index_tree}\0{diff_sha256}").as_bytes(),
    ))
}

pub fn capture(
    cwd: &Path,
    thread_id: &str,
    turn_id: &str,
    revision: u32,
    blob_root: &Path,
    options: &CaptureOptions,
) -> Result<CaptureResult> {
    let root = repository_root(cwd)?;
    let base_ref = resolve_base_ref(&root, options.base_ref_override.as_deref())?;
    let head_sha = git(&root, &["rev-parse", "HEAD"])?;
    let base_sha = git(&root, &["merge-base", &base_ref, "HEAD"])?;
    let branch = git_optional(&root, &["symbolic-ref", "--short", "HEAD"]);
    let mut patch = git_bytes(
        &root,
        &[
            "-c",
            "core.quotepath=false",
            "diff",
            "--no-ext-diff",
            "--find-renames",
            "--no-color",
            &base_sha,
            "--",
        ],
        &[0],
    )?;
    patch.extend(untracked_patch(&root)?);
    let patch_bytes = canonicalize_patch(&patch);
    let diff_sha256 = digest(&patch_bytes);
    let index_tree = git(&root, &["write-tree"])?;
    let worktree_fingerprint =
        digest(format!("{base_sha}\0{head_sha}\0{index_tree}\0{diff_sha256}").as_bytes());

    let blob_dir = blob_root.join(&diff_sha256[..2]);
    fs::create_dir_all(&blob_dir)?;
    let blob_path = blob_dir.join(format!("{diff_sha256}.patch"));
    if !blob_path.exists() {
        fs::write(&blob_path, &patch_bytes)?;
        let mut permissions = fs::metadata(&blob_path)?.permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&blob_path, permissions)?;
    }

    let files = parse_patch(&patch_bytes, options.file_cap, options.patch_cap)?;
    let hunk_classifications = files
        .iter()
        .flat_map(|file| file.hunks.iter())
        .map(|hunk| HunkClassification {
            hunk_id: hunk.id.clone(),
            category: hunk.category.clone(),
            decision_ids: Vec::new(),
            contains_secret: hunk.contains_secret,
        })
        .collect();
    let other_changes = files
        .iter()
        .flat_map(|file| {
            file.hunks.iter().map(|hunk| EvidenceRef {
                path: file.path.clone(),
                hunk_id: hunk.id.clone(),
            })
        })
        .collect();
    let title = branch.as_ref().map_or_else(
        || "Detached worktree review".to_string(),
        |branch| format!("Review {branch}"),
    );
    let summary = if patch_bytes.is_empty() {
        "No repository changes were captured.".to_string()
    } else {
        format!("Captured {} changed file(s) for synthesis.", files.len())
    };
    let artifact = ReviewArtifact {
        schema_version: 1,
        artifact_id: Uuid::new_v4().to_string(),
        thread_id: thread_id.to_string(),
        turn_id: turn_id.to_string(),
        revision,
        repository: RepositoryIdentity {
            root: root.to_string_lossy().into_owned(),
            branch,
            base_ref,
            base_sha,
            head_sha,
            worktree_fingerprint,
        },
        patch_set: PatchSetIdentity {
            diff_sha256,
            blob_path: blob_path.to_string_lossy().into_owned(),
        },
        title,
        summary,
        status: ArtifactStatus::Collecting,
        created_at: Utc::now().to_rfc3339(),
        decisions: Vec::new(),
        validations: Vec::new(),
        hunk_classifications,
        other_changes,
        files,
        revision_comparison: Default::default(),
    };
    Ok(CaptureResult {
        artifact,
        patch_bytes,
    })
}

pub fn repository_root(cwd: &Path) -> Result<PathBuf> {
    let value = git(cwd, &["rev-parse", "--show-toplevel"])
        .context("the completion working directory is not inside a Git repository")?;
    Ok(PathBuf::from(value))
}

pub fn resolve_base_ref(root: &Path, override_ref: Option<&str>) -> Result<String> {
    if let Some(base_ref) = override_ref {
        git(root, &["rev-parse", "--verify", base_ref])
            .with_context(|| format!("configured base ref `{base_ref}` does not exist"))?;
        return Ok(base_ref.to_string());
    }
    if let Some(upstream) = git_optional(root, &["rev-parse", "--abbrev-ref", "@{upstream}"]) {
        return Ok(upstream);
    }
    if let Some(origin_head) = git_optional(
        root,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    ) {
        return Ok(origin_head);
    }
    bail!("base ref is required: configure an override, tracking branch, or origin/HEAD")
}

fn untracked_patch(root: &Path) -> Result<Vec<u8>> {
    let output = git_bytes(
        root,
        &["ls-files", "--others", "--exclude-standard", "-z"],
        &[0],
    )?;
    let mut patch = Vec::new();
    for raw_path in output
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        let relative = String::from_utf8_lossy(raw_path).replace('\\', "/");
        let absolute = root.join(String::from_utf8_lossy(raw_path).as_ref());
        let contents = fs::read(&absolute)
            .with_context(|| format!("failed to read untracked file `{relative}`"))?;
        patch.extend(
            format!("diff --git a/{relative} b/{relative}\nnew file mode 100644\n").bytes(),
        );
        if contents.contains(&0) {
            patch.extend(format!("Binary files /dev/null and b/{relative} differ\n").bytes());
            continue;
        }
        let text = String::from_utf8_lossy(&contents).replace("\r\n", "\n");
        let line_count = text.lines().count();
        patch.extend(
            format!("--- /dev/null\n+++ b/{relative}\n@@ -0,0 +1,{line_count} @@\n").bytes(),
        );
        for line in text.lines() {
            patch.push(b'+');
            patch.extend(line.bytes());
            patch.push(b'\n');
        }
        if !text.is_empty() && !text.ends_with('\n') {
            patch.extend(b"\\ No newline at end of file\n");
        }
    }
    Ok(patch)
}

fn canonicalize_patch(bytes: &[u8]) -> Vec<u8> {
    String::from_utf8_lossy(bytes)
        .replace("\r\n", "\n")
        .into_bytes()
}

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let output = git_bytes(root, args, &[0])?;
    Ok(String::from_utf8_lossy(&output).trim().to_string())
}

fn git_optional(root: &Path, args: &[&str]) -> Option<String> {
    git(root, args).ok().filter(|value| !value.is_empty())
}

fn git_bytes(root: &Path, args: &[&str], accepted_codes: &[i32]) -> Result<Vec<u8>> {
    let output = Command::new("git").args(args).current_dir(root).output()?;
    let code = output.status.code().unwrap_or(-1);
    if !accepted_codes.contains(&code) {
        return Err(anyhow!(
            String::from_utf8_lossy(&output.stderr).trim().to_string()
        ));
    }
    Ok(output.stdout)
}

pub fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn parse_patch(bytes: &[u8], file_cap: usize, patch_cap: usize) -> Result<Vec<DiffFile>> {
    let text = String::from_utf8(bytes.to_vec()).context("Git patch was not valid UTF-8")?;
    let mut sections = Vec::new();
    let mut current = String::new();
    for line in text.split_inclusive('\n') {
        if line.starts_with("diff --git ") && !current.is_empty() {
            sections.push(std::mem::take(&mut current));
        }
        current.push_str(line);
    }
    if !current.is_empty() {
        sections.push(current);
    }
    let mut consumed = 0usize;
    sections
        .into_iter()
        .map(|section| {
            let omitted_reason = if section.len() > file_cap {
                Some(format!(
                    "File patch exceeds the {file_cap}-byte synthesis cap"
                ))
            } else if consumed.saturating_add(section.len()) > patch_cap {
                Some(format!(
                    "Total patch exceeds the {patch_cap}-byte synthesis cap"
                ))
            } else {
                consumed += section.len();
                None
            };
            parse_file(&section, omitted_reason)
        })
        .collect()
}

fn parse_file(section: &str, omission_reason: Option<String>) -> Result<DiffFile> {
    let first = section
        .lines()
        .next()
        .context("diff section has no header")?;
    let (old_header_path, mut path) = parse_diff_header(first)?;
    let mut old_path = None;
    let mut kind = DiffFileKind::Modify;
    for line in section.lines() {
        if let Some(value) = line.strip_prefix("rename from ") {
            old_path = Some(value.to_string());
            kind = DiffFileKind::Rename;
        } else if let Some(value) = line.strip_prefix("rename to ") {
            path = value.to_string();
        } else if line.starts_with("new file mode ") {
            kind = DiffFileKind::Add;
        } else if line.starts_with("deleted file mode ") {
            kind = DiffFileKind::Delete;
        } else if line.starts_with("Binary files ") || line.starts_with("GIT binary patch") {
            kind = DiffFileKind::Binary;
        }
    }
    if old_path.is_none() && old_header_path != path {
        old_path = Some(old_header_path);
    }
    let generated = is_generated(&path);
    let omitted = omission_reason.is_some();
    let hunks = if omitted || kind == DiffFileKind::Binary {
        Vec::new()
    } else {
        parse_hunks(section, &path, generated)?
    };
    Ok(DiffFile {
        path,
        old_path,
        kind,
        generated,
        omitted,
        omission_reason,
        hunks,
    })
}

fn parse_diff_header(header: &str) -> Result<(String, String)> {
    let value = header
        .strip_prefix("diff --git a/")
        .context("invalid diff header")?;
    let (old, new) = value.rsplit_once(" b/").context("invalid diff paths")?;
    Ok((
        old.trim_matches('"').to_string(),
        new.trim_matches('"').to_string(),
    ))
}

fn parse_hunks(section: &str, path: &str, generated: bool) -> Result<Vec<DiffHunk>> {
    let header_re = Regex::new(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@")?;
    let mut result = Vec::new();
    let mut lines = section.lines().peekable();
    while let Some(line) = lines.next() {
        let Some(captures) = header_re.captures(line) else {
            continue;
        };
        let old_start = captures[1].parse::<u32>()?;
        let old_lines = captures
            .get(2)
            .map_or(Ok(1), |value| value.as_str().parse())?;
        let new_start = captures[3].parse::<u32>()?;
        let new_lines = captures
            .get(4)
            .map_or(Ok(1), |value| value.as_str().parse())?;
        let mut old_line = old_start;
        let mut new_line = new_start;
        let mut diff_lines = Vec::new();
        while let Some(next) = lines.peek() {
            if next.starts_with("@@ ") || next.starts_with("diff --git ") {
                break;
            }
            let raw = lines.next().unwrap_or_default();
            let (kind, content, old, new) = if let Some(content) = raw.strip_prefix('+') {
                let current = new_line;
                new_line += 1;
                (DiffLineKind::Addition, content, None, Some(current))
            } else if let Some(content) = raw.strip_prefix('-') {
                let current = old_line;
                old_line += 1;
                (DiffLineKind::Deletion, content, Some(current), None)
            } else if let Some(content) = raw.strip_prefix(' ') {
                let old_current = old_line;
                let new_current = new_line;
                old_line += 1;
                new_line += 1;
                (
                    DiffLineKind::Context,
                    content,
                    Some(old_current),
                    Some(new_current),
                )
            } else {
                (DiffLineKind::Meta, raw, None, None)
            };
            diff_lines.push(DiffLine {
                kind,
                content: content.to_string(),
                old_line: old,
                new_line: new,
            });
        }
        let body = diff_lines
            .iter()
            .map(|line| line.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let category = category(path, generated);
        let contains_secret = contains_secret(&body);
        let id = digest(
            format!("{path}\0{old_start}\0{old_lines}\0{new_start}\0{new_lines}\0{body}")
                .as_bytes(),
        );
        result.push(DiffHunk {
            id,
            path: path.to_string(),
            header: line.to_string(),
            old_start,
            old_lines,
            new_start,
            new_lines,
            lines: diff_lines,
            category,
            contains_secret,
        });
    }
    Ok(result)
}

fn is_generated(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".snap")
        || lower.ends_with(".lock")
        || lower.ends_with("-lock.json")
        || lower.contains("/generated/")
        || lower.contains("/dist/")
        || lower.contains("/target/")
        || lower.ends_with(".min.js")
        || lower.ends_with(".min.css")
}

fn category(path: &str, generated: bool) -> HunkCategory {
    if generated {
        return HunkCategory::Generated;
    }
    let lower = path.to_ascii_lowercase();
    if [
        "auth",
        "security",
        "permission",
        "migration",
        "secret",
        "credential",
    ]
    .iter()
    .any(|part| lower.contains(part))
    {
        HunkCategory::RiskSensitive
    } else if lower.contains("test") || lower.contains("fixture") || lower.ends_with(".md") {
        HunkCategory::Supporting
    } else {
        HunkCategory::Primary
    }
}

fn contains_secret(body: &str) -> bool {
    let patterns = [
        r"AKIA[0-9A-Z]{16}",
        r"(?i)-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----",
        r"(?i)bearer\s+[a-z0-9._~+/=-]{16,}",
        r#"(?i)(?:password|secret|api[_-]?key|access[_-]?token)\s*[:=]\s*["'][^"']{8,}["']"#,
    ];
    patterns
        .iter()
        .any(|pattern| Regex::new(pattern).is_ok_and(|regex| regex.is_match(body)))
}

#[cfg(test)]
#[path = "git_capture_tests.rs"]
mod tests;
