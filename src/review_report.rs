//! 固定した一つの事例版から、確認相手へ見せる資料を作る。原文ログや認証情報は出力しない。
use crate::{
    profile,
    workspace::{self, Check, CheckView, CodeReference, Decision, Workspace},
};
use serde::Serialize;
use std::{collections::BTreeMap, io};

#[derive(Debug, Serialize)]
pub struct ReportObservation {
    pub id: String,
    pub scope: String,
    pub source: String,
    pub expected: String,
    pub observed: Option<crate::trial::Observed>,
}
#[derive(Debug, Serialize)]
pub struct EvidenceReference {
    pub id: String,
    pub run_id: String,
    pub check_version: u32,
    pub source: Option<CodeReference>,
    pub review_profile: Option<profile::Selection>,
    pub observations: Vec<ReportObservation>,
    pub kind: &'static str,
    pub scope: String,
}
#[derive(Debug, Serialize)]
pub struct ReportCheck {
    pub definition: Check,
    pub current: CheckView,
    pub references: Vec<EvidenceReference>,
}
/// 試用先URLと実行環境の内部設定は共有資料へ含めない。
#[derive(Debug, Serialize)]
pub struct ReportTrialVariant {
    pub label: crate::user_trial::Label,
    pub source: CodeReference,
    pub evidence_ids: Vec<String>,
}
#[derive(Debug, Serialize)]
pub struct ReportTrialPlan {
    pub id: String,
    pub concern: String,
    pub task: String,
    pub conditions: String,
    pub variants: [ReportTrialVariant; 2],
    pub created_revision: u64,
    pub recorded_by: String,
    pub check_versions: BTreeMap<String, u32>,
}

impl From<crate::user_trial::Plan> for ReportTrialPlan {
    fn from(plan: crate::user_trial::Plan) -> Self {
        Self {
            id: plan.id,
            concern: plan.concern,
            task: plan.task,
            conditions: plan.conditions,
            variants: plan.variants.map(|variant| ReportTrialVariant {
                label: variant.label,
                source: variant.source,
                evidence_ids: variant.evidence_ids,
            }),
            created_revision: plan.created_revision,
            recorded_by: plan.recorded_by,
            check_versions: plan.check_versions,
        }
    }
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub schema_version: u32,
    pub case_id: String,
    pub title: String,
    pub revision: u64,
    pub verification_passed: bool,
    pub domain: profile::Domain,
    pub selection_version: u32,
    pub catalog_version: &'static str,
    pub profile_history: Vec<profile::Selection>,
    pub checks: Vec<ReportCheck>,
    pub decisions: Vec<Decision>,
    pub statements: Vec<crate::workspace::Statement>,
    pub architecture_models: Vec<crate::architecture::Model>,
    pub architecture_links: Vec<crate::architecture::Link>,
    pub user_trial_plans: Vec<ReportTrialPlan>,
    pub user_trial_sessions: Vec<crate::user_trial::Session>,
    pub limits: Vec<&'static str>,
}
pub fn build(workspace: Workspace) -> io::Result<Report> {
    workspace::validate(&workspace)?;
    let (domain, selection_version) = profile::current(&workspace);
    let mut view = workspace::view(workspace);
    let checks = view
        .checks
        .into_iter()
        .map(|mut current| {
            let definition = view
                .workspace
                .checks
                .iter()
                .find(|c| c.id == current.check_id)
                .ok_or_else(|| io::Error::other("確認資料の条件がありません"))?
                .clone();
            let references = view
                .workspace
                .evidence
                .iter()
                .filter(|e| e.check_id == current.check_id)
                .map(|e| EvidenceReference {
                    id: e.id.clone(),
                    run_id: e.manifest.correlation.run_id.clone(),
                    check_version: e.check_version,
                    source: e.source.clone(),
                    review_profile: e.review_profile.clone(),
                    observations: current
                        .evidence
                        .iter()
                        .find(|v| v.id == e.id)
                        .and_then(|v| v.summary.as_ref())
                        .map_or_else(Vec::new, |s| {
                            s.items
                                .iter()
                                .map(|i| ReportObservation {
                                    id: i.id.clone(),
                                    scope: i.scope.clone(),
                                    source: i.source.clone(),
                                    expected: i.expected.clone(),
                                    observed: i.observed,
                                })
                                .collect()
                        }),
                    kind: if e.reported.is_some() {
                        "外部からの報告。基盤による実行・独立検証ではない"
                    } else if e.execution.is_some() {
                        "固定版の実行記録。観測方法ごとの確認範囲に限る"
                    } else {
                        "既存の試行記録。対象と確認範囲は記録を参照"
                    },
                    scope: e.manifest.observation_scope.clone(),
                })
                .collect();
            for evidence in &mut current.evidence {
                evidence.summary = None;
            }
            Ok(ReportCheck {
                definition,
                current,
                references,
            })
        })
        .collect::<io::Result<Vec<_>>>()?;
    for decision in &mut view.workspace.decisions {
        for check in &mut decision.basis {
            for evidence in &mut check.evidence {
                evidence.summary = None;
            }
        }
    }
    Ok(Report {
        verification_passed: view.verification_passed,
        schema_version: 1,
        case_id: view.workspace.id,
        title: view.workspace.title,
        revision: view.workspace.revision,
        domain,
        selection_version,
        catalog_version: profile::CATALOG_VERSION,
        profile_history: view.workspace.profile_history,
        checks,
        decisions: view.workspace.decisions,
        statements: view.workspace.statements,
        architecture_models: view.workspace.architecture_models,
        architecture_links: view.workspace.architecture_links,
        user_trial_plans: view
            .workspace
            .user_trial_plans
            .into_iter()
            .map(Into::into)
            .collect(),
        user_trial_sessions: view.workspace.user_trial_sessions,
        limits: vec![
            "この資料は記載した事例版の確認結果です。システム全体の安全性や医療業務への適合を保証するものではありません。",
            "確認できていない範囲と、観測していないことによる判定不能を残しています。",
            "原文ログ・入力データ・認証情報・画面写真は含めません。条件文・構成の説明・質問・判断理由に記入された情報は含まれるため、共有前に内容を確認してください。",
            "元の事例は更新される可能性があります。根拠IDと試行IDを使い、同じ事例版の記録で照合してください。",
            "人による試用の感想は自己申告です。自動試験の判定や合意とは区別します。試用先URLと内部の実行条件は省き、手順として記入した条件を含めます。共有前に試用者番号・操作・感想も確認してください。",
        ],
    })
}
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn verdict(v: crate::model::Verdict) -> &'static str {
    match v {
        crate::model::Verdict::Satisfied => "条件を満たす",
        crate::model::Verdict::NotSatisfied => "条件を満たさない",
        crate::model::Verdict::Unknown => "検証未通過（証拠不足・判定不能）",
    }
}

fn trial_label(label: crate::user_trial::Label) -> &'static str {
    match label {
        crate::user_trial::Label::A => "A",
        crate::user_trial::Label::B => "B",
    }
}

fn user_trials(out: &mut String, report: &Report) {
    if report.user_trial_plans.is_empty() {
        return;
    }
    out.push_str("<h2>人によるWebアプリの試用</h2><p>操作と感想は自己申告です。自動試験の判定、保存状態の独立した確認、利用への合意とは区別します。</p>");
    for plan in &report.user_trial_plans {
        out.push_str(&format!(
            "<section><h3>試用計画 {}</h3><p>相談：{}</p><p>試す操作：{}</p><p>試用条件・手順：{}</p><p>作成時の事例版{}／記入者：{}</p><ul>",
            escape(&plan.id), escape(&plan.concern), escape(&plan.task),
            escape(&plan.conditions), plan.created_revision, escape(&plan.recorded_by)
        ));
        for variant in &plan.variants {
            out.push_str(&format!(
                "<li>{}：{}／{}／{} {}〜{}行目。検証結果の参照：{}</li>",
                trial_label(variant.label),
                escape(&variant.source.repository),
                escape(&variant.source.commit),
                escape(&variant.source.path),
                variant.source.line,
                variant.source.end_line,
                if variant.evidence_ids.is_empty() {
                    "記録なし".into()
                } else {
                    escape(&variant.evidence_ids.join("、"))
                }
            ));
        }
        out.push_str("</ul>");
        let sessions: Vec<_> = report
            .user_trial_sessions
            .iter()
            .filter(|session| session.plan_id == plan.id)
            .collect();
        if sessions.is_empty() {
            out.push_str("<p>この計画で試用を開始した記録はありません。</p>");
        }
        for session in sessions {
            out.push_str(&format!(
                "<h4>試用 {}／試用者 {}</h4><p>開始時の事例版{}／操作順 {}→{}。感想の記録：{}件／2件</p>",
                escape(&session.id), escape(&session.participant), session.started_revision,
                trial_label(session.order[0]), trial_label(session.order[1]), session.feedback.len()
            ));
            for feedback in &session.feedback {
                let outcome = match feedback.outcome {
                    crate::user_trial::Outcome::Completed => "操作を完了したと申告",
                    crate::user_trial::Outcome::Blocked => "途中で進めなくなったと申告",
                    crate::user_trial::Outcome::NotTried => "試していないと申告",
                };
                out.push_str(&format!(
                    "<p>{}：{outcome}</p><p>行った操作：{}</p><p>感想・残る困りごと：{}</p><p>記録時の事例版{}／記入者：{}／時刻 {} Unixミリ秒</p>",
                    trial_label(feedback.variant), escape(&feedback.operation), escape(&feedback.feedback),
                    feedback.recorded_revision, escape(&feedback.recorded_by), feedback.unix_ms
                ));
            }
        }
        out.push_str("</section>");
    }
}
pub fn html(report: &Report) -> io::Result<String> {
    let mut out = format!(
        "<!doctype html><html lang=\"ja\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><style>{}\n{}\nbody{{margin:0 auto;padding:24px;max-width:960px;overflow-wrap:anywhere}}section{{border-top:1px solid #bbb;margin-top:24px;padding-top:16px}}pre{{white-space:pre-wrap}}summary{{cursor:pointer}}li{{margin:8px 0}}</style><main><h1>{}</h1><p>事例 {}／版{}／{}モード／選択版{}</p><h2>この資料で確認できること</h2><ul>",
        escape(&report.title),
        include_str!("../ui/dads/tokens.css"),
        include_str!("../ui/dads/components.css"),
        escape(&report.title),
        escape(&report.case_id),
        report.revision,
        report.domain.label(),
        report.selection_version
    );
    for limit in &report.limits {
        out.push_str(&format!("<li>{}</li>", escape(limit)));
    }
    out.push_str(if report.verification_passed {
        "</ul><p>記録した全条件の検証を通過。実運用の許可やシステム全体の安全性を示すものではありません。</p><h2>確認事項と結果</h2>"
    } else {
        "</ul><p>検証未通過。未確認または未達の条件があり、「採用する」という判断は記録できません。</p><h2>確認事項と結果</h2>"
    });
    if report.checks.is_empty() {
        out.push_str("<p>確認事項はまだありません。</p>");
    }
    for check in &report.checks {
        out.push_str(&format!(
            "<section><h3>{}</h3><p>{}</p><p>{}</p><p>条件 {}／版{}。出所：{}</p>",
            escape(&check.definition.text),
            verdict(check.current.verdict.clone()),
            escape(&check.current.reason),
            escape(&check.definition.id),
            check.definition.version,
            escape(&check.definition.origin)
        ));
        out.push_str("<h4>追加で確かめること</h4>");
        out.push_str(&format!(
            "<p>{}</p><p>{}</p>",
            escape(&check.current.next.text),
            escape(&check.current.next.reason)
        ));
        for question in &check.current.unanswered {
            out.push_str(&format!("<p>未回答：{}</p>", escape(&question.text)));
        }
        for evidence in &check.current.evidence {
            out.push_str(&format!(
                "<details><summary>根拠 {}／試行 {}：{}。{}</summary><p>{}</p><ul>",
                escape(&evidence.id),
                escape(&evidence.run_id),
                verdict(evidence.observed.verdict.clone()),
                if evidence.applicable {
                    "現在の条件に適用できる"
                } else {
                    "現在の条件には再確認が必要"
                },
                escape(&evidence.observed.reason)
            ));
            for item in &evidence.observed.unconfirmed {
                out.push_str(&format!("<li>未確認：{}</li>", escape(item)));
            }
            for item in &evidence.invalidated_by {
                out.push_str(&format!("<li>再確認の理由：{}</li>", escape(item)));
            }
            if let Some(reference) = check.references.iter().find(|r| r.id == evidence.id) {
                out.push_str(&format!(
                    "<li>{}</li><li>観測範囲：{}</li>",
                    escape(reference.kind),
                    escape(&reference.scope)
                ));
                for item in &reference.observations {
                    out.push_str(&format!(
                        "<li>{}：{}。観測元：{}。確認条件：{}</li>",
                        escape(&item.scope),
                        item.observed
                            .map_or("観測なし", crate::trial::observed_label),
                        escape(&item.source),
                        escape(&item.expected)
                    ));
                }
                if let Some(source) = &reference.source {
                    out.push_str(&format!(
                        "<li>固定版：{}／{}／{} {}〜{}行目</li>",
                        escape(&source.repository),
                        escape(&source.commit),
                        escape(&source.path),
                        source.line,
                        source.end_line
                    ));
                }
                out.push_str(&format!(
                    "<li>取り込み時のモード：{}</li>",
                    reference.review_profile.as_ref().map_or(
                        "記録なし（旧形式または未選択）".into(),
                        |s| format!(
                            "{}／選択版{}／カタログ{}",
                            s.domain.label(),
                            s.version,
                            escape(&s.catalog_version)
                        )
                    )
                ));
            }
            out.push_str("</ul></details>");
        }
        out.push_str("</section>");
    }
    user_trials(&mut out, report);
    if !report.architecture_models.is_empty() {
        out.push_str("<h2>構成と確認範囲</h2>");
        for model in &report.architecture_models {
            out.push_str(&format!(
                "<details><summary>{}／構成版{}</summary><ul>",
                escape(&model.title),
                model.version
            ));
            for element in &model.elements {
                out.push_str(&format!(
                    "<li>{}：{}</li>",
                    escape(&element.title),
                    escape(&element.description)
                ));
            }
            for relation in &model.relations {
                out.push_str(&format!(
                    "<li>接続 {} → {}：{}</li>",
                    escape(&relation.from),
                    escape(&relation.to),
                    escape(&relation.description)
                ));
            }
            for limit in &model.limits {
                out.push_str(&format!("<li>未確認の範囲：{}</li>", escape(limit)));
            }
            out.push_str("</ul></details>");
        }
    }
    out.push_str("<h2>質問と説明</h2>");
    if report.statements.is_empty() {
        out.push_str("<p>質問や説明はまだ記録されていません。</p>");
    }
    for statement in &report.statements {
        out.push_str(&format!(
            "<p>{}</p><p>出所：{}</p>",
            escape(&statement.text),
            escape(&statement.origin)
        ));
    }
    out.push_str("<h2>判断の履歴</h2>");
    if report.decisions.is_empty() {
        out.push_str("<p>人による判断はまだ記録されていません。</p>");
    }
    for decision in &report.decisions {
        let action = match decision.action {
            crate::workspace::DecisionKind::Release => "採用する",
            crate::workspace::DecisionKind::Hold => "保留する",
            crate::workspace::DecisionKind::Modify => "修正する",
            crate::workspace::DecisionKind::Verify => "追加で確かめる",
            crate::workspace::DecisionKind::Ask => "質問する",
        };
        out.push_str(&format!("<p>判断：{action}</p>"));
        out.push_str(&format!(
            "<p>事例版{}での判断／{}：{}</p>",
            decision.based_on_revision,
            escape(&decision.actor),
            escape(&decision.reason)
        ));
    }
    // JSONはHTMLとして挿入せず、すべての文字をエスケープした技術資料として残す。
    out.push_str(&format!("<details><summary>条件・規則・根拠参照・判断当時の結果を詳しく読む</summary><pre>{}</pre></details></main></html>", escape(&serde_json::to_string_pretty(report).map_err(io::Error::other)?)));
    Ok(out)
}
