use mugiurushi::{
    backup,
    comparison::{self, CompareRequest},
    execution::{Criterion, ObservationOutcome},
    model::*,
    reader::{self, ReaderRequest},
    reported::*,
    workspace::*,
};
use std::{collections::BTreeMap, fs, path::PathBuf};
struct Case(PathBuf);
impl Drop for Case {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn case() -> Case {
    let (dir, _) =
        mugiurushi::recording::new_run_directory(&std::env::temp_dir().join("eel-backend-tests"))
            .unwrap();
    let dir = dir.join("case");
    initialize(
        &dir,
        empty_workspace("general".into(), "合成の通知確認".into()),
        "合成の作成者",
        "開始",
    )
    .unwrap();
    let c = Case(dir);
    apply(
        &c,
        Change::AddCheck {
            check: Check {
                id: "delivered".into(),
                version: 1,
                text: "通知を読み出した結果が入力と一致する".into(),
                origin: "合成の条件".into(),
                source: None,
                observation: None,
                rule: Rule::ManualOnly,
                dependencies: vec![],
                questions: vec![],
            },
        },
    );
    apply(
        &c,
        Change::ConfigureReportedObservation {
            check_id: "delivered".into(),
            criterion: Criterion {
                assertion: "readback_matches".into(),
                subject: "notification-1".into(),
            },
        },
    );
    c
}
fn apply(c: &Case, action: Change) -> WorkspaceView {
    change(
        &c.0,
        ChangeRequest {
            expected_revision: load(&c.0).unwrap().revision,
            actor: "合成の確認者".into(),
            reason: "合成の検証".into(),
            action,
        },
    )
    .unwrap()
}
fn trace(id: &str, outcomes: &[ObservationOutcome]) -> ReportedTrace {
    let correlation = Correlation {
        run_id: id.into(),
        record_id: "notification-1".into(),
        attempt_id: format!("{id}-attempt"),
    };
    ReportedTrace {
        schema_version: 1,
        correlation: correlation.clone(),
        producer: "合成のGoアプリを想定した入力".into(),
        origin: "合成の報告。Goの実行は未実施".into(),
        completed: true,
        source: None,
        context: BTreeMap::from([(
            "input".into(),
            serde_json::json!({"id":9007199254740993u64,"message":"合成"}),
        )]),
        observations: outcomes
            .iter()
            .enumerate()
            .map(|(i, outcome)| ReportedObservation {
                sequence: i as u64 + 1,
                correlation: correlation.clone(),
                assertion: "readback_matches".into(),
                subject: "notification-1".into(),
                outcome: outcome.clone(),
                detail: "報告の範囲のみ".into(),
                source: None,
            })
            .collect(),
    }
}
fn import(c: &Case, t: ReportedTrace) -> WorkspaceView {
    let w = load(&c.0).unwrap();
    let check = &w.checks[0];
    apply(
        c,
        Change::ImportReported {
            condition: Condition {
                id: check.id.clone(),
                version: check.version,
                text: check.text.clone(),
                origin: check.origin.clone(),
                kind: ConditionKind::Custom,
            },
            trace: t,
        },
    )
}
#[test]
fn reported_observations_do_not_fill_missing_or_conflicting_results_with_success() {
    let c = case();
    assert_eq!(
        import(&c, trace("pass", &[ObservationOutcome::Pass])).checks[0].verdict,
        Verdict::Satisfied
    );
    assert_eq!(
        import(&c, trace("fail", &[ObservationOutcome::Fail])).checks[0].verdict,
        Verdict::NotSatisfied
    );
    assert_eq!(
        import(
            &c,
            trace(
                "conflict",
                &[ObservationOutcome::Pass, ObservationOutcome::Fail]
            )
        )
        .checks[0]
            .verdict,
        Verdict::Unknown
    );
    assert_eq!(
        import(&c, trace("missing", &[])).checks[0].verdict,
        Verdict::Unknown
    );
    let mut incomplete = trace("incomplete", &[ObservationOutcome::Pass]);
    incomplete.completed = false;
    let v = import(&c, incomplete);
    assert_eq!(v.checks[0].verdict, Verdict::Unknown);
    assert!(v.checks[0].evidence.last().unwrap().observed.unconfirmed[0].contains("独立検証"));
}
#[test]
fn reported_import_rejects_foreign_ids_stale_conditions_and_duplicates_without_mutation() {
    let c = case();
    let w = load(&c.0).unwrap();
    let mut t = trace("foreign", &[ObservationOutcome::Pass]);
    t.observations[0].correlation.attempt_id = "other".into();
    assert!(mugiurushi::reported::validate(&t).is_err());
    let cond = Condition {
        id: w.checks[0].id.clone(),
        version: w.checks[0].version - 1,
        text: w.checks[0].text.clone(),
        origin: w.checks[0].origin.clone(),
        kind: ConditionKind::Custom,
    };
    assert!(
        change(
            &c.0,
            ChangeRequest {
                expected_revision: w.revision,
                actor: "合成".into(),
                reason: "古い条件".into(),
                action: Change::ImportReported {
                    condition: cond,
                    trace: trace("old", &[])
                }
            }
        )
        .is_err()
    );
    assert_eq!(load(&c.0).unwrap().revision, w.revision);
    import(&c, trace("unique", &[]));
    let saved = load(&c.0).unwrap();
    let check = &saved.checks[0];
    assert!(
        change(
            &c.0,
            ChangeRequest {
                expected_revision: saved.revision,
                actor: "合成".into(),
                reason: "二重取り込み".into(),
                action: Change::ImportReported {
                    condition: Condition {
                        id: check.id.clone(),
                        version: check.version,
                        text: check.text.clone(),
                        origin: check.origin.clone(),
                        kind: ConditionKind::Custom
                    },
                    trace: trace("unique", &[])
                }
            }
        )
        .is_err()
    );
    assert_eq!(load(&c.0).unwrap().revision, saved.revision);
}
#[test]
fn backend_comparison_keeps_exact_json_numbers_and_historical_applicability() {
    let c = case();
    import(&c, trace("old", &[ObservationOutcome::Fail]));
    let mut changed = trace("new", &[ObservationOutcome::Pass]);
    changed.context.insert(
        "input".into(),
        serde_json::json!({"id":9007199254740992u64}),
    );
    let v = import(&c, changed);
    let request = CompareRequest {
        expected_revision: v.workspace.revision,
        check_id: "delivered".into(),
        before_id: "evidence-1".into(),
        after_id: "evidence-2".into(),
    };
    let compared = comparison::compare(v.workspace.clone(), request).unwrap();
    assert!(!compared.before.evidence.applicable);
    assert_eq!(
        compared.before.evidence.observed.verdict,
        Verdict::NotSatisfied
    );
    let row = compared
        .differences
        .iter()
        .find(|r| r.key == "/reported/context")
        .unwrap();
    assert_eq!(row.status, "changed");
    assert_eq!(
        row.before.as_ref().unwrap()["input"]["id"].as_u64(),
        Some(9007199254740993)
    );
    assert!(
        comparison::compare(
            v.workspace,
            CompareRequest {
                expected_revision: 0,
                check_id: "delivered".into(),
                before_id: "evidence-1".into(),
                after_id: "evidence-2".into()
            }
        )
        .is_err()
    );
}
#[test]
fn backup_restores_all_revisions_and_decision_basis_without_replacing_existing_case() {
    let c = case();
    import(&c, trace("pass", &[ObservationOutcome::Pass]));
    apply(
        &c,
        Change::Decide {
            role: Role::Developer,
            action: DecisionKind::Verify,
            check_ids: vec!["delivered".into()],
            change_reference: None,
            confidence: None,
            elapsed_ms: None,
        },
    );
    import(&c, trace("missing", &[]));
    let saved = load(&c.0).unwrap();
    let b = backup::snapshot(&c.0).unwrap();
    let path = c.0.with_file_name("restored");
    let v = backup::restore(&path, &b).unwrap();
    assert_eq!(
        serde_json::to_value(&saved).unwrap(),
        serde_json::to_value(&v.workspace).unwrap()
    );
    assert_eq!(v.checks[0].verdict, Verdict::Unknown);
    assert!(backup::restore(&path, &b).is_err());
    let mut bad = backup::snapshot(&c.0).unwrap();
    bad.revisions.last_mut().unwrap().evidence[0]
        .reported
        .as_mut()
        .unwrap()
        .producer = "changed".into();
    assert!(backup::validate(&bad).is_err());
    let mut bad = backup::snapshot(&c.0).unwrap();
    bad.revisions.last_mut().unwrap().decisions[0].basis[0].verdict = Verdict::NotSatisfied;
    assert!(backup::validate(&bad).is_err());
    fs::write(path.join("restore.pending"), "{}").unwrap();
    assert!(load(&path).is_err());
    let _ = fs::remove_dir_all(path);
}
#[test]
fn reader_api_reuses_static_analysis_and_never_runs_repository_source() {
    let snapshot = serde_json::json!({"schema":1,"fullName":"example/project","commit":"a".repeat(40),"reference":"main","truncated":false,"files":{"app.js":"function main() { save(); }\nfunction save() {}"},"inventory":[{"path":"app.js","size":55,"mode":"100644"}],"omitted":[]});
    let result = reader::read(ReaderRequest::Analyze {
        snapshot,
        path: Some("app.js".into()),
        line: Some(1),
    })
    .unwrap();
    assert_eq!(result["guide"]["name"], "main");
    assert_eq!(result["guide"]["calls"][0]["candidates"][0]["name"], "save");
    assert!(
        reader::read(ReaderRequest::Read {
            url: "http://127.0.0.1:9/private".into(),
            reference: "main".into()
        })
        .is_err()
    );
}

#[test]
fn backup_keeps_archived_policy_explanations_and_rejects_foreign_basis() {
    let c = case();
    import(&c, trace("pass", &[ObservationOutcome::Pass]));
    apply(
        &c,
        Change::Decide {
            role: Role::Developer,
            action: DecisionKind::Verify,
            check_ids: vec!["delivered".into()],
            change_reference: None,
            confidence: None,
            elapsed_ms: None,
        },
    );
    let mut archive = backup::snapshot(&c.0).unwrap();
    // 旧版の説明文を現在の判定器で書き換えたり、説明が違うだけで復元不能にしない。
    archive.revisions.last_mut().unwrap().decisions[0].basis[0].reason =
        "旧版の判定器が残した説明".into();
    backup::validate(&archive).unwrap();
    archive.revisions.last_mut().unwrap().decisions[0].basis[0].evidence[0].run_id =
        "foreign".into();
    assert!(backup::validate(&archive).is_err());
}

#[test]
fn oversized_restore_is_rejected_before_creating_a_target() {
    let c = case();
    let mut archive = backup::snapshot(&c.0).unwrap();
    // バックアップ全体の32MiB上限を超える。4MiBを超える事例自体は保存できる。
    for i in 0..34 {
        archive.revisions[0]
            .context
            .insert(format!("large-{i}"), "x".repeat(1024 * 1024));
    }
    let target = c.0.with_file_name("oversized-restore");
    assert!(backup::restore(&target, &archive).is_err());
    assert!(!target.exists());
}
