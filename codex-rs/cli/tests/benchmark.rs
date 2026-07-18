use anyhow::Result;
use assert_cmd::prelude::*;
use codex_utils_cargo_bin::cargo_bin;
use predicates::prelude::*;
use std::process::Command;

#[test]
fn benchmark_command_exposes_launch_options() -> Result<()> {
    Command::new(cargo_bin("codex")?)
        .args(["benchmark", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--root"))
        .stdout(predicate::str::contains("--port"))
        .stdout(predicate::str::contains("--no-open"));
    Ok(())
}
