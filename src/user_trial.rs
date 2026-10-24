//! 人がWebアプリを試した記録。自動試験の判定や合意の記録とは分ける。
use crate::workspace::{CodeReference, Workspace, validate_source};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    net::IpAddr,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Label {
    A,
    B,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Variant {
    pub label: Label,
    pub source: CodeReference,
    /// 運営者が起動した試用環境。コードの版との一致は人が確認する。
    pub url: String,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanInput {
    pub id: String,
    pub concern: String,
    pub task: String,
    /// 試験用データ、初期状態、起動・終了の手順など、人が確認した条件。
    pub conditions: String,
    pub variants: [Variant; 2],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub id: String,
    pub concern: String,
    pub task: String,
    pub conditions: String,
    pub variants: [Variant; 2],
    pub created_revision: u64,
    pub recorded_by: String,
    pub check_versions: BTreeMap<String, u32>,
    pub context: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Completed,
    Blocked,
    NotTried,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Feedback {
    pub variant: Label,
    pub outcome: Outcome,
    pub operation: String,
    pub feedback: String,
    pub recorded_by: String,
    pub recorded_revision: u64,
    pub unix_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub id: String,
    pub plan_id: String,
    /// 本人確認を行ったことを意味しない。氏名の代わりに試用者番号を使える。
    pub participant: String,
    pub order: [Label; 2],
    pub started_revision: u64,
    pub recorded_by: String,
    pub feedback: Vec<Feedback>,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
fn text(value: &str) -> io::Result<()> {
    if value.trim().is_empty() || value.len() > 16_384 {
        return Err(invalid("試用の記録は空でない16384バイト以内の値が必要です"));
    }
    Ok(())
}
fn id(value: &str) -> io::Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
    {
        return Err(invalid(
            "試用の識別子は英数字・ハイフン・下線の1〜128文字です",
        ));
    }
    Ok(())
}
fn unique_ids<'a>(values: impl Iterator<Item = &'a str>) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    for value in values {
        id(value)?;
        if !seen.insert(value) {
            return Err(invalid("試用の識別子が重複しています"));
        }
    }
    Ok(())
}
fn validate_url(value: &str) -> io::Result<url::Url> {
    let u = url::Url::parse(value).map_err(|_| invalid("試用先のURLを確認してください"))?;
    let loopback = match u.host() {
        Some(url::Host::Domain(host)) => host == "localhost",
        Some(url::Host::Ipv4(ip)) => IpAddr::V4(ip).is_loopback(),
        Some(url::Host::Ipv6(ip)) => IpAddr::V6(ip).is_loopback(),
        None => false,
    };
    if value.len() > 2048
        || !matches!(u.scheme(), "http" | "https")
        || !loopback
        || !u.username().is_empty()
        || u.password().is_some()
        || u.query().is_some()
        || u.fragment().is_some()
    {
        return Err(invalid(
            "試用先は認証情報・クエリー・フラグメントを含まないlocalhostまたはループバックIPのHTTP(S) URLにしてください",
        ));
    }
    Ok(u)
}
fn check_versions(workspace: &Workspace) -> BTreeMap<String, u32> {
    workspace
        .checks
        .iter()
        .map(|c| (c.id.clone(), c.version))
        .collect()
}
fn revision(workspace: &Workspace) -> io::Result<u64> {
    workspace
        .revision
        .checked_add(1)
        .ok_or_else(|| invalid("事例の版の上限です"))
}
fn validate_plan(workspace: &Workspace, plan: &Plan) -> io::Result<()> {
    id(&plan.id)?;
    for value in [
        &plan.concern,
        &plan.task,
        &plan.conditions,
        &plan.recorded_by,
    ] {
        text(value)?;
    }
    if plan.created_revision < 2
        || plan.created_revision > workspace.revision
        || plan.variants[0].label != Label::A
        || plan.variants[1].label != Label::B
    {
        return Err(invalid("試用計画の版とA・Bの定義を確認してください"));
    }
    if plan.variants[0].source.commit == plan.variants[1].source.commit {
        return Err(invalid(
            "比較するOSSには異なる固定コミットを指定してください。forkの変更版も登録できます",
        ));
    }
    if validate_url(&plan.variants[0].url)? == validate_url(&plan.variants[1].url)? {
        return Err(invalid("A・Bには別々の試用先URLが必要です"));
    }
    for variant in &plan.variants {
        validate_source(&variant.source)?;
        validate_url(&variant.url)?;
        unique_ids(variant.evidence_ids.iter().map(String::as_str))?;
        for evidence_id in &variant.evidence_ids {
            let evidence = workspace
                .evidence
                .iter()
                .find(|e| &e.id == evidence_id)
                .ok_or_else(|| invalid("試用計画が参照する検証結果がありません"))?;
            if !evidence.source.as_ref().is_some_and(|s| {
                s.repository == variant.source.repository && s.commit == variant.source.commit
            }) || evidence.execution.as_ref().is_some_and(|trace| {
                trace.recipe.source.repository != variant.source.repository
                    || trace.recipe.source.commit != variant.source.commit
            }) || evidence.reported.as_ref().is_some_and(|trace| {
                !trace.source.as_ref().is_some_and(|s| {
                    s.repository == variant.source.repository && s.commit == variant.source.commit
                })
            }) {
                return Err(invalid(
                    "試用する版と検証結果のリポジトリ・コミットが一致しません",
                ));
            }
        }
    }
    for (key, version) in &plan.check_versions {
        if *version == 0
            || !workspace
                .checks
                .iter()
                .any(|c| &c.id == key && c.version >= *version)
        {
            return Err(invalid("試用計画の確認事項の版を照合できません"));
        }
    }
    Ok(())
}

pub fn validate(workspace: &Workspace) -> io::Result<()> {
    if workspace.user_trial_plans.is_empty() && workspace.user_trial_sessions.is_empty() {
        return Ok(());
    }
    if workspace.schema_version < 3 {
        return Err(invalid("人による試用記録を持つ事例には形式3が必要です"));
    }
    unique_ids(workspace.user_trial_plans.iter().map(|p| p.id.as_str()))?;
    unique_ids(workspace.user_trial_sessions.iter().map(|s| s.id.as_str()))?;
    for plan in &workspace.user_trial_plans {
        validate_plan(workspace, plan)?;
    }
    for session in &workspace.user_trial_sessions {
        let plan = workspace
            .user_trial_plans
            .iter()
            .find(|p| p.id == session.plan_id)
            .ok_or_else(|| invalid("試用の対象となる計画がありません"))?;
        text(&session.participant)?;
        text(&session.recorded_by)?;
        if session.started_revision <= plan.created_revision
            || session.started_revision > workspace.revision
            || session.order[0] == session.order[1]
            || session.feedback.len() > 2
        {
            return Err(invalid("試用の開始版・操作順序を確認してください"));
        }
        let mut previous = session.started_revision;
        for (index, feedback) in session.feedback.iter().enumerate() {
            if session.order[index] != feedback.variant
                || feedback.recorded_revision <= previous
                || feedback.recorded_revision > workspace.revision
            {
                return Err(invalid("試用した順序と感想の記録が一致しません"));
            }
            for value in [
                &feedback.operation,
                &feedback.feedback,
                &feedback.recorded_by,
            ] {
                text(value)?;
            }
            previous = feedback.recorded_revision;
        }
    }
    Ok(())
}

pub fn add_plan(workspace: &mut Workspace, input: PlanInput, actor: &str) -> io::Result<()> {
    if workspace.user_trial_plans.iter().any(|p| p.id == input.id) {
        return Err(invalid(
            "試用計画は上書きできません。新しい識別子で追加してください",
        ));
    }
    let plan = Plan {
        id: input.id,
        concern: input.concern,
        task: input.task,
        conditions: input.conditions,
        variants: input.variants,
        created_revision: revision(workspace)?,
        recorded_by: actor.into(),
        check_versions: check_versions(workspace),
        context: workspace.context.clone(),
    };
    workspace.schema_version = 3;
    workspace.user_trial_plans.push(plan);
    Ok(())
}

pub fn start(
    workspace: &mut Workspace,
    plan_id: String,
    participant: String,
    order: [Label; 2],
    actor: &str,
) -> io::Result<()> {
    let plan = workspace
        .user_trial_plans
        .iter()
        .find(|p| p.id == plan_id)
        .ok_or_else(|| invalid("試用計画がありません"))?;
    if plan.check_versions != check_versions(workspace) || plan.context != workspace.context {
        return Err(invalid(
            "試用計画の作成後に条件が変わっています。新しい計画で試用してください",
        ));
    }
    workspace.user_trial_sessions.push(Session {
        id: format!("user-trial-{}", workspace.user_trial_sessions.len() + 1),
        plan_id,
        participant,
        order,
        started_revision: revision(workspace)?,
        recorded_by: actor.into(),
        feedback: vec![],
    });
    Ok(())
}

pub fn record_feedback(
    workspace: &mut Workspace,
    session_id: &str,
    variant: Label,
    outcome: Outcome,
    operation: String,
    feedback: String,
    actor: &str,
) -> io::Result<()> {
    let recorded_revision = revision(workspace)?;
    let session = workspace
        .user_trial_sessions
        .iter_mut()
        .find(|s| s.id == session_id)
        .ok_or_else(|| invalid("試用の記録がありません"))?;
    if session.order.get(session.feedback.len()) != Some(&variant) {
        return Err(invalid(
            "次に試す版が一致しません。保存した感想は上書きできません",
        ));
    }
    session.feedback.push(Feedback {
        variant,
        outcome,
        operation,
        feedback,
        recorded_by: actor.into(),
        recorded_revision,
        unix_ms: crate::recording::unix_ms()?,
    });
    Ok(())
}
