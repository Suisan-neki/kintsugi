//! 依存先と複数のプロセスを含む試行を、架空の受信サーバー・送信側・依存パッケージで確かめる。
//! 送信側は pynetdicom の storescu と同じく、受信側が失敗を返しても終了コード0で終わる。
use mugiurushi::{
    execution::*, model::*, python_env::*, recording::*, server::*, trial::*, workspace::*,
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const STOREKIT_PYPROJECT: &str = r#"[project]
name = "storekit"
version = "1.2.0"
dependencies = []

[project.scripts]
storekit-show = "storekit.cli:main"
"#;
const STOREKIT_INIT: &str = r#"from importlib.metadata import version
__version__ = version("storekit")
"#;
const STOREKIT_CLI: &str = r#"import sys
import storekit
def main():
    with open(sys.argv[1]) as f:
        print("storekit " + storekit.__version__ + ": " + f.read().strip())
    return 0
"#;
const RECEIVER_PYPROJECT: &str = r#"[project]
name = "receiver"
version = "0.1.0"
dependencies = ["storekit >=1, <2"]
"#;
const RECEIVER_MAIN: &str = r#"import os, socket, sys, time
import storekit

def serve(port, directory):
    server = socket.create_server(("127.0.0.1", port))
    while True:
        conn, _ = server.accept()
        with conn:
            data = conn.recv(1024).decode().strip()
            if not data:
                continue
            try:
                os.makedirs(directory, exist_ok=True)
                with open(os.path.join(directory, "received.txt"), "w") as f:
                    f.write(data + "\n")
                conn.sendall(b"0000\n")
            except OSError as e:
                sys.stderr.write("E: could not store: " + str(e) + "\n")
                sys.stderr.flush()
                conn.sendall(b"A700\n")

def send(port, value):
    with socket.create_connection(("127.0.0.1", port)) as conn:
        conn.sendall((value + "\n").encode())
        reply = conn.recv(1024).decode().strip()
    print("I: status " + reply)
    return 0

command = sys.argv[1]
if command == "serve":
    serve(int(sys.argv[2]), sys.argv[3])
elif command == "send":
    sys.exit(send(int(sys.argv[2]), sys.argv[3]))
elif command == "hang":
    time.sleep(60)
"#;

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .expect("gitを起動できる");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("gitの出力はUTF-8")
        .trim()
        .into()
}
fn repository(root: &Path, origin: &str, files: &[(&str, &str)]) -> String {
    fs::create_dir_all(root).expect("リポジトリ");
    git(root, &["init", "-q"]);
    git(root, &["config", "user.name", "Synthetic test"]);
    git(root, &["config", "user.email", "test@example.invalid"]);
    git(root, &["remote", "add", "origin", origin]);
    for (path, text) in files {
        let path = root.join(path);
        fs::create_dir_all(path.parent().expect("親")).expect("ディレクトリ");
        fs::write(path, text).expect("ファイル");
    }
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "synthetic"]);
    git(root, &["rev-parse", "HEAD"])
}

struct Fixture {
    directory: PathBuf,
    target: PathBuf,
    base: PathBuf,
    target_commit: String,
    dependency_commit: String,
}
impl Fixture {
    fn new() -> Self {
        let directory = new_run_directory(&std::env::temp_dir().join("evidence-multi-process"))
            .expect("試験用ディレクトリ")
            .0;
        let base = directory.join("fetched");
        let dependency_commit = repository(
            &base.join("example/storekit"),
            "https://github.com/example/storekit.git",
            &[
                ("pyproject.toml", STOREKIT_PYPROJECT),
                ("src/storekit/__init__.py", STOREKIT_INIT),
                ("src/storekit/cli.py", STOREKIT_CLI),
                ("tests/test_cli.py", "def test_nothing():\n    pass\n"),
            ],
        );
        let target = directory.join("receiver");
        let target_commit = repository(
            &target,
            "https://github.com/example/receiver.git",
            &[
                ("pyproject.toml", RECEIVER_PYPROJECT),
                ("receiver/__init__.py", ""),
                ("receiver/__main__.py", RECEIVER_MAIN),
            ],
        );
        Self {
            directory,
            target,
            base,
            target_commit,
            dependency_commit,
        }
    }
    fn recipe(&self, server_args: &[&str], spec: TrialSpec) -> Recipe {
        Recipe {
            schema_version: 1,
            check_id: "receive".into(),
            source: CodeReference {
                repository: "example/receiver".into(),
                commit: self.target_commit.clone(),
                path: "receiver/__main__.py".into(),
                line: 1,
                end_line: 1,
            },
            runtime: Runtime::Python3,
            entrypoint: "receiver".into(),
            args: vec!["send".into(), PORT.into(), TEST_VALUE.into()],
            input: serde_json::json!({}),
            criterion: Criterion {
                assertion: "eel.trial".into(),
                subject: "receive".into(),
            },
            timeout_ms: 10000,
            trial: Some(spec),
            launch: Launch::Module,
            python: Some(PythonSetup {
                root: None,
                dependencies: vec![Dependency {
                    repository: "example/storekit".into(),
                    commit: self.dependency_commit.clone(),
                    root: Some("src".into()),
                }],
            }),
            server: Some(ServerSpec {
                launch: Launch::Module,
                entrypoint: "receiver".into(),
                args: server_args.iter().map(|a| (*a).into()).collect(),
                ready_timeout_ms: 5000,
            }),
            containers: None,
            http: None,
            browser: None,
        }
    }
    fn run(&self, recipe: Recipe) -> io::Result<RunResult> {
        execute_with_dependencies(
            &self.target,
            Some(&self.base),
            &self.directory.join("runs"),
            recipe,
            Condition {
                id: "receive".into(),
                version: 1,
                text: "送った架空の値を受信側が保存し、後から読み出せる".into(),
                origin: "試験".into(),
                kind: ConditionKind::Custom,
            },
            None,
            ExecutionMode::Local,
        )
    }
}
use std::io;
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn check(
    id: &str,
    scope: &str,
    method: CheckMethod,
    expect: Expectation,
    text: Option<&str>,
) -> TrialCheck {
    TrialCheck {
        id: id.into(),
        scope: scope.into(),
        method,
        expect,
        text: text.map(Into::into),
        target: None,
    }
}
const SERVE: &[&str] = &["serve", PORT, "received"];
fn normal() -> TrialSpec {
    TrialSpec {
        schema_version: 1,
        recovery: None,
        fault: TrialFault::Normal,
        storage: Some("received/received.txt".into()),
        readback: Some(Readback {
            launch: Launch::Script,
            entrypoint: "storekit-show".into(),
            args: vec!["received/received.txt".into()],
        }),
        checks: vec![
            check(
                "accepted",
                "受付",
                CheckMethod::Stdout,
                Expectation::Contains,
                Some("status 0000"),
            ),
            check(
                "written",
                "書き込み",
                CheckMethod::StoredFile,
                Expectation::Contains,
                Some(TEST_VALUE),
            ),
            check(
                "readback",
                "後からの読み出し",
                CheckMethod::ReadbackOutput,
                Expectation::Contains,
                Some(TEST_VALUE),
            ),
            check(
                "no-error",
                "受信側のエラー",
                CheckMethod::ServerOutput,
                Expectation::NotContains,
                Some("E:"),
            ),
        ],
    }
}
fn failure_reported(fault: TrialFault) -> TrialSpec {
    TrialSpec {
        schema_version: 1,
        recovery: None,
        fault,
        storage: Some("received/received.txt".into()),
        readback: None,
        checks: vec![
            check(
                "exit",
                "失敗の通知",
                CheckMethod::ExitCode,
                Expectation::Nonzero,
                None,
            ),
            check(
                "not-written",
                "書き込み",
                CheckMethod::StoredFile,
                Expectation::NotContains,
                Some(TEST_VALUE),
            ),
        ],
    }
}
fn trace(result: &RunResult) -> ExecutionTrace {
    serde_json::from_slice(
        &fs::read(result.directory.join("execution.json")).expect("execution.json"),
    )
    .expect("記録を読める")
}

#[test]
fn receiver_and_sender_run_with_a_pinned_dependency() {
    let fixture = Fixture::new();
    let result = fixture
        .run(fixture.recipe(SERVE, normal()))
        .expect("試行を実行できる");
    assert_eq!(
        result.assessment.verdict,
        Verdict::Satisfied,
        "{:?}",
        result.assessment
    );
    let trace = trace(&result);
    let python = trace.python.expect("Pythonの準備の記録");
    assert_eq!(python.flags, ["-S", "-P"]);
    assert_eq!(python.import_path, ["source", "deps/0/src", "python-meta"]);
    let dependency = &python.packages[1];
    assert_eq!(dependency.commit, fixture.dependency_commit);
    assert_eq!(
        dependency.files, 2,
        "読み込み先（src）の外の tests と pyproject.toml はコピーしない"
    );
    assert!(
        dependency.pyproject_blob.is_some(),
        "pyproject.toml はGitから読む"
    );
    assert_eq!(
        dependency.metadata.as_deref(),
        Some("storekit-1.2.0.dist-info")
    );
    let readback = python
        .launches
        .iter()
        .find(|l| l.role == "readback")
        .expect("読み出しの解決");
    assert_eq!(readback.path, "src/storekit/cli.py");
    let outcome = trace.trial.expect("試験の結果");
    let server = outcome.server.expect("待つプロセスの記録");
    assert!(server.ready && server.stopped_by_platform && !server.exited_before_stop);
    assert!(server.command.iter().any(|a| a == &server.port.to_string()));
    assert!(
        outcome
            .readback
            .expect("読み出し")
            .stdout_excerpt
            .contains("storekit 1.2.0: eel-test-"),
        "依存先は基盤が作ったパッケージ情報から自分の版を読める"
    );
}

#[test]
fn a_failure_returned_only_to_the_sender_output_is_not_success() {
    let fixture = Fixture::new();
    let mut spec = failure_reported(TrialFault::StorageDirectoryReadOnly);
    spec.checks.push(check(
        "server-error",
        "受信側のエラー",
        CheckMethod::ServerOutput,
        Expectation::Contains,
        Some("could not store"),
    ));
    let result = fixture
        .run(fixture.recipe(SERVE, spec))
        .expect("試行を実行できる");
    assert_eq!(
        result.assessment.verdict,
        Verdict::NotSatisfied,
        "{:?}",
        result.assessment
    );
    let trace = trace(&result);
    assert_eq!(
        trace.exit_code,
        Some(0),
        "送信側は失敗を受け取っても0で終わる"
    );
    assert!(trace.stdout_excerpt.contains("status A700"));
    let outcome = trace.trial.expect("試験の結果");
    let observed = |id: &str| {
        outcome
            .results
            .iter()
            .find(|r| r.id == id)
            .expect(id)
            .observed
    };
    assert_eq!(observed("exit"), Observed::Fail);
    assert_eq!(observed("not-written"), Observed::Pass);
    assert_eq!(observed("server-error"), Observed::Pass);
}

#[test]
fn a_sender_without_a_receiver_reports_the_failure() {
    let fixture = Fixture::new();
    let result = fixture
        .run(fixture.recipe(SERVE, failure_reported(TrialFault::ServerNotStarted)))
        .expect("試行を実行できる");
    assert_eq!(
        result.assessment.verdict,
        Verdict::Satisfied,
        "{:?}",
        result.assessment
    );
    let outcome = trace(&result).trial.expect("試験の結果");
    assert!(outcome.fault_effective);
    assert!(!outcome.server.expect("待つプロセスの記録").started);
}

#[test]
fn a_receiver_that_never_listens_stays_undetermined() {
    let fixture = Fixture::new();
    let mut recipe = fixture.recipe(&["hang", PORT], normal());
    if let Some(server) = recipe.server.as_mut() {
        server.ready_timeout_ms = 300;
    }
    let result = fixture.run(recipe).expect("試行を実行できる");
    assert_eq!(
        result.assessment.verdict,
        Verdict::Unknown,
        "{:?}",
        result.assessment
    );
    assert!(result.assessment.reason.contains("準備できず"));
    let trace = trace(&result);
    assert_eq!(trace.exit_code, None, "操作するプロセスは起動していない");
    let server = trace.trial.expect("試験の結果").server.expect("記録");
    assert!(!server.ready && server.stopped_by_platform);
}

#[test]
fn dependencies_must_be_fetched_and_launches_must_resolve() {
    let fixture = Fixture::new();
    let mut missing = fixture.recipe(SERVE, normal());
    if let Some(python) = missing.python.as_mut() {
        python.dependencies[0].repository = "example/not-fetched".into();
    }
    let error = fixture
        .run(missing)
        .expect_err("取得していない依存先は使わない");
    assert!(error.to_string().contains("取得していません"), "{error}");

    let mut unknown = fixture.recipe(SERVE, normal());
    if let Some(readback) = unknown.trial.as_mut().and_then(|t| t.readback.as_mut()) {
        readback.entrypoint = "not-declared".into();
    }
    assert!(
        fixture.run(unknown).is_err(),
        "宣言されていないコマンドは起動しない"
    );

    let mut no_port = fixture.recipe(&["serve", "11112", "received"], normal());
    assert!(validate_recipe(&no_port).is_err());
    no_port.server = None;
    no_port.python = None;
    assert!(
        validate_recipe(&no_port).is_err(),
        "モジュール起動には読み込み先の設定が必要"
    );
}
