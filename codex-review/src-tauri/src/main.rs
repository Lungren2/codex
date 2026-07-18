fn main() {
    let arguments = std::env::args().collect::<Vec<_>>();
    if arguments
        .get(1)
        .is_some_and(|argument| argument == "install-notify")
    {
        let state = codex_review_lib::AppState::open_default().expect("failed to open app state");
        let executable = std::env::current_exe().expect("failed to resolve executable");
        let status = codex_review_lib::install_notify_for_cli(&state, &executable)
            .expect("failed to install Codex notify integration");
        println!(
            "{}",
            serde_json::to_string(&status).expect("serialize status")
        );
        return;
    }
    if arguments
        .get(1)
        .is_some_and(|argument| argument == "uninstall-notify")
    {
        let state = codex_review_lib::AppState::open_default().expect("failed to open app state");
        let status = codex_review_lib::uninstall_notify_for_cli(&state)
            .expect("failed to uninstall Codex notify integration");
        println!(
            "{}",
            serde_json::to_string(&status).expect("serialize status")
        );
        return;
    }
    if arguments
        .get(1)
        .is_some_and(|argument| argument == "ingest")
    {
        let payload = match arguments.get(2) {
            Some(payload) => payload,
            None => {
                eprintln!("usage: codex-review ingest '<json-payload>'");
                std::process::exit(2);
            }
        };
        let result = codex_review_lib::ingest(payload);
        if let Err(error) = codex_review_lib::spawn_chained_notify(payload) {
            eprintln!("codex-review could not start chained notify: {error:#}");
        }
        match result {
            Ok(Some(artifact_id)) => tracing::info!(%artifact_id, "captured completion"),
            Ok(None) => {}
            Err(error) => {
                eprintln!("codex-review ingest failed: {error:#}");
                std::process::exit(1);
            }
        }
        return;
    }
    codex_review_lib::run();
}
