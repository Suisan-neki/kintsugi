//! 試験案（判定条件・故障・基盤による保存先の読み出し）を、架空の小さな記録アプリで確かめる。
mod support;
use mugiurushi::{execution::*, model::*, recording::*, trial::*, workspace::*};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use support::execute;

const NOTES_V1: &str = r#"import sys, os
def main(argv):
    directory, command = argv[1], argv[2]
    path = os.path.join(directory, "notes")
    if command == "add":
        try:
            with open(path, "a") as f:
                f.write(argv[3] + "\n")
        except OSError as e:
            sys.stderr.write("error: " + str(e) + "\n")
            return 0
        print("added")
        return 0
    if command == "claim":
        print("saved " + argv[3])
        return 0
    if command == "list":
        with open(path) as f:
            sys.stdout.write(f.read())
        return 0
    return 2
sys.exit(main(sys.argv))
"#;

const NOTES_MJS: &str = r#"import { appendFileSync, readFileSync } from "node:fs";
const [directory, command, value] = process.argv.slice(2);
const path = `${directory}/notes`;
if (command === "add") {
  try { appendFileSync(path, `${value}\n`); console.log("added"); }
  catch (error) { console.error(`error: ${error.message}`); process.exit(1); }
} else if (command === "list") {
  process.stdout.write(readFileSync(path, "utf8"));
}
"#;

struct Fixture {
    directory: PathBuf,
    root: PathBuf,
    commits: Vec<String>,
}
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
impl Fixture {
    fn new() -> Self {
        let directory = new_run_directory(&std::env::temp_dir().join("evidence-trial-tests"))
            .expect("試験用ディレクトリ")
            .0;
        let root = directory.join("repo");
        fs::create_dir(&root).expect("リポジトリ");
        git(&root, &["init", "-q"]);
        git(&root, &["config", "user.name", "Synthetic test"]);
        git(&root, &["config", "user.email", "test@example.invalid"]);
        git(
            &root,
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/example/notes.git",
            ],
        );
        fs::write(root.join("notes.py"), NOTES_V1).expect("v1");
        fs::write(root.join("notes.mjs"), NOTES_MJS).expect("mjs");
        git(&root, &["add", "."]);
        git(&root, &["commit", "-qm", "v1"]);
        let first = git(&root, &["rev-parse", "HEAD"]);
        // 修正後: 書き込み失敗を終了コード1で知らせる。
        fs::write(
            root.join("notes.py"),
            NOTES_V1.replacen(
                "            return 0\n        print",
                "            return 1\n        print",
                1,
            ),
        )
        .expect("v2");
        git(&root, &["commit", "-qam", "v2"]);
        let second = git(&root, &["rev-parse", "HEAD"]);
        Self {
            directory,
            root,
            commits: vec![first, second],
        }
    }
    fn run(&self, commit: usize, runtime: Runtime, command: &str, spec: TrialSpec) -> RunResult {
        let entrypoint = if runtime == Runtime::Node {
            "notes.mjs"
        } else {
            "notes.py"
        };
        let recipe = Recipe {
            schema_version: 1,
            check_id: "notes".into(),
            source: CodeReference {
                repository: "example/notes".into(),
                commit: self.commits[commit].clone(),
                path: entrypoint.into(),
                line: 1,
                end_line: 1,
            },
            runtime,
            entrypoint: entrypoint.into(),
            args: vec![".".into(), command.into(), TEST_VALUE.into()],
            input: serde_json::json!({}),
            criterion: Criterion {
                assertion: "eel.trial".into(),
                subject: "notes".into(),
            },
            timeout_ms: 5000,
            trial: Some(spec),
            launch: Default::default(),
            python: None,
            server: None,
            containers: None,
            http: None,
            browser: None,
        };
        git(&self.root, &["checkout", "-q", &self.commits[commit]]);
        execute(
            &self.root,
            &self.directory.join("runs"),
            recipe,
            Condition {
                id: "notes".into(),
                version: 1,
                text: "架空のメモを追加したとき、後から読み出せる".into(),
                origin: "試験".into(),
                kind: ConditionKind::Custom,
            },
            None,
        )
        .expect("試行を実行できる")
    }
}
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
fn saved(fault: TrialFault, readback: bool) -> TrialSpec {
    let mut checks = vec![
        check(
            "accepted",
            "受付",
            CheckMethod::ExitCode,
            Expectation::Zero,
            None,
        ),
        check(
            "written",
            "書き込み",
            CheckMethod::StoredFile,
            Expectation::Contains,
            Some(TEST_VALUE),
        ),
    ];
    if readback {
        checks.push(check(
            "readback",
            "後からの読み出し",
            CheckMethod::ReadbackOutput,
            Expectation::Contains,
            Some(TEST_VALUE),
        ));
    }
    TrialSpec {
        schema_version: 1,
        recovery: None,
        fault,
        storage: Some("notes".into()),
        readback: readback.then(|| Readback {
            launch: Default::default(),
            entrypoint: "notes.py".into(),
            args: vec![".".into(), "list".into()],
        }),
        checks,
    }
}
fn failure_reported(fault: TrialFault) -> TrialSpec {
    TrialSpec {
        schema_version: 1,
        recovery: None,
        fault,
        storage: Some("notes".into()),
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
fn outcome(result: &RunResult) -> TrialOutcome {
    let trace: ExecutionTrace = serde_json::from_slice(
        &fs::read(result.directory.join("execution.json")).expect("execution.json"),
    )
    .expect("記録を読める");
    trace.trial.expect("試験の結果")
}

#[test]
fn normal_run_is_confirmed_by_storage_and_separate_readback() {
    let fixture = Fixture::new();
    let result = fixture.run(0, Runtime::Python3, "add", saved(TrialFault::Normal, true));
    assert_eq!(
        result.assessment.verdict,
        Verdict::Satisfied,
        "{:?}",
        result.assessment
    );
    let outcome = outcome(&result);
    assert!(outcome.test_value.starts_with("eel-test-"));
    let state = outcome.state.expect("保存先を読み出した");
    assert_eq!(state.status, "file");
    assert_eq!(state.contains_test_value, Some(true));
    assert!(
        outcome
            .readback
            .expect("読み出しを実行した")
            .stdout_excerpt
            .contains(&outcome.test_value)
    );
    // 試行のたびに架空の値が変わり、前の試行の値と混ざらない。
    let again = fixture.run(0, Runtime::Python3, "add", saved(TrialFault::Normal, false));
    assert_ne!(self::outcome(&again).test_value, outcome.test_value);
}

#[test]
fn exit_code_zero_and_success_message_are_not_enough() {
    let fixture = Fixture::new();
    let result = fixture.run(
        0,
        Runtime::Python3,
        "claim",
        saved(TrialFault::Normal, false),
    );
    assert_eq!(result.assessment.verdict, Verdict::NotSatisfied);
    let outcome = outcome(&result);
    let written = outcome
        .results
        .iter()
        .find(|r| r.id == "written")
        .expect("書き込みの結果");
    assert_eq!(written.observed, Observed::Fail);
    assert_eq!(outcome.state.expect("読み出した").status, "missing");
}

#[test]
fn missing_observation_stays_unknown() {
    let fixture = Fixture::new();
    let result = fixture.run(
        0,
        Runtime::Python3,
        "add",
        saved(TrialFault::SkipStateRead, false),
    );
    assert_eq!(
        result.assessment.verdict,
        Verdict::Unknown,
        "{:?}",
        result.assessment
    );
    let outcome = outcome(&result);
    assert!(!outcome.state.expect("読み出しの記録").performed);
}

#[test]
fn read_only_storage_distinguishes_silent_failure_from_reported_failure() {
    let fixture = Fixture::new();
    let before = fixture.run(
        0,
        Runtime::Python3,
        "add",
        failure_reported(TrialFault::StorageReadOnly),
    );
    assert_eq!(
        before.assessment.verdict,
        Verdict::NotSatisfied,
        "{:?}",
        before.assessment
    );
    let outcome = outcome(&before);
    assert!(outcome.fault_effective, "{}", outcome.fault_applied);
    let after = fixture.run(
        1,
        Runtime::Python3,
        "add",
        failure_reported(TrialFault::StorageReadOnly),
    );
    assert_eq!(
        after.assessment.verdict,
        Verdict::Satisfied,
        "{:?}",
        after.assessment
    );
    // 前の試行の記録は残る。
    assert!(before.directory.join("execution.json").is_file());
}

#[test]
fn directory_at_storage_path_is_a_real_fault() {
    let fixture = Fixture::new();
    let result = fixture.run(
        1,
        Runtime::Python3,
        "add",
        failure_reported(TrialFault::StorageIsDirectory),
    );
    let outcome = outcome(&result);
    assert!(outcome.fault_effective);
    assert_eq!(outcome.state.expect("読み出した").status, "directory");
    assert_eq!(
        result.assessment.verdict,
        Verdict::Satisfied,
        "{:?}",
        result.assessment
    );
}

#[test]
fn node_target_runs_with_the_same_checks() {
    let fixture = Fixture::new();
    let mut spec = saved(TrialFault::Normal, false);
    spec.checks.push(check(
        "stdout",
        "受付",
        CheckMethod::Stdout,
        Expectation::Contains,
        Some("added"),
    ));
    let result = fixture.run(0, Runtime::Node, "add", spec);
    assert_eq!(
        result.assessment.verdict,
        Verdict::Satisfied,
        "{:?}",
        result.assessment
    );
}

#[test]
fn conflicting_observations_in_one_scope_are_unknown() {
    let fixture = Fixture::new();
    let mut spec = saved(TrialFault::Normal, false);
    // 同じ「書き込み」の範囲で、対象の出力と基盤の読み出しが食い違う条件を置く。
    spec.checks.push(check(
        "claimed",
        "書き込み",
        CheckMethod::Stdout,
        Expectation::Contains,
        Some("never-printed"),
    ));
    let result = fixture.run(0, Runtime::Python3, "add", spec);
    assert_eq!(
        result.assessment.verdict,
        Verdict::Unknown,
        "{:?}",
        result.assessment
    );
}

#[test]
fn invalid_specs_are_rejected_before_running() {
    let fixture = Fixture::new();
    let only_exit = TrialSpec {
        schema_version: 1,
        recovery: None,
        fault: TrialFault::Normal,
        storage: None,
        readback: None,
        checks: vec![check(
            "exit",
            "受付",
            CheckMethod::ExitCode,
            Expectation::Zero,
            None,
        )],
    };
    let escape = TrialSpec {
        storage: Some("../outside".into()),
        ..saved(TrialFault::Normal, false)
    };
    for spec in [only_exit, escape] {
        let recipe = Recipe {
            schema_version: 1,
            check_id: "notes".into(),
            source: CodeReference {
                repository: "example/notes".into(),
                commit: fixture.commits[0].clone(),
                path: "notes.py".into(),
                line: 1,
                end_line: 1,
            },
            runtime: Runtime::Python3,
            entrypoint: "notes.py".into(),
            args: vec![],
            input: serde_json::json!({}),
            criterion: Criterion {
                assertion: "eel.trial".into(),
                subject: "notes".into(),
            },
            timeout_ms: 5000,
            trial: Some(spec),
            launch: Default::default(),
            python: None,
            server: None,
            containers: None,
            http: None,
            browser: None,
        };
        assert!(validate_recipe(&recipe).is_err());
    }
}

fn database_read(
    id: &str,
    performed: bool,
    exit_code: Option<i32>,
    text: &str,
) -> mugiurushi::browser::DatabaseRead {
    mugiurushi::browser::DatabaseRead {
        id: id.into(),
        performed,
        exit_code,
        text: text.into(),
        detail: String::new(),
    }
}

#[test]
fn recovery_comparison_never_reads_an_unreadable_state_as_no_rows() {
    let before = [
        database_read("patient", false, None, ""),
        database_read("visit", true, Some(0), "v\t1\n"),
        database_read("failed", true, Some(1), ""),
        database_read("soap", true, Some(0), "a\t1\n"),
        database_read("note", true, Some(0), "a\t1\n"),
        database_read("dup", true, Some(0), "a\t1\n"),
        database_read("gone", true, Some(0), "a\t1\nb\t2\n"),
        database_read("unknown", true, None, ""),
        database_read("truncated", true, Some(0), "a\t1\n"),
        database_read("empty", true, Some(0), ""),
        database_read("missing", true, Some(0), "a\t1\n"),
    ];
    let after = [
        database_read("patient", true, Some(0), "p\t1\n"),
        database_read("visit", true, Some(1), ""),
        database_read("failed", true, Some(0), "f\t1\n"),
        database_read("soap", true, Some(0), "a\t1\n"),
        database_read("note", true, Some(0), ""),
        database_read("dup", true, Some(0), "a\t1\na\t1\n"),
        database_read("gone", true, Some(0), "a\t1\n"),
        database_read("unknown", true, None, ""),
        database_read("truncated", true, None, ""),
        database_read("empty", true, Some(0), ""),
    ];
    let changes = compare_state(&before, &after, &[], &[]);
    let by_id = |id: &str| changes.iter().find(|c| c.id == id).unwrap();
    // 停止中に読めなかった行は「行がなかった」ではなく「復旧後に読めた」とする。
    assert_eq!(by_id("patient").change, "readable_after_recovery");
    assert_eq!(by_id("patient").rows_during_fault, None);
    assert_eq!(by_id("patient").rows_after_recovery, Some(1));
    // 失敗した読み出しも同様に、行がないとは扱わない。
    assert_eq!(by_id("visit").change, "unreadable_after_recovery");
    assert_eq!(by_id("failed").change, "readable_after_recovery");
    assert_eq!(by_id("failed").rows_during_fault, None);
    assert_eq!(by_id("soap").change, "unchanged");
    assert_eq!(by_id("note").change, "rows_removed");
    assert_eq!(by_id("dup").change, "rows_added");
    assert_eq!(by_id("dup").rows_after_recovery, Some(2));
    assert_eq!(by_id("gone").change, "rows_removed");
    assert_eq!(by_id("unknown").change, "not_observed");
    assert_eq!(by_id("unknown").rows_during_fault, None);
    assert_eq!(by_id("unknown").rows_after_recovery, None);
    assert_eq!(by_id("truncated").change, "unreadable_after_recovery");
    assert_eq!(by_id("truncated").rows_after_recovery, None);
    assert_eq!(by_id("empty").change, "unchanged");
    assert_eq!(by_id("empty").rows_after_recovery, Some(0));
    assert_eq!(by_id("missing").change, "unreadable_after_recovery");
}

#[test]
fn recovery_is_only_allowed_for_faults_that_can_be_undone() {
    let mut spec = saved(TrialFault::StorageReadOnly, false);
    spec.recovery = Some(Recovery {
        ready_timeout_ms: 1000,
    });
    assert!(mugiurushi::trial::validate(&spec, false, true, false, |_| Ok(())).is_err());
    let mut database = saved(TrialFault::DatabaseStopped, false);
    database.recovery = Some(Recovery {
        ready_timeout_ms: 0,
    });
    assert!(mugiurushi::trial::validate(&database, false, true, true, |_| Ok(())).is_err());
    database.recovery = Some(Recovery {
        ready_timeout_ms: 1000,
    });
    assert!(mugiurushi::trial::validate(&database, false, true, true, |_| Ok(())).is_ok());
    // 復旧を指定しない設定の保存形式は変わらない。
    assert!(
        !serde_json::to_string(&saved(TrialFault::Normal, false))
            .unwrap()
            .contains("recovery")
    );
}
