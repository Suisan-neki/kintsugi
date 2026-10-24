mod support;
use mugiurushi::{
    profile,
    recording::new_run_directory,
    user_trial::{Label, Outcome, PlanInput, Variant},
    workspace::*,
};
use std::{
    fs,
    path::{Path, PathBuf},
};

struct Case(PathBuf);
impl Case {
    fn new() -> Self {
        let root = new_run_directory(&std::env::temp_dir().join("mugiurushi-user-trial-tests"))
            .unwrap()
            .0;
        let path = root.join("case");
        initialize(&path, support::workspace(), "開発者", "合成の試用条件").unwrap();
        Self(path)
    }
    fn apply(&self, action: Change) -> std::io::Result<WorkspaceView> {
        change(
            &self.0,
            ChangeRequest {
                expected_revision: load(&self.0)?.revision,
                actor: "合成の記入者".into(),
                reason: "試験".into(),
                action,
            },
        )
    }
}
impl Drop for Case {
    fn drop(&mut self) {
        if let Some(parent) = self.0.parent() {
            let _ = fs::remove_dir_all(parent);
        }
    }
}
fn source(commit: &str) -> CodeReference {
    CodeReference {
        repository: "openemr/openemr".into(),
        commit: commit.repeat(40),
        path: "interface/forms/soap/save.php".into(),
        line: 1,
        end_line: 1,
    }
}
fn plan() -> PlanInput {
    PlanInput {
        id: "save-comparison".into(),
        concern: "保存できたか分からない（合成事例）".into(),
        task: "架空の問診を保存して次の操作へ進む".into(),
        conditions: "試験専用データ。開始前にデータを初期化。手動で起動し、終了後に停止する。"
            .into(),
        variants: [
            Variant {
                label: Label::A,
                source: source("a"),
                url: "http://127.0.0.1:8081/".into(),
                evidence_ids: vec![],
            },
            Variant {
                label: Label::B,
                source: source("b"),
                url: "http://localhost:8082/".into(),
                evidence_ids: vec![],
            },
        ],
    }
}
fn feedback(label: Label) -> Change {
    Change::RecordUserTrialFeedback {
        session_id: "user-trial-1".into(),
        variant: label,
        outcome: Outcome::Completed,
        operation: "保存ボタンを一度押した".into(),
        feedback: "完了したか判断できなかった".into(),
    }
}
fn start() -> Change {
    Change::StartUserTrial {
        plan_id: "save-comparison".into(),
        participant: "参加者01（合成）".into(),
        order: [Label::B, Label::A],
    }
}
fn revision_count(path: &Path) -> usize {
    fs::read_dir(path.join("revisions")).unwrap().count()
}

#[test]
fn ordered_feedback_keeps_exact_plan_and_does_not_turn_into_quality_or_consent() {
    let case = Case::new();
    case.apply(Change::AddUserTrialPlan { plan: plan() })
        .unwrap();
    case.apply(start()).unwrap();
    let before = revision_count(&case.0);
    assert!(case.apply(feedback(Label::A)).is_err());
    assert_eq!(revision_count(&case.0), before);
    let first = case.apply(feedback(Label::B)).unwrap();
    assert_eq!(
        first.workspace.user_trial_sessions[0].feedback[0].variant,
        Label::B
    );
    assert!(case.apply(feedback(Label::B)).is_err());
    let final_view = case.apply(feedback(Label::A)).unwrap();
    assert!(!final_view.verification_passed);
    assert!(final_view.workspace.decisions.is_empty());
    assert!(final_view.workspace.statements.is_empty());
    let saved = load(&case.0).unwrap();
    let p = &saved.user_trial_plans[0];
    assert_eq!(p.variants[0].source.commit, "a".repeat(40));
    assert_eq!(p.recorded_by, "合成の記入者");
    assert_eq!(saved.user_trial_sessions[0].feedback.len(), 2);
    assert!(case.apply(feedback(Label::A)).is_err());
}

#[test]
fn changed_conditions_require_new_plan_but_existing_session_keeps_historical_feedback() {
    let case = Case::new();
    case.apply(Change::AddUserTrialPlan { plan: plan() })
        .unwrap();
    case.apply(start()).unwrap();
    case.apply(Change::SetContext {
        key: "environment".into(),
        value: "changed after session start".into(),
    })
    .unwrap();
    assert!(case.apply(start()).is_err());
    case.apply(feedback(Label::B)).unwrap();
    assert_eq!(
        load(&case.0).unwrap().user_trial_plans[0].context["environment"],
        "test"
    );
    assert!(
        case.apply(Change::AddUserTrialPlan { plan: plan() })
            .is_err()
    );
    let mut next = plan();
    next.id = "save-comparison-2".into();
    case.apply(Change::AddUserTrialPlan { plan: next }).unwrap();
}

#[test]
fn revised_check_requires_new_trial_plan() {
    let case = Case::new();
    case.apply(Change::AddUserTrialPlan { plan: plan() })
        .unwrap();
    case.apply(Change::ReviseCheck {
        check_id: "later-view".into(),
        text: "別の確認事項".into(),
        origin: "合成".into(),
        rule: Rule::ManualOnly,
        dependencies: vec![],
    })
    .unwrap();
    assert!(case.apply(start()).is_err());
}

#[test]
fn trial_targets_reject_unpinned_or_unrelated_sources_and_unsafe_urls_without_saving() {
    let case = Case::new();
    for bad in [
        "https://example.com/",
        "javascript:alert(1)",
        "http://user:password@localhost:8081/",
        "http://localhost:8081/?token=secret",
        "http://localhost:8081/#secret",
        "http://localhost.example.com/",
        "file:///tmp/index.html",
    ] {
        let mut p = plan();
        p.variants[0].url = bad.into();
        assert!(
            case.apply(Change::AddUserTrialPlan { plan: p }).is_err(),
            "accepted {bad}"
        );
    }
    let mut p = plan();
    p.variants[1].source.commit = p.variants[0].source.commit.clone();
    assert!(case.apply(Change::AddUserTrialPlan { plan: p }).is_err());
    let mut p = plan();
    p.variants[1].url = "http://127.0.0.1:8081".into();
    assert!(case.apply(Change::AddUserTrialPlan { plan: p }).is_err());
    let mut p = plan();
    p.variants[0].evidence_ids.push("missing".into());
    assert!(case.apply(Change::AddUserTrialPlan { plan: p }).is_err());
    assert_eq!(revision_count(&case.0), 1);
}

#[test]
fn an_oss_fork_keeps_its_actual_repository_identity() {
    let case = Case::new();
    let mut p = plan();
    p.variants[1].source.repository = "research-team/openemr".into();
    let saved = case.apply(Change::AddUserTrialPlan { plan: p }).unwrap();
    assert_eq!(
        saved.workspace.user_trial_plans[0].variants[1]
            .source
            .repository,
        "research-team/openemr"
    );
}

#[test]
fn evidence_links_must_match_the_tried_commit() {
    let case = Case::new();
    case.apply(Change::BindSource {
        check_id: "save".into(),
        source: source("a"),
    })
    .unwrap();
    let mut trace = support::trace(&[mugiurushi::execution::ObservationOutcome::Pass]);
    trace.source = Some(source("a"));
    let imported = support::import(&case.0, trace);
    let evidence_id = imported.workspace.evidence[0].id.clone();
    let mut invalid = plan();
    invalid.variants[1].evidence_ids.push(evidence_id.clone());
    assert!(
        case.apply(Change::AddUserTrialPlan { plan: invalid })
            .is_err()
    );
    let mut valid = plan();
    valid.variants[0].evidence_ids.push(evidence_id);
    case.apply(Change::AddUserTrialPlan { plan: valid })
        .unwrap();
}

#[test]
fn schema_three_preserves_human_records_and_older_documents_still_load() {
    let case = Case::new();
    let initial = load(&case.0).unwrap();
    let mut old = serde_json::to_value(&initial).unwrap();
    old.as_object_mut().unwrap().remove("user_trial_plans");
    old.as_object_mut().unwrap().remove("user_trial_sessions");
    let decoded: Workspace = serde_json::from_value(old).unwrap();
    validate(&decoded).unwrap();
    assert!(decoded.user_trial_plans.is_empty());
    case.apply(Change::AddUserTrialPlan { plan: plan() })
        .unwrap();
    let changed = case
        .apply(Change::SelectProfile {
            domain: profile::Domain::Medical,
        })
        .unwrap();
    assert_eq!(changed.workspace.schema_version, 3);
    let mut downgraded = changed.workspace;
    downgraded.schema_version = 2;
    assert!(validate(&downgraded).is_err());
    assert!(
        initialize(
            &case.0.with_extension("injected"),
            downgraded,
            "合成",
            "試用履歴の混入"
        )
        .is_err()
    );
}
