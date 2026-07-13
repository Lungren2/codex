use super::*;

#[test]
fn automatic_capture_requires_more_than_generated_changes() {
    let mut artifact: ReviewArtifact = serde_json::from_value(serde_json::json!({
        "schemaVersion": 1,
        "artifactId": "a", "threadId": "t", "turnId": "u", "revision": 1,
        "repository": { "root": "r", "branch": null, "baseRef": "main", "baseSha": "a", "headSha": "b", "worktreeFingerprint": "f" },
        "patchSet": { "diffSha256": "d", "blobPath": "p" },
        "title": "x", "summary": "x", "status": "collecting", "createdAt": "now",
        "decisions": [], "validations": [], "hunkClassifications": [], "otherChanges": [],
        "files": [{ "path": "Cargo.lock", "oldPath": null, "kind": "modify", "generated": true, "omitted": false, "omissionReason": null, "hunks": [] }]
    })).unwrap();
    assert!(!is_meaningful(&artifact));
    artifact.files[0].generated = false;
    artifact.files[0].omitted = true;
    assert!(is_meaningful(&artifact));
}
