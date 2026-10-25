mod support;
use mugiurushi::{execution::*, model::*, recording::*, workspace::*};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use support::{execute, run_external};
struct Fixture {
    directory: PathBuf,
    root: PathBuf,
    commit: String,
}
fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}
impl Fixture {
    fn new() -> Self {
        let directory = new_run_directory(&std::env::temp_dir().join("evidence-execution-tests"))
            .unwrap()
            .0;
        let root = directory.join("repo");
        fs::create_dir(&root).unwrap();
        git(&root, &["init", "-q"]);
        git(&root, &["config", "user.name", "Synthetic test"]);
        git(&root, &["config", "user.email", "test@example.invalid"]);
        git(
            &root,
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/example/project.git",
            ],
        );
        fs::create_dir_all(root.join("examples/observable-store")).unwrap();
        fs::create_dir_all(root.join("adapters/observation")).unwrap();
        fs::write(
            root.join("adapters/observation/observe.mjs"),
            include_str!("../adapters/observation/observe.mjs"),
        )
        .unwrap();
        fs::write(
            root.join("adapters/observation/observe.py"),
            include_str!("../adapters/observation/observe.py"),
        )
        .unwrap();
        fs::write(
            root.join("examples/observable-store/app.mjs"),
            include_str!("../examples/observable-store/app.mjs"),
        )
        .unwrap();
        fs::write(
            root.join("examples/observable-store/app.py"),
            include_str!("../examples/observable-store/app.py"),
        )
        .unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-qm", "synthetic app"]);
        let commit = git(&root, &["rev-parse", "HEAD"]);
        Self {
            directory,
            root,
            commit,
        }
    }
    fn recipe(&self, runtime: Runtime, scenario: &str) -> Recipe {
        let path = if runtime == Runtime::Node {
            "examples/observable-store/app.mjs"
        } else {
            "examples/observable-store/app.py"
        };
        Recipe {
            schema_version: 1,
            check_id: "stored".into(),
            source: CodeReference {
                repository: "example/project".into(),
                commit: self.commit.clone(),
                path: path.into(),
                line: 1,
                end_line: 1,
            },
            runtime,
            entrypoint: path.into(),
            args: vec![scenario.into()],
            input: serde_json::json!({"message":"架空データ"}),
            criterion: Criterion {
                assertion: "stored_matches_input".into(),
                subject: "record-1".into(),
            },
            timeout_ms: 1500,
            trial: None,
            launch: Default::default(),
            python: None,
            server: None,
            containers: None,
            http: None,
            browser: None,
        }
    }
    fn run(&self, runtime: Runtime, scenario: &str) -> RunResult {
        execute(
            &self.root,
            &self.directory.join("runs"),
            self.recipe(runtime, scenario),
            self.condition(),
            None,
        )
        .unwrap()
    }
    fn condition(&self) -> Condition {
        Condition {
            id: "stored".into(),
            version: 1,
            text: "試行用の保存物を読み出し、入力と一致する".into(),
            origin: "合成の接続試験".into(),
            kind: ConditionKind::Custom,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

#[test]
fn both_languages_run_pinned_code_and_distinguish_failure_missing_and_conflict() {
    let fixture = Fixture::new();
    for runtime in [Runtime::Node, Runtime::Python3] {
        for (scenario, expected) in [
            ("normal", Verdict::Satisfied),
            ("failure", Verdict::NotSatisfied),
            ("missing", Verdict::Unknown),
            ("conflict", Verdict::Unknown),
        ] {
            let result = fixture.run(runtime.clone(), scenario);
            assert_eq!(
                result.assessment.verdict, expected,
                "{scenario}: {}",
                result.assessment.reason
            );
            let trace: ExecutionTrace =
                read_json(&result.directory.join("execution.json")).unwrap();
            assert_eq!(trace.recipe.source.commit, fixture.commit);
            assert!(trace.snapshot_unchanged);
            assert_eq!(trace.exit_code, Some(0));
            assert!(result.directory.join("complete.json").exists());
        }
    }
}

#[test]
fn zero_exit_foreign_trace_and_timeout_never_supply_positive_evidence() {
    let fixture = Fixture::new();
    for scenario in [
        "exit-zero",
        "foreign",
        "bad-line",
        "sequence-gap",
        "mutate",
        "too-much-output",
    ] {
        let result = fixture.run(Runtime::Node, scenario);
        assert_eq!(result.assessment.verdict, Verdict::Unknown);
    }
    let mut recipe = fixture.recipe(Runtime::Node, "timeout");
    recipe.timeout_ms = 80;
    let result = execute(
        &fixture.root,
        &fixture.directory.join("runs"),
        recipe,
        fixture.condition(),
        None,
    )
    .unwrap();
    assert_eq!(result.assessment.verdict, Verdict::Unknown);
    let trace: ExecutionTrace = read_json(&result.directory.join("execution.json")).unwrap();
    assert!(trace.timed_out);
}

#[test]
fn dirty_worktree_is_not_executed_and_wrong_origin_and_symlinks_are_rejected() {
    let fixture = Fixture::new();
    fs::write(
        fixture.root.join("examples/observable-store/app.mjs"),
        "throw Error('dirty code must not run')",
    )
    .unwrap();
    assert_eq!(
        fixture.run(Runtime::Node, "normal").assessment.verdict,
        Verdict::Satisfied
    );
    git(
        &fixture.root,
        &[
            "remote",
            "set-url",
            "origin",
            "https://github.com/other/project.git",
        ],
    );
    assert!(
        execute(
            &fixture.root,
            &fixture.directory.join("runs"),
            fixture.recipe(Runtime::Node, "normal"),
            fixture.condition(),
            None
        )
        .is_err()
    );
    git(
        &fixture.root,
        &[
            "remote",
            "set-url",
            "origin",
            "https://github.com/example/project.git",
        ],
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/etc/passwd", fixture.root.join("outside")).unwrap();
        git(&fixture.root, &["add", "outside"]);
        git(&fixture.root, &["commit", "-qm", "symlink"]);
        let mut recipe = fixture.recipe(Runtime::Node, "normal");
        recipe.source.commit = git(&fixture.root, &["rev-parse", "HEAD"]);
        assert!(
            execute(
                &fixture.root,
                &fixture.directory.join("runs"),
                recipe,
                fixture.condition(),
                None
            )
            .is_err()
        );
    }
}

#[test]
fn external_trial_returns_to_its_code_and_retest_preserves_old_evidence() {
    let fixture = Fixture::new();
    let case = fixture.directory.join("case");
    let recipe = fixture.recipe(Runtime::Node, "failure");
    let mut workspace = empty_workspace("external".into(), "外部コードの合成試験".into());
    workspace.checks.push(Check {
        id: "stored".into(),
        version: 1,
        text: fixture.condition().text,
        origin: "合成の仕様".into(),
        source: Some(recipe.source.clone()),
        observation: None,
        rule: Rule::ManualOnly,
        dependencies: vec![],
        questions: vec![],
    });
    initialize(&case, workspace, "Synthetic", "外部試験を始める").unwrap();
    change(
        &case,
        ChangeRequest {
            expected_revision: 1,
            actor: "Synthetic".into(),
            reason: "観測名と対象を対応づける".into(),
            action: Change::ConfigureObservation {
                check_id: "stored".into(),
                criterion: recipe.criterion.clone(),
            },
        },
    )
    .unwrap();
    let failure = run_external(
        &case,
        &fixture.root,
        recipe.clone(),
        "Synthetic".into(),
        "保存前の人工失敗".into(),
    )
    .unwrap();
    assert_eq!(failure.checks[0].verdict, Verdict::NotSatisfied);
    let mut normal = recipe;
    normal.args = vec!["normal".into()];
    let success = run_external(
        &case,
        &fixture.root,
        normal,
        "Synthetic".into(),
        "正常に戻して再試験".into(),
    )
    .unwrap();
    assert_eq!(success.checks[0].verdict, Verdict::Satisfied);
    assert_eq!(success.workspace.evidence.len(), 2);
    assert!(
        success.workspace.evidence[1]
            .manifest
            .previous_run
            .is_some()
    );
    assert_eq!(
        success.workspace.evidence[0]
            .execution
            .as_ref()
            .unwrap()
            .recipe
            .source
            .commit,
        fixture.commit
    );
    assert!(!success.checks[0].evidence[0].applicable); // 起動引数が変わった結果を現在へ補わない。
    let mut changed = success.workspace.evidence[1]
        .execution
        .as_ref()
        .unwrap()
        .recipe
        .source
        .clone();
    changed.commit = "b".repeat(40);
    let stale = change(
        &case,
        ChangeRequest {
            expected_revision: success.workspace.revision,
            actor: "Synthetic".into(),
            reason: "別のコード版を参照する".into(),
            action: Change::BindSource {
                check_id: "stored".into(),
                source: changed,
            },
        },
    )
    .unwrap();
    assert_eq!(stale.checks[0].verdict, Verdict::Unknown);
}

#[cfg(target_os = "linux")]
#[test]
fn sandbox_unavailability_never_falls_back_to_local_execution() {
    let fixture = Fixture::new();
    let result = execute_with_mode(
        &fixture.root,
        &fixture.directory.join("runs"),
        fixture.recipe(Runtime::Node, "normal"),
        fixture.condition(),
        None,
        ExecutionMode::Bubblewrap,
    );
    match result {
        Ok(result) => {
            let trace: ExecutionTrace =
                read_json(&result.directory.join("execution.json")).unwrap();
            assert_eq!(trace.mode, "bubblewrap");
            if trace.exit_code == Some(0) {
                assert_eq!(result.assessment.verdict, Verdict::Satisfied);
            } else {
                assert_eq!(result.assessment.verdict, Verdict::Unknown);
                assert!(!result.directory.join("work/stored.json").exists());
            }
            eprintln!(
                "bubblewrap: mode={}, exit={:?}, verdict={:?}",
                trace.mode, trace.exit_code, result.assessment.verdict
            );
        }
        Err(error) => assert!(error.to_string().contains("bwrap")),
    }
}

#[test]
fn next_confirmation_distinguishes_execution_failure_missing_and_invalid_observations() {
    let fixture = Fixture::new();
    for (scenario, kind) in [
        ("missing", ActionKind::AddObservation),
        ("foreign", ActionKind::InspectConflict),
        ("conflict", ActionKind::InspectConflict),
        ("timeout", ActionKind::InspectFailure),
    ] {
        let case = fixture.directory.join(format!("next-{scenario}"));
        let mut recipe = fixture.recipe(Runtime::Node, scenario);
        if scenario == "timeout" {
            recipe.timeout_ms = 80;
        }
        let mut workspace =
            empty_workspace(format!("case-{scenario}"), "次の確認を選ぶ合成事例".into());
        workspace.checks.push(Check {
            id: "stored".into(),
            version: 1,
            text: fixture.condition().text,
            origin: "合成の仕様".into(),
            source: Some(recipe.source.clone()),
            observation: None,
            rule: Rule::ManualOnly,
            dependencies: vec![],
            questions: vec![],
        });
        initialize(&case, workspace, "Synthetic", "初期化").unwrap();
        change(
            &case,
            ChangeRequest {
                expected_revision: 1,
                actor: "Synthetic".into(),
                reason: "観測規則を対応づける".into(),
                action: Change::ConfigureObservation {
                    check_id: "stored".into(),
                    criterion: recipe.criterion.clone(),
                },
            },
        )
        .unwrap();
        let result = run_external(
            &case,
            &fixture.root,
            recipe,
            "Synthetic".into(),
            scenario.into(),
        )
        .unwrap();
        assert_eq!(result.checks[0].verdict, Verdict::Unknown);
        assert_eq!(
            std::mem::discriminant(&result.checks[0].next.kind),
            std::mem::discriminant(&kind),
            "{scenario}"
        );
        if scenario == "missing" {
            assert!(result.checks[0].next.text.contains("stored_matches_input"));
        }
        if scenario == "timeout" {
            assert!(result.checks[0].next.text.contains("起動ファイル"));
        }
    }
}

#[test]
fn prepare_never_replaces_existing_repo_and_bad_size_metadata_is_rejected() {
    let fixture = Fixture::new();
    let recipe = fixture.recipe(Runtime::Node, "normal");
    assert!(prepare_repository(&fixture.root, &recipe).is_err());
    assert_eq!(git(&fixture.root, &["rev-parse", "HEAD"]), fixture.commit);
    let run = fixture.run(Runtime::Node, "normal");
    let mut trace: ExecutionTrace = read_json(&run.directory.join("execution.json")).unwrap();
    trace.files[1].bytes = u64::MAX;
    assert!(validate_trace(&trace, &run.manifest).is_err());
    let mut bad = recipe;
    bad.source.repository = "other/../project".into();
    let destination = fixture.directory.join("invalid-prepare");
    assert!(prepare_repository(&destination, &bad).is_err());
    assert!(!destination.exists());
}

fn review_case(fixture: &Fixture) -> PathBuf {
    let case = fixture.directory.join("review-case");
    let recipe = fixture.recipe(Runtime::Node, "normal");
    let mut spec = empty_workspace("review-execution".into(), "合成の画面実行".into());
    spec.checks.push(Check {
        id: "stored".into(),
        version: 1,
        text: fixture.condition().text,
        origin: "合成の仕様".into(),
        source: Some(recipe.source),
        observation: Some(recipe.criterion),
        rule: Rule::ObservedAssertion,
        dependencies: vec!["execution-stored".into()],
        questions: vec![],
    });
    initialize(&case, spec, "Synthetic", "合成の試験を設定する").unwrap();
    case
}
fn execute_request(
    fixture: &Fixture,
    revision: u64,
    scenario: &str,
) -> mugiurushi::review_execution::ExecuteRequest {
    mugiurushi::review_execution::ExecuteRequest {
        expected_revision: revision,
        actor: "Synthetic".into(),
        reason: "画面経由の合成試験".into(),
        recipe: fixture.recipe(Runtime::Node, scenario),
    }
}
fn wait_job(
    controller: &mugiurushi::review_execution::Controller,
) -> mugiurushi::review_execution::Job {
    let start = std::time::Instant::now();
    loop {
        let job = controller.status().unwrap().job.unwrap();
        if job.status != "running" {
            return job;
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(10));
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
fn review_http(
    case: &Path,
    controller: Option<&mugiurushi::review_execution::Controller>,
    method: &str,
    path: &str,
    body: &str,
    authenticated: bool,
) -> (u16, serde_json::Value) {
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        thread,
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let case = case.to_owned();
    let controller = controller.cloned();
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        if let Some(controller) = controller {
            mugiurushi::review::handle_connection_with_execution(
                stream,
                &case,
                "test-token",
                &controller,
            )
            .unwrap();
        } else {
            mugiurushi::review::handle_connection(stream, &case, "test-token").unwrap();
        }
    });
    let mut stream = TcpStream::connect(address).unwrap();
    write!(stream, "{method} {path} HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n", body.len()).unwrap();
    if authenticated {
        write!(
            stream,
            "Origin: http://{address}\r\nX-Review-Token: test-token\r\n"
        )
        .unwrap();
    }
    write!(stream, "\r\n{body}").unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    server.join().unwrap();
    let (header, body) = response.split_once("\r\n\r\n").unwrap();
    (
        header.split_whitespace().nth(1).unwrap().parse().unwrap(),
        serde_json::from_str(body).unwrap(),
    )
}

#[test]
fn review_execution_is_opt_in_authenticated_and_cannot_replace_root_or_mode() {
    use mugiurushi::review_execution::Controller;
    let fixture = Fixture::new();
    let case = review_case(&fixture);
    let controller =
        Controller::new(case.clone(), fixture.root.clone(), ExecutionMode::Local).unwrap();
    let value = serde_json::json!({"expected_revision":1,"actor":"Synthetic","reason":"合成の画面試験","recipe":fixture.recipe(Runtime::Node,"normal")});
    assert_eq!(
        review_http(
            &case,
            None,
            "POST",
            "/api/execute",
            &value.to_string(),
            true
        )
        .0,
        403
    );
    assert_eq!(
        review_http(&case, None, "GET", "/api/execution", "", false).1["enabled"],
        false
    );
    assert_eq!(
        review_http(
            &case,
            Some(&controller),
            "POST",
            "/api/execute",
            &value.to_string(),
            false
        )
        .0,
        403
    );
    for key in ["repository_root", "mode", "program"] {
        let mut modified = value.clone();
        modified[key] = "arbitrary".into();
        assert_eq!(
            review_http(
                &case,
                Some(&controller),
                "POST",
                "/api/execute",
                &modified.to_string(),
                true
            )
            .0,
            400
        );
    }
    let mut stale = value.clone();
    stale["expected_revision"] = 0.into();
    assert_eq!(
        review_http(
            &case,
            Some(&controller),
            "POST",
            "/api/execute",
            &stale.to_string(),
            true
        )
        .0,
        403
    );
    let mut foreign = value.clone();
    foreign["recipe"]["source"]["repository"] = "other/project".into();
    assert_eq!(
        review_http(
            &case,
            Some(&controller),
            "POST",
            "/api/execute",
            &foreign.to_string(),
            true
        )
        .0,
        403
    );
    assert!(controller.status().unwrap().job.is_none());
    let (status, _) = review_http(
        &case,
        Some(&controller),
        "POST",
        "/api/execute",
        &value.to_string(),
        true,
    );
    assert_eq!(status, 403);
    assert!(controller.status().unwrap().job.is_none());
    // 端末側の明示的な開発用実行はHTTPの権限とは別。
    controller
        .start(serde_json::from_value(value).unwrap())
        .unwrap();
    assert_eq!(wait_job(&controller).status, "completed");
    let view = view(load(&case).unwrap());
    assert_eq!(view.checks[0].verdict, Verdict::Satisfied);
    assert_eq!(view.workspace.evidence.len(), 1);
}

#[test]
fn web_execution_requires_exact_single_use_terminal_approval_and_never_accepts_local() {
    use mugiurushi::review_execution::{Controller, ExecuteRequest, approve};
    let fixture = Fixture::new();
    let case = review_case(&fixture);
    let unused = fixture.root.join("forbidden-app-data");
    assert_eq!(
        mugiurushi::review::serve_app(unused.clone(), 0, ExecutionMode::Local)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert!(
        !unused.exists(),
        "Local指定はデータや待受を作る前に拒否する"
    );
    assert_eq!(
        mugiurushi::review::serve_api(
            case.clone(),
            0,
            unused.join("token"),
            Some((fixture.root.clone(), ExecutionMode::Local))
        )
        .unwrap_err()
        .kind(),
        std::io::ErrorKind::PermissionDenied
    );
    let local = Controller::new(case.clone(), fixture.root.clone(), ExecutionMode::Local).unwrap();
    let isolated = Controller::new(
        case.clone(),
        fixture.root.clone(),
        ExecutionMode::Bubblewrap,
    )
    .unwrap();
    let managed = Controller::managed(
        case.clone(),
        fixture.root.join("managed"),
        ExecutionMode::Bubblewrap,
    )
    .unwrap();
    let value = serde_json::json!({"expected_revision":1,"actor":"Synthetic","reason":"端末で内容を確認した試験","recipe":fixture.recipe(Runtime::Node,"normal")});
    for route in ["/api/execute", "/api/v1/execute"] {
        for controller in [&local, &isolated, &managed] {
            assert_eq!(
                review_http(
                    &case,
                    Some(controller),
                    "POST",
                    route,
                    &value.to_string(),
                    true
                )
                .0,
                403
            );
            assert!(controller.status().unwrap().job.is_none());
        }
    }
    let request: ExecuteRequest = serde_json::from_value(value.clone()).unwrap();
    approve(&case, &request).unwrap();
    assert!(approve(&case, &request).is_err(), "承認を上書きしない");
    for (pointer, change) in [
        ("/recipe/args", serde_json::json!(["unapproved"])),
        ("/recipe/input", serde_json::json!({"command":"unapproved"})),
        ("/recipe/entrypoint", serde_json::json!("other.js")),
        ("/expected_revision", serde_json::json!(2)),
        ("/actor", serde_json::json!("other")),
        ("/reason", serde_json::json!("other")),
    ] {
        let mut changed = value.clone();
        *changed.pointer_mut(pointer).unwrap() = change;
        assert_eq!(
            review_http(
                &case,
                Some(&isolated),
                "POST",
                "/api/v1/execute",
                &changed.to_string(),
                true
            )
            .0,
            403
        );
        assert!(isolated.status().unwrap().job.is_none());
    }
    for (key, nested) in [
        (
            "trial",
            serde_json::json!({"schema_version":1,"fault":"normal","storage":"notes","readback":{"entrypoint":"other.js"},"checks":[]}),
        ),
        (
            "python",
            serde_json::json!({"dependencies":[{"repository":"other/dependency","commit":"a".repeat(40)}]}),
        ),
        (
            "server",
            serde_json::json!({"entrypoint":"other.js","args":["{{port}}"],"ready_timeout_ms":1000}),
        ),
        (
            "containers",
            serde_json::json!({"services":[{"name":"app","image":format!("example@sha256:{}", "a".repeat(64)),"role":"app","expose":80,"env":{"UNAPPROVED":"yes"},"command":["other"],"ready":"tcp","ready_timeout_ms":1000}]}),
        ),
    ] {
        let mut changed = value.clone();
        changed["recipe"][key] = nested;
        assert_eq!(
            review_http(
                &case,
                Some(&isolated),
                "POST",
                "/api/execute",
                &changed.to_string(),
                true
            )
            .0,
            403,
            "{key}"
        );
        assert!(isolated.status().unwrap().job.is_none());
    }
    assert_eq!(
        review_http(
            &case,
            Some(&local),
            "POST",
            "/api/execute",
            &value.to_string(),
            true
        )
        .0,
        403
    );
    let mut statuses = std::thread::scope(|scope| {
        let (case, isolated, value) = (&case, &isolated, &value);
        let handles = ["/api/execute", "/api/v1/execute"].map(|route| {
            scope.spawn(move || {
                review_http(
                    case,
                    Some(isolated),
                    "POST",
                    route,
                    &value.to_string(),
                    true,
                )
                .0
            })
        });
        handles.map(|h| h.join().unwrap())
    });
    statuses.sort();
    assert_eq!(statuses, [202, 403], "並行要求も一回だけ受け付ける");
    // bwrapがない環境でもLocalへ切り替えない。承認の再利用は環境にかかわらず拒否する。
    wait_job(&isolated);
    assert_eq!(
        review_http(
            &case,
            Some(&isolated),
            "POST",
            "/api/execute",
            &value.to_string(),
            true
        )
        .0,
        403
    );
}

#[test]
fn background_review_keeps_reads_available_rejects_duplicates_and_preserves_conflicting_run() {
    use mugiurushi::review_execution::Controller;
    let fixture = Fixture::new();
    let case = review_case(&fixture);
    let controller =
        Controller::new(case.clone(), fixture.root.clone(), ExecutionMode::Local).unwrap();
    let mut request = execute_request(&fixture, 1, "timeout");
    request.recipe.timeout_ms = 800;
    controller.start(request).unwrap();
    assert_eq!(
        controller
            .start(execute_request(&fixture, 1, "normal"))
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::WouldBlock
    );
    let (status, current) = review_http(&case, Some(&controller), "GET", "/api/case", "", false);
    assert_eq!(status, 200);
    assert_eq!(current["workspace"]["revision"], 1);
    // コピーや起動が終わる前でも、実行終了前の変更は取り込み時に照合する。
    let start = std::time::Instant::now();
    while !case.join("runs").exists() {
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    change(
        &case,
        ChangeRequest {
            expected_revision: 1,
            actor: "Synthetic".into(),
            reason: "実行中の条件変更".into(),
            action: Change::SetContext {
                key: "configuration".into(),
                value: "revised".into(),
            },
        },
    )
    .unwrap();
    let job = wait_job(&controller);
    assert_eq!(job.status, "failed");
    assert!(job.error.unwrap().contains("試行記録"));
    assert!(load(&case).unwrap().evidence.is_empty());
    assert!(
        fs::read_dir(case.join("runs")).unwrap().any(|entry| entry
            .unwrap()
            .path()
            .join("complete.json")
            .exists())
    );
    controller
        .start(execute_request(&fixture, 2, "normal"))
        .unwrap();
    assert_eq!(wait_job(&controller).status, "completed");
    assert_eq!(
        view(load(&case).unwrap()).checks[0].verdict,
        Verdict::Satisfied
    );
}

#[test]
fn pinned_review_reports_target_failure_and_timeout_as_unknown_without_running_other_origin() {
    use mugiurushi::review_execution::Controller;
    let fixture = Fixture::new();
    let case = review_case(&fixture);
    let controller =
        Controller::new(case.clone(), fixture.root.clone(), ExecutionMode::Local).unwrap();
    let mut request = execute_request(&fixture, 1, "timeout");
    request.recipe.timeout_ms = 60;
    controller.start(request).unwrap();
    assert_eq!(wait_job(&controller).status, "completed");
    assert_eq!(
        view(load(&case).unwrap()).checks[0].verdict,
        Verdict::Unknown
    );
    git(
        &fixture.root,
        &[
            "remote",
            "set-url",
            "origin",
            "https://github.com/other/project.git",
        ],
    );
    controller
        .start(execute_request(&fixture, 2, "normal"))
        .unwrap();
    assert_eq!(wait_job(&controller).status, "failed");
    assert_eq!(load(&case).unwrap().evidence.len(), 1);
}
