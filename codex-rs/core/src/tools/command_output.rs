//! Optional model-facing views of completed command output.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::PoisonError;

use serde::Deserialize;
use uuid::Uuid;

const MAX_VIEW_BYTES: usize = 8_000;
const MAX_LOGS: usize = 8;
const PAGE_BYTES: usize = 6_000;
const MAX_QUERY_BYTES: usize = 256;
const PAGE_HEADER_BYTES: usize = 120;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CommandOutputFormat {
    #[default]
    Raw,
    GroupedSearch,
    Diagnostics,
}

#[derive(Default)]
pub(crate) struct CommandOutputStore(Mutex<VecDeque<(Uuid, String)>>);

impl CommandOutputStore {
    pub(crate) fn render(
        &self,
        format: CommandOutputFormat,
        exit_code: i32,
        output: &str,
        budget: usize,
    ) -> Option<String> {
        let budget = budget.min(MAX_VIEW_BYTES);
        match format {
            CommandOutputFormat::GroupedSearch if exit_code == 0 => group_search(output, budget),
            CommandOutputFormat::Diagnostics if exit_code != 0 => {
                // The handler only calls this with a complete capture. Do not retain
                // an unbounded copy or emit a reference we cannot serve.
                if output.len() > crate::unified_exec::UNIFIED_EXEC_OUTPUT_MAX_BYTES {
                    return None;
                }
                let id = Uuid::new_v4();
                let view = diagnostic_view(output, budget, id)?;
                let mut logs = self.0.lock().unwrap_or_else(PoisonError::into_inner);
                if logs.len() == MAX_LOGS {
                    logs.pop_front();
                }
                logs.push_back((id, output.to_string()));
                Some(view)
            }
            CommandOutputFormat::Raw
            | CommandOutputFormat::GroupedSearch
            | CommandOutputFormat::Diagnostics => None,
        }
    }

    pub(crate) fn read(
        &self,
        id: Uuid,
        offset: usize,
        query: Option<&str>,
        budget: usize,
    ) -> Result<String, &'static str> {
        if query.is_some_and(|query| query.is_empty() || query.len() > MAX_QUERY_BYTES) {
            return Err("query must contain 1 to 256 bytes");
        }
        let page_bytes = PAGE_BYTES.min(budget.saturating_sub(PAGE_HEADER_BYTES));
        if page_bytes < query.map_or(4, |query| query.len() * 2 + 4) {
            return Err("tool output budget is too small for the requested page or query");
        }
        let logs = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let Some((_, output)) = logs.iter().find(|(key, _)| *key == id) else {
            return Err("command output is unavailable in this chat or has expired");
        };
        let Some(rest) = output.get(offset..) else {
            return Err("offset must be a valid UTF-8 byte boundary in the command output");
        };
        let start = if let Some(query) = query {
            let Some(found) = rest.find(query) else {
                return Ok("No matching lines at or after the requested offset.".to_string());
            };
            let found = offset + found;
            // Include the line's prefix where possible. For a long line, start near
            // the match so the bounded result still contains the requested evidence.
            let line_start = output[offset..found]
                .rfind('\n')
                .map_or(offset, |i| offset + i + 1);
            line_start.max(output.ceil_char_boundary(found.saturating_sub(page_bytes / 2)))
        } else {
            offset
        };
        let mut end = output.floor_char_boundary(output.len().min(start + page_bytes));
        if let Some(query) = query
            && let Some(found) = output[start..end].find(query)
        {
            let match_end = start + found + query.len();
            if let Some(line_end) = output[match_end..end].find('\n') {
                end = match_end + line_end + 1;
            }
        }
        let next = if end < output.len() {
            format!("next_offset: {end}")
        } else {
            "End of command output".to_string()
        };
        let total = output.len();
        let page = &output[start..end];
        Ok(format!(
            "Command output bytes {start}..{end} of {total}. {next}\n{page}"
        ))
    }
}

fn search_line(line: &str) -> Option<(&str, &str)> {
    let drive = usize::from(
        line.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
            && line.as_bytes().get(1) == Some(&b':')
            && matches!(line.as_bytes().get(2), Some(b'/' | b'\\')),
    ) * 2;
    let prefix = drive + line.get(drive..)?.find(':')?;
    let (path, numbered) = (&line[..prefix], &line[prefix + 1..]);
    if path.is_empty() || path.chars().any(char::is_control) {
        return None;
    }
    let (line_number, _) = numbered.split_once(':')?;
    (line_number.bytes().all(|byte| byte.is_ascii_digit())
        && line_number.parse::<usize>().ok()? > 0)
        .then_some((path, numbered))
}

fn group_search(output: &str, budget: usize) -> Option<String> {
    let mut grouped = String::new();
    let mut previous = None;
    for line in output.split_inclusive('\n') {
        // Unknown output, context separators, and colored paths go through raw.
        let (file, numbered) = search_line(line)?;
        if previous != Some(file) {
            grouped.push_str("File: ");
            grouped.push_str(file);
            grouped.push('\n');
            previous = Some(file);
        }
        grouped.push_str(numbered);
        if grouped.len() > budget {
            return None;
        }
    }
    (grouped.len() < output.len()).then_some(grouped)
}

fn diagnostic_anchor(line: &str) -> bool {
    let line = line.trim_start();
    if line.starts_with('{') {
        return line.contains("compiler-message")
            && serde_json::from_str::<serde_json::Value>(line).is_ok_and(|row| {
                row["reason"] == "compiler-message"
                    && matches!(
                        row["message"]["level"].as_str(),
                        Some("error" | "failure-note")
                    )
            });
    }
    let lower = line.to_ascii_lowercase();
    lower.starts_with("error:")
        || lower.starts_with("error[e")
        || lower.starts_with("fatal error")
        || lower.starts_with("fail ")
        || lower.starts_with("failed ")
        || lower.starts_with("assertionerror")
        || lower.starts_with("rustc-llvm error:")
        || lower.starts_with("llvm error:")
        || (lower.starts_with("memory allocation of ") && lower.ends_with(" bytes failed"))
        || (lower.starts_with("test ") && lower.trim_end().ends_with("... failed"))
        || lower.contains(": error ts")
}

fn diagnostic_view(output: &str, budget: usize, id: Uuid) -> Option<String> {
    let lines: Vec<_> = output.split_inclusive('\n').collect();
    let mut retain = vec![false; lines.len()];
    let mut found = false;
    for (i, line) in lines.iter().enumerate() {
        if diagnostic_anchor(line.trim_end()) {
            retain[i.saturating_sub(2)..(i + 6).min(lines.len())].fill(true);
            found = true;
        }
    }
    if !found {
        return None;
    }
    let count = retain.len();
    retain[..count.min(3)].fill(true);
    retain[count.saturating_sub(6)..].fill(true);
    let omitted = retain.iter().filter(|keep| !**keep).count();
    if omitted == 0 {
        return None;
    }
    let mut view = format!(
        "Diagnostic view ({omitted} lines omitted). Read the original with read_command_output using output_id {id}.\n"
    );
    let mut gap = 0;
    for (line, keep) in lines.into_iter().zip(retain) {
        if !keep {
            gap += 1;
            continue;
        }
        if gap > 0 {
            view.push_str(&format!("[... {gap} lines omitted ...]\n"));
            gap = 0;
        }
        if view.len().saturating_add(line.len()) > budget {
            return None;
        }
        view.push_str(line);
    }
    (view.len() < output.len()).then_some(view)
}

#[cfg(test)]
#[path = "command_output_tests.rs"]
mod tests;
