mod support;
use mugiurushi::{
    execution::ObservationOutcome, model::*, recording::*, reported::ReportedTrace, review,
    workspace::*,
};
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    thread,
};
struct Sandbox(PathBuf);
impl Sandbox {
    fn new() -> Self {
        Self(
            new_run_directory(&std::env::temp_dir().join("eel-workspace-tests"))
                .unwrap()
                .0,
        )
    }
    fn case(&self) -> PathBuf {
        let p = self.0.join("case");
        initialize(&p, support::workspace(), "作成者", "合成の確認").unwrap();
        p
    }
    fn trial(&self) -> ReportedTrace {
        support::trace(&[ObservationOutcome::Pass])
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn apply(path: &std::path::Path, action: Change) -> WorkspaceView {
    change(
        path,
        ChangeRequest {
            expected_revision: load(path).unwrap().revision,
            actor: "合成の確認者".into(),
            reason: "試験".into(),
            action,
        },
    )
    .unwrap()
}
fn import(path: &std::path::Path, trace: &ReportedTrace, _: &str) -> WorkspaceView {
    support::import(path, trace.clone())
}
#[test]
fn adding_questions_and_changing_meaning_requires_new_answers() {
    let s = Sandbox::new();
    let case = s.case();
    import(&case, &s.trial(), "save");
    let v = apply(
        &case,
        Change::AddQuestion {
            check_id: "save".into(),
            question: Question {
                id: "scope".into(),
                text: "どの範囲か".into(),
                answer: None,
                origin: None,
            },
        },
    );
    assert_eq!(v.checks[0].verdict, Verdict::Unknown);
    assert_eq!(v.checks[0].unanswered.len(), 1);
    assert_eq!(v.checks[0].unanswered[0].text, "どの範囲か");
    apply(
        &case,
        Change::Answer {
            check_id: "save".into(),
            question_id: "scope".into(),
            answer: "ローカル試行のみ".into(),
            origin: "試験用の回答".into(),
        },
    );
    assert_eq!(
        import(&case, &s.trial(), "save").checks[0].verdict,
        Verdict::Satisfied
    );
    let v = apply(
        &case,
        Change::ReviseCheck {
            check_id: "save".into(),
            text: "翌日に閲覧できる".into(),
            origin: "試験用の別条件".into(),
            rule: Rule::ManualOnly,
            dependencies: vec![],
        },
    );
    assert_eq!(v.checks[0].verdict, Verdict::Unknown);
    assert!(v.workspace.checks[0].questions[0].answer.is_none());
    assert!(v.workspace.checks[0].questions[0].origin.is_none());
}

#[test]
fn general_case_starts_without_generated_domain_requirements() {
    let s = Sandbox::new();
    let case = s.0.join("general");
    let v = initialize(
        &case,
        empty_workspace("generic-001".into(), "通知を確認する".into()),
        "作成者",
        "汎用の事例を開始する",
    )
    .unwrap();
    assert!(v.checks.is_empty());
    assert!(v.workspace.statements.is_empty());
    assert!(v.workspace.context.is_empty());
    let v = apply(
        &case,
        Change::AddCheck {
            check: Check {
                source: None,
                observation: None,
                id: "notification".into(),
                version: 1,
                text: "通知が届く".into(),
                origin: "合成の利用条件".into(),
                rule: Rule::ManualOnly,
                dependencies: vec![],
                questions: vec![],
            },
        },
    );
    assert_eq!(v.checks[0].verdict, Verdict::Unknown);
    assert!(matches!(
        v.checks[0].next.kind,
        ActionKind::DefineObservation
    ));
}

fn http(
    path: &std::path::Path,
    method: &str,
    route: &str,
    body: &str,
    origin: Option<&str>,
    token: Option<&str>,
    host_override: Option<&str>,
) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let case = path.to_owned();
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        review::handle_connection(stream, &case, "test-token").unwrap();
    });
    let mut stream = TcpStream::connect(address).unwrap();
    let host = host_override
        .map(String::from)
        .unwrap_or_else(|| address.to_string());
    let origin = origin.map(|v| {
        if v == "same" {
            format!("http://{address}")
        } else {
            v.into()
        }
    });
    write!(stream,"{method} {route} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",body.len()).unwrap();
    if let Some(origin) = origin {
        write!(stream, "Origin: {origin}\r\n").unwrap();
    }
    if let Some(token) = token {
        write!(stream, "X-Review-Token: {token}\r\n").unwrap();
    }
    write!(stream, "\r\n{body}").unwrap();
    let mut result = String::new();
    stream.read_to_string(&mut result).unwrap();
    server.join().unwrap();
    result
}

#[test]
fn malformed_api_bodies_are_rejected_after_authentication_without_changing_the_case() {
    let sandbox = Sandbox::new();
    let case = sandbox.case();
    let before = serde_json::to_value(load(&case).unwrap()).unwrap();
    for route in ["change", "compare", "report-html", "understand"] {
        let route = format!("/api/v1/{route}");
        for (token, status) in [(None, 403), (Some("test-token"), 400)] {
            let response = http(&case, "POST", &route, "{", Some("same"), token, None);
            assert!(
                response.starts_with(&format!("HTTP/1.1 {status}")),
                "{route}: {response}"
            );
        }
    }
    assert_eq!(serde_json::to_value(load(&case).unwrap()).unwrap(), before);
}

#[test]
fn unknown_and_failed_checks_block_release_without_relabelling_the_observation() {
    let sandbox = Sandbox::new();
    let case = sandbox.case();
    let release = || {
        change(
            &case,
            ChangeRequest {
                expected_revision: load(&case).unwrap().revision,
                actor: "合成の確認者".into(),
                reason: "利用開始の確認".into(),
                action: Change::Decide {
                    role: Role::Operator,
                    action: DecisionKind::Release,
                    check_ids: vec!["save".into()],
                    change_reference: None,
                    confidence: None,
                    elapsed_ms: None,
                },
            },
        )
    };
    let empty = view(empty_workspace("empty".into(), "空の事例".into()));
    assert!(!empty.verification_passed);
    let unknown = view(load(&case).unwrap());
    assert_eq!(unknown.checks[0].verdict, Verdict::Unknown);
    assert!(!unknown.verification_passed);
    assert_eq!(
        release().unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    let failed = import(&case, &support::trace(&[ObservationOutcome::Fail]), "save");
    assert_eq!(failed.checks[0].verdict, Verdict::NotSatisfied);
    assert!(!failed.checks[0].verification_passed);
    assert!(release().is_err());
    let passed = import(&case, &support::trace(&[ObservationOutcome::Pass]), "save");
    assert!(passed.checks[0].verification_passed);
    assert!(
        !passed.verification_passed,
        "他の未確認条件を選択から外しても利用開始できない"
    );
    assert!(release().is_err());
    let report = mugiurushi::review_report::build(load(&case).unwrap()).unwrap();
    assert!(!report.verification_passed);
    assert!(
        mugiurushi::review_report::html(&report)
            .unwrap()
            .contains("検証未通過")
    );
    let passed_case = sandbox.0.join("passed");
    let mut workspace = support::workspace();
    workspace.checks.truncate(1);
    initialize(&passed_case, workspace, "合成の作成者", "一条件の確認").unwrap();
    let passed = import(
        &passed_case,
        &support::trace(&[ObservationOutcome::Pass]),
        "save",
    );
    assert!(passed.verification_passed);
    assert!(
        change(
            &passed_case,
            ChangeRequest {
                expected_revision: passed.workspace.revision,
                actor: "合成の確認者".into(),
                reason: "記録した全条件が検証を通過した".into(),
                action: Change::Decide {
                    role: Role::Operator,
                    action: DecisionKind::Release,
                    check_ids: vec!["save".into()],
                    change_reference: None,
                    confidence: None,
                    elapsed_ms: None
                },
            }
        )
        .is_ok()
    );
}

#[test]
fn review_reports_require_v1_auth_and_reject_a_stale_preview() {
    let s = Sandbox::new();
    let case = s.case();
    for route in ["/api/v1/report", "/api/v1/report-html"] {
        assert!(http(&case, "GET", route, "", None, None, None).starts_with("HTTP/1.1 403"));
        assert!(
            http(&case, "GET", route, "", None, Some("test-token"), None)
                .starts_with("HTTP/1.1 200")
        );
    }
    for route in ["/api/report", "/api/report-html"] {
        assert!(http(&case, "GET", route, "", None, None, None).starts_with("HTTP/1.1 410"));
    }
    assert!(
        http(
            &case,
            "POST",
            "/api/v1/report-html",
            "{\"expected_revision\":0}",
            None,
            Some("test-token"),
            None
        )
        .starts_with("HTTP/1.1 409")
    );
    assert!(
        http(
            &case,
            "POST",
            "/api/v1/report-html",
            "{\"expected_revision\":1}",
            None,
            Some("test-token"),
            None
        )
        .starts_with("HTTP/1.1 200")
    );
    assert!(
        http(
            &case,
            "POST",
            "/api/v1/report-html",
            "{\"expected_revision\":1}",
            Some("https://example.invalid"),
            Some("test-token"),
            None
        )
        .starts_with("HTTP/1.1 403")
    );
    let profiles = http(
        &case,
        "GET",
        "/api/v1/profiles",
        "",
        None,
        Some("test-token"),
        None,
    );
    assert!(profiles.contains("medical-access"));
    assert!(profiles.contains("catalog_version"));
    let module = http(&case, "GET", "/ui/review-tools.js", "", None, None, None);
    assert!(module.starts_with("HTTP/1.1 404"));
}

#[test]
fn review_http_rejects_cross_origin_missing_token_import_and_stale_update() {
    let s = Sandbox::new();
    let case = s.case();
    let body = serde_json::to_string(&ChangeRequest {
        expected_revision: 1,
        actor: "テスト".into(),
        reason: "画面の動作確認".into(),
        action: Change::SetContext {
            key: "configuration".into(),
            value: "v1".into(),
        },
    })
    .unwrap();
    for (origin, token) in [
        (Some("https://unrelated.invalid"), Some("test-token")),
        (Some("same"), None),
    ] {
        assert!(
            http(&case, "POST", "/api/v1/change", &body, origin, token, None)
                .starts_with("HTTP/1.1 403")
        );
    }
    assert!(
        http(
            &case,
            "GET",
            "/api/v1/case",
            "",
            None,
            None,
            Some("unrelated.invalid")
        )
        .starts_with("HTTP/1.1 403")
    );
    assert_eq!(load(&case).unwrap().revision, 1);
    assert!(
        http(
            &case,
            "POST",
            "/api/v1/change",
            &body,
            Some("same"),
            Some("test-token"),
            None
        )
        .starts_with("HTTP/1.1 200")
    );
    assert!(
        http(
            &case,
            "POST",
            "/api/v1/change",
            &body,
            Some("same"),
            Some("test-token"),
            None
        )
        .starts_with("HTTP/1.1 409")
    );
    let importing = serde_json::to_string(&ChangeRequest {
        expected_revision: 2,
        actor: "テスト".into(),
        reason: "任意ファイル取り込みの防止".into(),
        action: Change::Import {
            check_id: "save".into(),
            run_directory: "/not-readable-over-http".into(),
            use_run_context: true,
        },
    })
    .unwrap();
    assert!(
        http(
            &case,
            "POST",
            "/api/v1/change",
            &importing,
            Some("same"),
            Some("test-token"),
            None
        )
        .starts_with("HTTP/1.1 403")
    );
    assert_eq!(load(&case).unwrap().revision, 2);
    assert!(
        http(
            &case,
            "GET",
            "/api/v1/case",
            "",
            None,
            Some("test-token"),
            None
        )
        .contains("事例")
    );
}

fn code_reference(commit: char) -> CodeReference {
    CodeReference {
        repository: "example/project".into(),
        commit: commit.to_string().repeat(40),
        path: "src/storage.rs".into(),
        line: 2,
        end_line: 8,
    }
}

#[test]
fn bad_code_reference_and_stale_update_leave_case_unchanged() {
    let sandbox = Sandbox::new();
    let case = sandbox.case();
    let before = load(&case).unwrap().revision;
    let mut bad = code_reference('a');
    bad.path = "../secret".into();
    assert!(
        change(
            &case,
            ChangeRequest {
                expected_revision: before,
                actor: "合成試験".into(),
                reason: "不正な参照を確認".into(),
                action: Change::BindSource {
                    check_id: "save".into(),
                    source: bad
                },
            }
        )
        .is_err()
    );
    assert_eq!(load(&case).unwrap().revision, before);
    apply(
        &case,
        Change::BindSource {
            check_id: "save".into(),
            source: code_reference('a'),
        },
    );
    assert!(
        change(
            &case,
            ChangeRequest {
                expected_revision: before,
                actor: "合成試験".into(),
                reason: "古い画面からの更新".into(),
                action: Change::BindSource {
                    check_id: "save".into(),
                    source: code_reference('b')
                },
            }
        )
        .is_err()
    );
    assert_eq!(
        load(&case).unwrap().checks[0].source,
        Some(code_reference('a'))
    );
}

#[test]
fn old_workspace_records_load_without_code_reference() {
    let mut json = serde_json::to_value(support::workspace()).unwrap();
    for check in json["checks"].as_array_mut().unwrap() {
        check.as_object_mut().unwrap().remove("source");
    }
    let workspace: Workspace = serde_json::from_value(json).unwrap();
    validate(&workspace).unwrap();
    assert!(workspace.checks.iter().all(|c| c.source.is_none()));
}

#[test]
fn retired_api_aliases_never_return_case_backup_or_execution_data() {
    let sandbox = Sandbox::new();
    let case = sandbox.case();
    for route in ["case", "backup", "execution", "profiles", "change"] {
        for token in [None, Some("test-token")] {
            let response = http(
                &case,
                "GET",
                &format!("/api/{route}"),
                "",
                None,
                token,
                None,
            );
            assert!(response.starts_with("HTTP/1.1 410"), "{response}");
            assert!(!response.contains("\"workspace\""));
            assert!(!response.contains("\"revisions\""));
        }
    }
    let response = http(
        &case,
        "POST",
        "/api/v1/repository",
        "{}",
        Some("same"),
        Some("test-token"),
        None,
    );
    assert!(response.starts_with("HTTP/1.1 410"));
    let forged = r#"{"operation":"analyze","repository":"example/project","commit":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","snapshot":{"files":{"app.js":"forged"}}}"#;
    let response = http(
        &case,
        "POST",
        "/api/v1/understand",
        forged,
        Some("same"),
        Some("test-token"),
        None,
    );
    assert!(response.starts_with("HTTP/1.1 400"), "{response}");
}
