mod support;
use mugiurushi::{
    backup, model::*, recording::new_run_directory, run_record::make_manifest, workspace::*,
};
use serde_json::json;
use std::{fs, process::Command};

#[test]
fn retired_rules_remain_readable_but_cannot_supply_success_or_be_added() {
    let (root, correlation) =
        new_run_directory(&std::env::temp_dir().join("eel-history-tests")).unwrap();
    let mut manifest = make_manifest(
        correlation,
        Condition {
            id: "save".into(),
            version: 1,
            text: "以前の保存条件".into(),
            origin: "旧記録".into(),
            kind: ConditionKind::SaveCompleted,
        },
    )
    .unwrap();
    manifest.mode = "demo".into();
    manifest.input = Some(json!({"name":"過去の合成入力","age":30}));
    manifest.fault = Some(json!("normal"));
    let w = empty_workspace("historical".into(), "過去の記録の読取".into());
    let mut check = support::workspace().checks.remove(0);
    check.rule = Rule::DemoSave;
    check.observation = None;
    check.dependencies.clear();
    check.text = manifest.condition.text.clone();
    check.origin = manifest.condition.origin.clone();
    initialize(&root.join("case"), w, "試験", "旧形式の読取試験").unwrap();
    let mut new_case = empty_workspace("new-retired".into(), "旧規則の新規指定".into());
    new_case.checks.push(check.clone());
    assert!(initialize(&root.join("new-case"), new_case, "試験", "拒否を確認する").is_err());
    assert!(!root.join("new-case").exists());
    let mut historical = load(&root.join("case")).unwrap();
    historical.checks.push(check.clone());
    historical.evidence.push(Evidence {
        review_profile: None,
        id: "evidence-1".into(),
        check_id: "save".into(),
        check_version: 1,
        rule: Rule::DemoSave,
        source: None,
        observation: None,
        execution: None,
        reported: None,
        dependencies: Default::default(),
        dependency_origins: Default::default(),
        manifest: manifest.clone(),
        events: vec![Event {
            sequence: 1,
            elapsed_us: 0,
            correlation: manifest.correlation.clone(),
            source: Source::Server,
            kind: EventKind::SaveCommitted,
            detail: "過去の成功イベント".into(),
        }],
        adapter: None,
    });
    validate(&historical).unwrap();
    assert_eq!(view(historical.clone()).checks[0].verdict, Verdict::Unknown);
    // 旧JSON形式を作り、移行後も過去の記録を読めることを確認する。
    fs::remove_file(root.join("case/workspace.sqlite3")).unwrap();
    for suffix in ["-wal", "-shm"] {
        let _ = fs::remove_file(root.join(format!("case/workspace.sqlite3{suffix}")));
    }
    fs::create_dir(root.join("case/revisions")).unwrap();
    fs::write(
        root.join("case/revisions/00000000000000000001.json"),
        serde_json::to_vec(&historical).unwrap(),
    )
    .unwrap();
    let saved = backup::snapshot(&root.join("case")).unwrap();
    let restored = backup::restore(&root.join("restored"), &saved).unwrap();
    assert_eq!(
        restored.workspace.evidence[0].manifest.input,
        manifest.input
    );
    assert_eq!(restored.checks[0].verdict, Verdict::Unknown);
    check.id = "new-retired-check".into();
    let result = change(
        &root.join("case"),
        ChangeRequest {
            expected_revision: 1,
            actor: "試験".into(),
            reason: "旧規則の新規指定".into(),
            action: Change::AddCheck { check },
        },
    );
    assert!(result.unwrap_err().to_string().contains("旧デモ"));
    assert_eq!(load(&root.join("case")).unwrap().revision, 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn retired_cli_commands_fail_instead_of_starting_the_app_or_creating_outputs() {
    let (root, _) = new_run_directory(&std::env::temp_dir().join("eel-retired-cli-tests")).unwrap();
    for command in [
        "demo", "suite", "capture", "verify", "retest", "review", "explore",
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_mugiurushi"))
            .arg(command)
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(!result.status.success(), "{command}");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    }
    fs::remove_dir(root).unwrap();
}
