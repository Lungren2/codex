use super::*;
use pretty_assertions::assert_eq;

fn failure_log(anchor: &str) -> String {
    let noise = "Compiling another dependency from the workspace\n".repeat(15);
    format!("{noise}libération: original evidence\n{noise}{anchor}\n{noise}")
}

fn stored_id(view: &str) -> Uuid {
    view.lines()
        .next()
        .and_then(|line| line.rsplit_once(' '))
        .and_then(|(_, id)| Uuid::parse_str(id.trim_end_matches('.')).ok())
        .expect("diagnostic view has an output_id")
}

#[test]
fn groups_runs_without_reordering_matches_or_changing_match_text() {
    let path = r"D:\code\école\src\main.rs";
    let second = "local/config/Dockerfile";
    let output =
        format!("{path}:12:  hello:world\r\n{path}:99:💡\n{second}:3: match\n{path}:105:last");
    let expected = format!(
        "File: {path}\n12:  hello:world\r\n99:💡\nFile: {second}\n3: match\nFile: {path}\n105:last"
    );
    let store = CommandOutputStore::default();
    assert_eq!(
        store.render(
            CommandOutputFormat::GroupedSearch,
            /*exit_code*/ 0,
            &output,
            MAX_VIEW_BYTES
        ),
        Some(expected),
    );
    for unrecognized in [
        format!("{output}\n--\n"),
        format!("{output}\n\x1b[35m{path}\x1b[0m:21:match\n"),
        "unique.rs:1:single hit\n".to_string(),
    ] {
        assert_eq!(
            store.render(
                CommandOutputFormat::GroupedSearch,
                /*exit_code*/ 0,
                &unrecognized,
                MAX_VIEW_BYTES
            ),
            None,
        );
    }
    assert_eq!(
        store.render(
            CommandOutputFormat::GroupedSearch,
            /*exit_code*/ 0,
            &output,
            /*budget*/ 12
        ),
        None,
    );
}

#[test]
fn renders_known_failures_and_recovers_original_evidence() {
    let store = CommandOutputStore::default();
    for anchor in [
        "memory allocation of 8388608 bytes failed",
        "test checkout::failed ... FAILED",
        "src/test.ts(2,4): error TS2304: Cannot find name",
        r#"{"reason":"compiler-message","message":{"level":"error","message":"bad type"}}"#,
    ] {
        let log = failure_log(anchor);
        let view = store
            .render(
                CommandOutputFormat::Diagnostics,
                /*exit_code*/ 1,
                &log,
                MAX_VIEW_BYTES,
            )
            .expect("diagnostic view");
        assert!(view.contains(anchor));
        assert!(!view.contains("original evidence"));
        let id = stored_id(&view);
        let page = store
            .read(
                id,
                /*offset*/ 0,
                Some("original evidence"),
                /*budget*/ 500,
            )
            .unwrap();
        assert!(page.ends_with("libération: original evidence\n"));
        assert!(page.len() <= 500);
        let full_page = store
            .read(id, /*offset*/ 0, /*query*/ None, MAX_VIEW_BYTES)
            .unwrap();
        assert_eq!(full_page.split_once('\n').unwrap().1, log);
        assert!(
            CommandOutputStore::default()
                .read(id, /*offset*/ 0, /*query*/ None, MAX_VIEW_BYTES)
                .is_err()
        );
    }
    assert_eq!(
        store.render(
            CommandOutputFormat::Diagnostics,
            /*exit_code*/ 1,
            &failure_log("unclassified failure"),
            MAX_VIEW_BYTES
        ),
        None,
    );
}

#[test]
fn pages_by_utf8_offset_and_evicts_old_captures() {
    let store = CommandOutputStore::default();
    let log = failure_log("error[E0001]: failed").replace("libération", &"💡".repeat(10));
    let view = store
        .render(
            CommandOutputFormat::Diagnostics,
            /*exit_code*/ 1,
            &log,
            MAX_VIEW_BYTES,
        )
        .unwrap();
    let id = stored_id(&view);
    let first_emoji = log.find('💡').unwrap();
    assert!(
        store
            .read(
                id,
                first_emoji + 1,
                /*query*/ None,
                /*budget*/ 500
            )
            .is_err()
    );
    let page = store
        .read(id, first_emoji, /*query*/ None, PAGE_HEADER_BYTES + 5)
        .unwrap();
    let next_offset = first_emoji + 4;
    let expected = format!(
        "Command output bytes {first_emoji}..{next_offset} of {}. next_offset: {next_offset}\n💡",
        log.len()
    );
    assert_eq!(page, expected);
    assert!(
        store
            .read(id, next_offset, /*query*/ None, /*budget*/ 500)
            .unwrap()
            .split_once('\n')
            .unwrap()
            .1
            .starts_with('💡')
    );
    for _ in 0..MAX_LOGS {
        store
            .render(
                CommandOutputFormat::Diagnostics,
                /*exit_code*/ 1,
                &log,
                MAX_VIEW_BYTES,
            )
            .unwrap();
    }
    assert_eq!(
        store.read(id, /*offset*/ 0, /*query*/ None, MAX_VIEW_BYTES),
        Err("command output is unavailable in this chat or has expired"),
    );
}
