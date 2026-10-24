use mugiurushi::{backup, recording::new_run_directory, review_report, workspace::*};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

struct Case(PathBuf);
impl Case {
    fn apply(&self, action: Value) {
        change(
            &self.0,
            ChangeRequest {
                expected_revision: load(&self.0).unwrap().revision,
                actor: "合成の記入者".into(),
                reason: "試用記録の検査".into(),
                action: serde_json::from_value(action).unwrap(),
            },
        )
        .unwrap();
    }
}
impl Drop for Case {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(self.0.parent().unwrap());
    }
}

fn recorded_case() -> Case {
    let root = new_run_directory(&std::env::temp_dir().join("mugiurushi-trial-export"))
        .unwrap()
        .0;
    let case = Case(root.join("case"));
    let mut w = empty_workspace("trial-export".into(), "合成の試用記録".into());
    w.context
        .insert("private-configuration".into(), "内部設定の合成値".into());
    initialize(&case.0, w, "作成者", "試験").unwrap();
    let variants = ["a", "b"].map(|label| {
        json!({
            "label": label,
            "source": {"repository":"openemr/openemr", "commit":label.repeat(40),
                "path":"interface/forms/soap/save.php", "line":1,"end_line":1},
            "url":format!("http://localhost:{}/", if label == "a" {8081} else {8082}),
            "evidence_ids":[]
        })
    });
    case.apply(json!({"op":"add_user_trial_plan", "plan":{
        "id":"save", "concern":"保存完了が分からないという仮想の相談",
        "task":"架空の記録を保存する", "conditions":"合成データだけを使う。終了後に停止する。",
        "variants":variants
    }}));
    case.apply(json!({"op":"start_user_trial", "plan_id":"save", "participant":"participant-1", "order":["b","a"]}));
    for label in ["b", "a"] {
        case.apply(json!({"op":"record_user_trial_feedback", "session_id":"user-trial-1",
            "variant":label,"outcome":"completed", "operation":"保存ボタンを押した（合成の自己申告）",
            "feedback":"<script>未実行</script> 保存完了の表示を確認したと申告"}));
    }
    case
}

#[test]
fn backup_preserves_trial_binding_and_rejects_rewritten_plan_session_or_feedback() {
    let case = recorded_case();
    let archive = backup::snapshot(&case.0).unwrap();
    let restored = backup::restore(&case.0.parent().unwrap().join("restored"), &archive).unwrap();
    assert_eq!(
        restored.workspace.user_trial_plans[0].variants[1]
            .source
            .commit,
        "b".repeat(40)
    );
    assert_eq!(restored.workspace.user_trial_sessions[0].feedback.len(), 2);
    assert!(!restored.verification_passed);
    for (pointer, replacement) in [
        ("/user_trial_plans/0/task", json!("別の操作へ変更")),
        (
            "/user_trial_plans/0/variants/0/source/commit",
            json!("c".repeat(40)),
        ),
        ("/user_trial_sessions/0/participant", json!("別の試用者")),
        (
            "/user_trial_sessions/0/feedback/0/feedback",
            json!("後から変更した感想"),
        ),
        ("/user_trial_sessions/0/feedback", json!([])),
        ("/user_trial_sessions", json!([])),
    ] {
        let mut changed = serde_json::to_value(&archive).unwrap();
        *changed["revisions"]
            .as_array_mut()
            .unwrap()
            .last_mut()
            .unwrap()
            .pointer_mut(pointer)
            .unwrap() = replacement;
        let tampered = serde_json::from_value(changed).unwrap();
        assert!(backup::validate(&tampered).is_err(), "accepted {pointer}");
    }

    // 全版の計画を一緒に書き換えても、作成直前の実行条件との不一致を拒否する。
    let mut changed = serde_json::to_value(&archive).unwrap();
    for revision in changed["revisions"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .skip(1)
    {
        revision["user_trial_plans"][0]["context"]["private-configuration"] =
            json!("作成時にはなかった値");
    }
    assert!(backup::validate(&serde_json::from_value(changed).unwrap()).is_err());
}

#[test]
fn report_keeps_exact_trial_versions_and_escaped_feedback_without_endpoints_or_quality_claims() {
    let case = recorded_case();
    let report = review_report::build(load(&case.0).unwrap()).unwrap();
    let value = serde_json::to_value(&report).unwrap();
    assert_eq!(
        value["user_trial_plans"][0]["variants"][0]["source"]["commit"],
        "a".repeat(40)
    );
    assert_eq!(value["user_trial_sessions"][0]["order"], json!(["b", "a"]));
    assert_eq!(
        value["user_trial_sessions"][0]["feedback"][0]["recorded_revision"],
        4
    );
    assert!(
        value["user_trial_plans"][0]["variants"][0]
            .get("url")
            .is_none()
    );
    assert!(value["user_trial_plans"][0].get("context").is_none());
    assert!(!report.verification_passed);
    assert!(report.decisions.is_empty());
    let html = review_report::html(&report).unwrap();
    assert!(html.contains("操作順 B→A"));
    assert!(html.contains("自己申告"));
    assert!(html.contains("&lt;script&gt;未実行&lt;/script&gt;"));
    assert!(!html.contains("<script>"));
    assert!(!html.contains("http://localhost:808"));
    assert!(!html.contains("内部設定の合成値"));
}
