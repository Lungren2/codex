use std::collections::HashSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

use crate::app_server::{AppServerClient, AppServerConfig, SynthesisTurnRequest};
use crate::models::{ArtifactStatus, DecisionStatus, EvidenceRef, ReviewArtifact, ReviewDecision};

const DEFAULT_PROMPT_CAP: usize = 32_000;
const MIN_PROMPT_CAP: usize = 8_000;

#[derive(Clone, Debug)]
pub struct SynthesisConfig {
    pub app_server: AppServerConfig,
    pub max_attempts: u8,
    pub prompt_byte_cap: usize,
}

impl Default for SynthesisConfig {
    fn default() -> Self {
        Self {
            app_server: AppServerConfig::default(),
            max_attempts: 2,
            prompt_byte_cap: DEFAULT_PROMPT_CAP,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SynthesisContext {
    pub patch_workspace: PathBuf,
    pub original_request: String,
    pub final_response: String,
    pub repository_instructions: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SynthesisRun {
    pub artifact: ReviewArtifact,
    pub attempts: u8,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct SynthesisDocument {
    title: String,
    summary: String,
    decisions: Vec<ReviewDecision>,
    other_changes: Vec<EvidenceRef>,
}

#[derive(Debug, Error)]
enum SynthesisValidationError {
    #[error("synthesis referenced unknown evidence {path}#{hunk_id}")]
    UnknownEvidence { path: String, hunk_id: String },
    #[error("hunk {path}#{hunk_id} was not assigned to a decision or otherChanges")]
    MissingCoverage { path: String, hunk_id: String },
    #[error("claim `{0}` has neither evidence nor an inference marker")]
    UnsupportedClaim(String),
    #[error("decision `{decision_id}` referenced unknown validation `{validation_id}`")]
    UnknownValidation {
        decision_id: String,
        validation_id: String,
    },
    #[error("synthesis field `{0}` exceeded its hard size limit")]
    Oversized(&'static str),
}

pub fn synthesize_artifact(
    mut artifact: ReviewArtifact,
    context: &SynthesisContext,
    config: &SynthesisConfig,
) -> SynthesisRun {
    artifact.status = ArtifactStatus::Synthesizing;
    let max_attempts = config.max_attempts.max(1);
    let mut last_error = None;
    for attempt in 1..=max_attempts {
        match synthesize_once(&artifact, context, config) {
            Ok(document) => {
                artifact.title = document.title;
                artifact.summary = document.summary;
                artifact.decisions = document.decisions;
                artifact.other_changes = document.other_changes;
                for classification in &mut artifact.hunk_classifications {
                    classification.decision_ids = artifact
                        .decisions
                        .iter()
                        .filter(|decision| {
                            decision
                                .evidence
                                .iter()
                                .any(|evidence| evidence.hunk_id == classification.hunk_id)
                        })
                        .map(|decision| decision.id.clone())
                        .collect();
                }
                artifact.status = ArtifactStatus::Ready;
                return SynthesisRun {
                    artifact,
                    attempts: attempt,
                    error: None,
                };
            }
            Err(error) => last_error = Some(error.to_string()),
        }
    }

    artifact.status = ArtifactStatus::Partial;
    artifact.decisions.clear();
    artifact.other_changes = all_evidence(&artifact);
    if artifact.summary.trim().is_empty() {
        artifact.summary =
            "Semantic synthesis failed; the complete authoritative diff remains available.".into();
    }
    SynthesisRun {
        artifact,
        attempts: max_attempts,
        error: last_error,
    }
}

fn synthesize_once(
    artifact: &ReviewArtifact,
    context: &SynthesisContext,
    config: &SynthesisConfig,
) -> Result<SynthesisDocument, Box<dyn std::error::Error>> {
    let request = SynthesisTurnRequest {
        cwd: context.patch_workspace.clone(),
        prompt: build_prompt(artifact, context, config.prompt_byte_cap),
        output_schema: output_schema(),
    };
    let mut client = AppServerClient::spawn(&config.app_server)?;
    let response = client.run_synthesis_turn(&request)?;
    let mut document: SynthesisDocument = serde_json::from_value(response.message)?;
    for decision in &mut document.decisions {
        decision.status = DecisionStatus::Unresolved;
    }
    validate_document(artifact, &document)?;
    Ok(document)
}

fn build_prompt(artifact: &ReviewArtifact, context: &SynthesisContext, cap: usize) -> String {
    let cap = cap.max(MIN_PROMPT_CAP);
    let mut prompt = String::from(
        "You are a detached, read-only code-change synthesizer. Group the authoritative hunks into review decisions, not files. Do not claim validations that are not listed. Every claim needs evidence, unless inference is true. Assign every hunk exactly through decision evidence or otherChanges. Secret-marked hunk bodies are deliberately redacted; they still need an evidence assignment. Return only JSON matching the output schema.\n\n",
    );
    push_capped(
        &mut prompt,
        "ORIGINAL REQUEST\n",
        &context.original_request,
        cap,
    );
    push_capped(
        &mut prompt,
        "\nFINAL ASSISTANT RESPONSE\n",
        &context.final_response,
        cap,
    );
    push_capped(
        &mut prompt,
        "\nREPOSITORY INSTRUCTIONS\n",
        &context.repository_instructions,
        cap,
    );
    push_capped(
        &mut prompt,
        "\nARTIFACT IDENTITY AND VALIDATIONS\n",
        &serde_json::to_string_pretty(&json!({
            "artifactId": artifact.artifact_id,
            "diffSha256": artifact.patch_set.diff_sha256,
            "repository": artifact.repository,
            "validations": artifact.validations,
        }))
        .unwrap_or_default(),
        cap,
    );
    push_capped(
        &mut prompt,
        "\nAUTHORITATIVE HUNKS\n",
        &render_hunks(artifact),
        cap,
    );
    prompt
}

fn push_capped(target: &mut String, heading: &str, content: &str, cap: usize) {
    if target.len() >= cap {
        return;
    }
    let heading_boundary = floor_char_boundary(heading, cap - target.len());
    target.push_str(&heading[..heading_boundary]);
    let available = cap.saturating_sub(target.len());
    if content.len() <= available {
        target.push_str(content);
    } else {
        const TRUNCATED: &str = "\n[TRUNCATED]";
        let boundary = floor_char_boundary(content, available.saturating_sub(TRUNCATED.len()));
        target.push_str(&content[..boundary]);
        target.push_str(&TRUNCATED[..TRUNCATED.len().min(available - boundary)]);
    }
}

fn floor_char_boundary(value: &str, mut index: usize) -> usize {
    index = index.min(value.len());
    while !value.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn render_hunks(artifact: &ReviewArtifact) -> String {
    let mut output = String::new();
    for file in &artifact.files {
        if file.omitted {
            output.push_str(&format!(
                "FILE {} [OMITTED: {}]\n",
                file.path,
                file.omission_reason.as_deref().unwrap_or("size policy")
            ));
        }
        for hunk in &file.hunks {
            output.push_str(&format!(
                "FILE {} HUNK {} CATEGORY {:?}\n",
                file.path, hunk.id, hunk.category
            ));
            if hunk.contains_secret {
                output.push_str("[REDACTED: LOCAL SECRET PATTERN DETECTED]\n");
            } else {
                output.push_str(&hunk.header);
                output.push('\n');
                for line in &hunk.lines {
                    output.push(match line.kind {
                        crate::models::DiffLineKind::Addition => '+',
                        crate::models::DiffLineKind::Deletion => '-',
                        crate::models::DiffLineKind::Context => ' ',
                        crate::models::DiffLineKind::Meta => '\\',
                    });
                    output.push_str(&line.content);
                    output.push('\n');
                }
            }
        }
    }
    output
}

fn validate_document(
    artifact: &ReviewArtifact,
    document: &SynthesisDocument,
) -> Result<(), SynthesisValidationError> {
    if document.title.len() > 200 {
        return Err(SynthesisValidationError::Oversized("title"));
    }
    if document.summary.len() > 2_000 {
        return Err(SynthesisValidationError::Oversized("summary"));
    }
    if document.decisions.len() > 50 || document.other_changes.len() > 500 {
        return Err(SynthesisValidationError::Oversized(
            "decisions or otherChanges",
        ));
    }
    let known: HashSet<(String, String)> = all_evidence(artifact)
        .into_iter()
        .map(|evidence| (evidence.path, evidence.hunk_id))
        .collect();
    let validations: HashSet<&str> = artifact
        .validations
        .iter()
        .map(|validation| validation.id.as_str())
        .collect();
    let mut covered = HashSet::new();

    for decision in &document.decisions {
        if decision.id.len() > 100
            || decision.title.len() > 200
            || decision.consequence.len() > 500
            || decision.summary.len() > 2_000
            || decision.claims.len() > 30
            || decision.evidence.len() > 500
            || decision.validation_ids.len() > 30
        {
            return Err(SynthesisValidationError::Oversized("decision"));
        }
        for validation_id in &decision.validation_ids {
            if !validations.contains(validation_id.as_str()) {
                return Err(SynthesisValidationError::UnknownValidation {
                    decision_id: decision.id.clone(),
                    validation_id: validation_id.clone(),
                });
            }
        }
        for claim in &decision.claims {
            if claim.text.len() > 1_000 || claim.evidence.len() > 100 {
                return Err(SynthesisValidationError::Oversized("claim"));
            }
            if claim.evidence.is_empty() && !claim.inference {
                return Err(SynthesisValidationError::UnsupportedClaim(
                    claim.text.clone(),
                ));
            }
            validate_refs(&known, &claim.evidence, &mut covered)?;
        }
        validate_refs(&known, &decision.evidence, &mut covered)?;
    }
    validate_refs(&known, &document.other_changes, &mut covered)?;
    for (path, hunk_id) in known {
        if !covered.contains(&(path.clone(), hunk_id.clone())) {
            return Err(SynthesisValidationError::MissingCoverage { path, hunk_id });
        }
    }
    Ok(())
}

fn validate_refs(
    known: &HashSet<(String, String)>,
    evidence: &[EvidenceRef],
    covered: &mut HashSet<(String, String)>,
) -> Result<(), SynthesisValidationError> {
    for reference in evidence {
        let key = (reference.path.clone(), reference.hunk_id.clone());
        if !known.contains(&key) {
            return Err(SynthesisValidationError::UnknownEvidence {
                path: reference.path.clone(),
                hunk_id: reference.hunk_id.clone(),
            });
        }
        covered.insert(key);
    }
    Ok(())
}

fn all_evidence(artifact: &ReviewArtifact) -> Vec<EvidenceRef> {
    artifact
        .files
        .iter()
        .flat_map(|file| {
            file.hunks.iter().map(|hunk| EvidenceRef {
                path: file.path.clone(),
                hunk_id: hunk.id.clone(),
            })
        })
        .collect()
}

fn output_schema() -> Value {
    let evidence = json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["path", "hunkId"],
        "properties": { "path": {"type": "string"}, "hunkId": {"type": "string"} }
    });
    let evidence_array = json!({ "type": "array", "items": evidence });
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["title", "summary", "decisions", "otherChanges"],
        "properties": {
            "title": {"type": "string", "maxLength": 200},
            "summary": {"type": "string", "maxLength": 2000},
            "decisions": {
                "type": "array",
                "maxItems": 50,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["id", "title", "consequence", "summary", "importance", "risk", "claims", "evidence", "validationIds", "status"],
                    "properties": {
                        "id": {"type": "string", "maxLength": 100},
                        "title": {"type": "string", "maxLength": 200},
                        "consequence": {"type": "string", "maxLength": 500},
                        "summary": {"type": "string", "maxLength": 2000},
                        "importance": {"type": "string", "enum": ["primary", "supporting", "riskSensitive"]},
                        "risk": {"type": ["string", "null"]},
                        "claims": {
                            "type": "array",
                            "maxItems": 30,
                            "items": {
                                "type": "object",
                                "additionalProperties": false,
                                "required": ["text", "evidence", "inference"],
                                "properties": {
                                    "text": {"type": "string", "maxLength": 1000},
                                    "evidence": evidence_array,
                                    "inference": {"type": "boolean"}
                                }
                            }
                        },
                        "evidence": evidence_array,
                        "validationIds": {"type": "array", "maxItems": 30, "items": {"type": "string", "maxLength": 100}},
                        "status": {"type": "string", "enum": ["unresolved"]}
                    }
                }
            },
            "otherChanges": evidence_array
        }
    })
}

#[cfg(test)]
#[path = "synthesis_tests.rs"]
mod tests;
