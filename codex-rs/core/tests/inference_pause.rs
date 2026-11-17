//! Exercise the model-request boundary, rather than merely counting host polls.

use anyhow::Result;
use codex_core::StartThreadOptions;
use codex_core::TurnInputRequest;
use codex_features::Feature;
use codex_protocol::dynamic_tools::DynamicToolCallOutputContentItem;
use codex_protocol::dynamic_tools::DynamicToolFunctionSpec;
use codex_protocol::dynamic_tools::DynamicToolResponse;
use codex_protocol::dynamic_tools::DynamicToolSpec;
use codex_protocol::models::PermissionProfile;
use codex_protocol::openai_models::ToolMode;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::ThreadSettingsOverrides;
use codex_protocol::user_input::UserInput;
use codex_utils_path_uri::PathUri;
use core_test_support::TestTargetOs;
use core_test_support::responses;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_custom_tool_call;
use core_test_support::responses::ev_function_call;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::sse;
use core_test_support::test_codex::TestCodex;
use core_test_support::test_codex::test_codex;
use core_test_support::test_codex::turn_permission_fields;
use core_test_support::test_target_os;
use core_test_support::wait_for_event;
use core_test_support::wait_for_event_match;
use pretty_assertions::assert_eq;
use serde_json::json;
use test_case::test_case;
use tokio::time::Duration;

// Install the same helper aliases as the aggregate core suite without pulling
// unrelated suite modules into this focused integration-test target.
#[ctor::ctor]
static TEST_BINARY_DISPATCH: Option<codex_test_binary_support::TestBinaryDispatchGuard> = {
    codex_test_binary_support::configure_test_binary_dispatch(
        "codex-inference-pause",
        |exe, arg| {
            use codex_test_binary_support::TestBinaryDispatchMode;
            if arg == Some(codex_exec_server::CODEX_FS_HELPER_ARG1)
                || arg == Some(codex_apply_patch::CODEX_CORE_APPLY_PATCH_ARG1)
                || exe == codex_sandboxing::landlock::CODEX_LINUX_SANDBOX_ARG0
            {
                return TestBinaryDispatchMode::DispatchArg0Only;
            }
            #[cfg(unix)]
            if arg == Some(codex_exec_server::CODEX_ARG0_EXEC_HELPER_ARG1) {
                return TestBinaryDispatchMode::DispatchArg0Only;
            }
            TestBinaryDispatchMode::InstallAliases
        },
    )
};

#[derive(Clone, Copy)]
enum Resolution {
    Success,
    Failure,
    Interrupt,
    Preempt,
    Background,
    Noisy,
    Tty,
}

#[test_case(Resolution::Success; "success")]
#[test_case(Resolution::Failure; "failure")]
#[test_case(Resolution::Interrupt; "interrupt")]
#[test_case(Resolution::Preempt; "user_preemption")]
#[test_case(Resolution::Background; "background")]
#[test_case(Resolution::Noisy; "bounded_output")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn code_cell_suspends_inference_until_resolution(resolution: Resolution) -> Result<()> {
    let server = responses::start_mock_server().await;
    let mut builder = test_codex()
        .with_model_info_override("gpt-5.5", |model| {
            model.use_responses_lite = true;
            model.tool_mode = Some(ToolMode::CodeMode);
        })
        .with_config(move |config| {
            config.model = Some("gpt-5.5".to_string());
            let _ = config.features.enable(Feature::CodeModeInterrupt);
            let _ = config.features.disable(Feature::CodeModeHost);
            if matches!(resolution, Resolution::Preempt) {
                let _ = config.features.enable(Feature::InstantInterrupt);
            }

            let _ = config.features.enable(Feature::CodeMode);
            config.tool_inference_pause_threshold_ms = Some(25);
        });
    let mut test = builder.build_with_auto_env(&server).await?;
    let thread = test
        .thread_manager
        .start_thread(StartThreadOptions {
            dynamic_tools: vec![DynamicToolSpec::Function(DynamicToolFunctionSpec {
                name: "held".to_string(),
                description: "Wait for the test's release.".to_string(),
                input_schema: json!({"type":"object","properties":{},"additionalProperties":false}),
                defer_loading: false,
            })],
            ..StartThreadOptions::new(test.config.clone())
        })
        .await?;
    test.codex = thread.thread;
    test.session_configured = thread.session_configured;
    let ending = match resolution {
        Resolution::Failure => "throw new Error('resolved failure');",
        Resolution::Success
        | Resolution::Interrupt
        | Resolution::Preempt
        | Resolution::Background
        | Resolution::Noisy
        | Resolution::Tty => "text('after');",
    };
    // A tiny model-supplied yield must not defeat the host scheduling policy.
    let yield_ms = if matches!(resolution, Resolution::Background) {
        0
    } else {
        1
    };
    let noise = if matches!(resolution, Resolution::Noisy) {
        "text('x'.repeat(100000));"
    } else {
        ""
    };
    let code = if matches!(resolution, Resolution::Background) {
        "// @exec: {\"yield_time_ms\": 0}\nawait new Promise(resolve => setTimeout(resolve, 10000)); text('background-done');".to_string()
    } else {
        format!(
            "// @exec: {{\"yield_time_ms\": {yield_ms}, \"max_output_tokens\": 1000000}}\ntext('before'); {noise} text(await tools.held({{}})); {ending}"
        )
    };
    let mut model_responses = vec![
        sse(vec![
            ev_response_created("r1"),
            ev_custom_tool_call("held-cell", "exec", &code),
            ev_completed("r1"),
        ]),
        sse(vec![
            ev_response_created("r2"),
            ev_assistant_message("done", "finished"),
            ev_completed("r2"),
        ]),
    ];
    if matches!(resolution, Resolution::Interrupt) {
        model_responses.truncate(1);
    }
    let mock = responses::mount_sse_sequence(&server, model_responses).await;
    start_input(
        &test,
        "exercise the held tool",
        ThreadSettingsOverrides::default(),
    )
    .await?;
    if matches!(resolution, Resolution::Background) {
        wait_for_event(&test.codex, |event| {
            matches!(event, EventMsg::TurnComplete(_))
        })
        .await;
        assert_eq!(
            mock.requests().len(),
            2,
            "explicit background work must release inference"
        );
        let output = mock.requests()[1]
            .custom_tool_call_output("held-cell")
            .to_string();
        assert!(output.contains("Script running with cell ID"));
        test.codex.shutdown_and_wait().await?;
        return Ok(());
    }
    let request = wait_for_event_match(&test.codex, |event| match event {
        EventMsg::DynamicToolCallRequest(request) => Some(request.clone()),
        _ => None,
    })
    .await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        mock.requests().len(),
        1,
        "no inference request while the tool is unresolved"
    );
    if matches!(resolution, Resolution::Preempt) {
        start_input(
            &test,
            "steer while the owned tool is held",
            Default::default(),
        )
        .await?;
        tokio::time::timeout(Duration::from_secs(10), async {
            while mock.requests().len() < 2 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await?;
        let requests = mock.requests();
        assert_eq!(
            requests.len(),
            2,
            "an explicit user wake resumes inference once"
        );
        let output = requests[1].custom_tool_call_output("held-cell").to_string();
        assert!(output.contains("Script running with cell ID") && output.contains("before"));
        test.codex.shutdown_and_wait().await?;
        return Ok(());
    }
    if matches!(resolution, Resolution::Interrupt) {
        test.codex.submit(Op::Interrupt).await?;
        wait_for_event(&test.codex, |event| {
            matches!(event, EventMsg::TurnAborted(_))
        })
        .await;
        assert_eq!(mock.requests().len(), 1);
        test.codex.shutdown_and_wait().await?;
        return Ok(());
    }
    test.codex
        .submit(Op::DynamicToolResponse {
            id: request.call_id,
            response: DynamicToolResponse {
                content_items: vec![DynamicToolCallOutputContentItem::InputText {
                    text: "released".to_string(),
                }],
                success: true,
            },
        })
        .await?;
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    let requests = mock.requests();
    assert_eq!(requests.len(), 2);
    let output = requests[1].custom_tool_call_output("held-cell").to_string();
    assert!(
        output.contains("before") && output.contains("released"),
        "preserve output across the elapsed threshold: {output}"
    );
    match resolution {
        Resolution::Success | Resolution::Noisy | Resolution::Tty => {
            assert!(output.contains("Script completed") && output.contains("after"))
        }
        Resolution::Failure => {
            assert!(output.contains("Script failed") && output.contains("resolved failure"))
        }
        Resolution::Interrupt | Resolution::Preempt | Resolution::Background => unreachable!(),
    }
    if matches!(resolution, Resolution::Noisy) {
        assert!(
            output.len() < 40_000,
            "foreground accumulation must obey a hard cap even for an oversized requested budget"
        );
    }
    test.codex.shutdown_and_wait().await?;
    Ok(())
}

#[test_case(Resolution::Success; "foreground")]
#[test_case(Resolution::Background; "background")]
#[test_case(Resolution::Tty; "foreground_pty")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn foreground_command_suspends_inference_until_exit(resolution: Resolution) -> Result<()> {
    let server = responses::start_mock_server().await;
    let mut builder = test_codex()
        .with_model_info_override("gpt-5.5", |model| {
            model.tool_mode = Some(ToolMode::Direct);
        })
        .with_config(|config| {
            config.model = Some("gpt-5.5".to_string());

            config.tool_inference_pause_threshold_ms = Some(25);
        });
    let test = builder.build_with_auto_env(&server).await?;
    let gate = PathUri::from_abs_path(test.executor_environment().cwd()).join("release-command")?;
    let cmd = if test_target_os() == TestTargetOs::Windows {
        let gate = "release-command";
        format!(
            "Write-Output 'command-started'; $attempt = 0; while (-not (Test-Path -LiteralPath '{gate}') -and $attempt -lt 3000) {{ Start-Sleep -Milliseconds 10; $attempt++ }}; if (-not (Test-Path -LiteralPath '{gate}')) {{ exit 99 }}; Write-Output 'command-resolved'; exit 7"
        )
    } else {
        let gate = "release-command";
        format!(
            "printf 'command-started\\n'; attempt=0; while [ ! -f {gate} ] && [ $attempt -lt 3000 ]; do sleep 0.01; attempt=$((attempt + 1)); done; if [ ! -f {gate} ]; then exit 99; fi; printf 'command-resolved\\n'; exit 7"
        )
    };
    let yield_ms = if matches!(resolution, Resolution::Background) {
        0
    } else {
        1
    };
    let args = json!({"cmd":cmd,"login":false,"yield_time_ms":yield_ms,"tty":matches!(resolution, Resolution::Tty)});
    let mock = responses::mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_response_created("r1"),
                ev_function_call("held-command", "exec_command", &args.to_string()),
                ev_completed("r1"),
            ]),
            sse(vec![
                ev_response_created("r2"),
                ev_assistant_message("done", "finished"),
                ev_completed("r2"),
            ]),
        ],
    )
    .await;
    let (sandbox_policy, permission_profile) =
        turn_permission_fields(PermissionProfile::Disabled, test.config.cwd.as_path());
    start_input(
        &test,
        "run the held command",
        ThreadSettingsOverrides {
            approval_policy: Some(AskForApproval::Never),
            sandbox_policy: Some(sandbox_policy),
            permission_profile,
            ..Default::default()
        },
    )
    .await?;
    if matches!(resolution, Resolution::Background) {
        wait_for_event(&test.codex, |event| {
            matches!(event, EventMsg::TurnComplete(_))
        })
        .await;
        assert_eq!(mock.requests().len(), 2);
        test.fs()
            .write_file(&gate, b"release".to_vec(), /*sandbox*/ None)
            .await?;
        wait_for_event(&test.codex, |event| {
            matches!(event, EventMsg::ExecCommandEnd(_))
        })
        .await;
        let output = mock.requests()[1]
            .function_call_output("held-command")
            .to_string();
        assert!(
            output.contains("Process running") || output.contains("session_id"),
            "background result keeps its control handle: {output}"
        );
        test.codex.shutdown_and_wait().await?;
        return Ok(());
    }
    // Output events remain live while inference is suspended.
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::ExecCommandOutputDelta(_))
    })
    .await;
    // Also cross the legacy Windows minimum yield window: a disabled or
    // incorrectly bypassed policy must fail the no-second-request assertion.
    tokio::time::sleep(Duration::from_millis(
        if test_target_os() == TestTargetOs::Windows {
            10_500
        } else {
            350
        },
    ))
    .await;
    assert_eq!(mock.requests().len(), 1);
    test.fs()
        .write_file(&gate, b"release".to_vec(), /*sandbox*/ None)
        .await?;
    let mut end_exit = None;
    let mut turn_complete = false;
    wait_for_event(&test.codex, |event| {
        match event {
            EventMsg::ExecCommandEnd(end) if end.call_id == "held-command" => {
                end_exit = Some(end.exit_code);
            }
            EventMsg::TurnComplete(_) => turn_complete = true,
            _ => {}
        }
        end_exit.is_some() && turn_complete
    })
    .await;
    assert_eq!(end_exit, Some(7));
    let requests = mock.requests();
    assert_eq!(requests.len(), 2);
    let output = requests[1].function_call_output("held-command").to_string();
    assert!(
        output.contains("command-started")
            && output.contains("command-resolved")
            && output.contains("Process exited with code 7"),
        "final command output and exit must reach the resumed model: {output}"
    );
    test.codex.shutdown_and_wait().await?;
    Ok(())
}

async fn start_input(
    test: &TestCodex,
    prompt: &str,
    settings: ThreadSettingsOverrides,
) -> Result<()> {
    test.codex
        .start_or_steer_turn(
            TurnInputRequest::user_input(vec![UserInput::Text {
                text: prompt.to_string(),
                text_elements: Vec::new(),
            }])
            .with_thread_settings(settings),
        )
        .await?;
    Ok(())
}
