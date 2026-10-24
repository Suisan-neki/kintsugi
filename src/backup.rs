//! 事例の全版を持ち出し、既存を置き換えずに復元する。試行の作業領域は含めない。
use crate::{
    recording::write_json_new,
    workspace::{self, Workspace},
};
use serde::{Deserialize, Serialize};
use std::{fs, io, path::Path};
pub const MAX_BACKUP: u64 = 32 * 1024 * 1024;
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Backup {
    pub schema_version: u32,
    pub case_id: String,
    pub revisions: Vec<Workspace>,
}
fn invalid(text: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, text)
}
fn same(a: &(impl Serialize + ?Sized), b: &(impl Serialize + ?Sized)) -> io::Result<bool> {
    Ok(serde_json::to_value(a).map_err(io::Error::other)?
        == serde_json::to_value(b).map_err(io::Error::other)?)
}

fn validate_trial_history(previous: &Workspace, current: &Workspace) -> io::Result<()> {
    let plans = previous.user_trial_plans.len();
    if current.user_trial_plans.len() < plans
        || !same(
            &current.user_trial_plans[..plans],
            &previous.user_trial_plans,
        )?
        || current.user_trial_sessions.len() < previous.user_trial_sessions.len()
    {
        return Err(invalid("以前の試用計画・試用記録が書き換えられています"));
    }
    if current.user_trial_plans[plans..]
        .iter()
        .any(|plan| plan.created_revision != current.revision)
        || current.user_trial_sessions[previous.user_trial_sessions.len()..]
            .iter()
            .any(|session| {
                session.started_revision != current.revision || !session.feedback.is_empty()
            })
    {
        return Err(invalid(
            "試用計画・開始記録の版が最初に追加された版と一致しません",
        ));
    }
    for (old, new) in previous
        .user_trial_sessions
        .iter()
        .zip(&current.user_trial_sessions)
    {
        let mut metadata = new.clone();
        metadata.feedback = old.feedback.clone();
        if !same(old, &metadata)?
            || new.feedback.len() < old.feedback.len()
            || !same(&new.feedback[..old.feedback.len()], &old.feedback)?
        {
            return Err(invalid(
                "以前の試用者・操作順序・感想が書き換えられています",
            ));
        }
        if new.feedback.len() > old.feedback.len() + 1
            || new.feedback[old.feedback.len()..]
                .iter()
                .any(|feedback| feedback.recorded_revision != current.revision)
        {
            return Err(invalid("感想の記録版が最初に追加された版と一致しません"));
        }
    }
    Ok(())
}

fn trial_conditions_match(plan: &crate::user_trial::Plan, workspace: &Workspace) -> bool {
    plan.context == workspace.context
        && plan.check_versions.len() == workspace.checks.len()
        && workspace
            .checks
            .iter()
            .all(|check| plan.check_versions.get(&check.id) == Some(&check.version))
}

fn validate_trial_origins(backup: &Backup, workspace: &Workspace) -> io::Result<()> {
    for plan in &workspace.user_trial_plans {
        let before = &backup.revisions[plan.created_revision as usize - 2];
        let created = &backup.revisions[plan.created_revision as usize - 1];
        if !trial_conditions_match(plan, before)
            || created.history.last().map(|audit| &audit.actor) != Some(&plan.recorded_by)
            || !created.user_trial_plans.iter().any(|p| p.id == plan.id)
            || plan.variants.iter().any(|variant| {
                variant
                    .evidence_ids
                    .iter()
                    .any(|id| !before.evidence.iter().any(|e| &e.id == id))
            })
        {
            return Err(invalid(
                "試用計画の条件・検証結果・記入者が作成時の事例版と一致しません",
            ));
        }
    }
    for session in &workspace.user_trial_sessions {
        // workspace::validateが計画の存在と開始版の範囲を先に確認する。
        let plan = workspace
            .user_trial_plans
            .iter()
            .find(|p| p.id == session.plan_id)
            .ok_or_else(|| invalid("試用計画がありません"))?;
        let before = &backup.revisions[session.started_revision as usize - 2];
        let started = &backup.revisions[session.started_revision as usize - 1];
        if !trial_conditions_match(plan, before)
            || started.history.last().map(|audit| &audit.actor) != Some(&session.recorded_by)
            || !started
                .user_trial_sessions
                .iter()
                .any(|s| s.id == session.id)
        {
            return Err(invalid("試用の条件・記入者が開始時の事例版と一致しません"));
        }
        for feedback in &session.feedback {
            let recorded = &backup.revisions[feedback.recorded_revision as usize - 1];
            if recorded.history.last().map(|audit| &audit.actor) != Some(&feedback.recorded_by)
                || !recorded.user_trial_sessions.iter().any(|s| {
                    s.id == session.id
                        && s.feedback
                            .iter()
                            .any(|f| f.recorded_revision == feedback.recorded_revision)
                })
            {
                return Err(invalid("感想の記入者・記録時の事例版が一致しません"));
            }
        }
    }
    Ok(())
}
pub fn validate(backup: &Backup) -> io::Result<()> {
    if backup.schema_version != 1
        || backup.revisions.is_empty()
        || backup.revisions.len() > 1000
        || serde_json::to_vec_pretty(backup)
            .map_err(io::Error::other)?
            .len() as u64
            + 1
            > MAX_BACKUP
    {
        return Err(invalid("バックアップは形式1・1〜1000版・32MiB以内です"));
    }
    for (i, w) in backup.revisions.iter().enumerate() {
        workspace::validate(w)?;
        if w.id != backup.case_id
            || w.revision != i as u64 + 1
            || w.history.len() != i + 1
            || w.history.iter().enumerate().any(|(j, h)| {
                h.revision != j as u64 + 1
                    || h.actor.trim().is_empty()
                    || h.reason.trim().is_empty()
            })
            || serde_json::to_vec_pretty(w)
                .map_err(io::Error::other)?
                .len()
                + 1
                > 4 * 1024 * 1024
        {
            return Err(invalid("バックアップの事例・版・更新履歴が一致しません"));
        }
        validate_trial_origins(backup, w)?;
        if i > 0 {
            let previous = &backup.revisions[i - 1];
            validate_trial_history(previous, w)?;
            if w.profile_history.len() < previous.profile_history.len()
                || !same(
                    &w.profile_history[..previous.profile_history.len()],
                    &previous.profile_history,
                )?
                || w.architecture_models.len() < previous.architecture_models.len()
                || w.architecture_links.len() < previous.architecture_links.len()
                || !same(
                    &w.architecture_models[..previous.architecture_models.len()],
                    &previous.architecture_models,
                )?
                || !same(
                    &w.architecture_links[..previous.architecture_links.len()],
                    &previous.architecture_links,
                )?
                || w.evidence.len() < previous.evidence.len()
                || w.decisions.len() < previous.decisions.len()
                || !same(&w.evidence[..previous.evidence.len()], &previous.evidence)?
                || !same(
                    &w.decisions[..previous.decisions.len()],
                    &previous.decisions,
                )?
                || !same(&w.history[..i], &previous.history)?
            {
                return Err(invalid("以前の根拠・判断・更新履歴が書き換えられています"));
            }
        }
        for decision in &w.decisions {
            if decision.based_on_revision == 0
                || decision.based_on_revision >= w.revision
                || decision.check_ids.is_empty()
                || decision.confidence.is_some_and(|v| v > 5)
                || decision.elapsed_ms.is_some_and(|v| v > 86400000)
            {
                return Err(invalid("判断の参照版と記録範囲が不正です"));
            }
            let old = &backup.revisions[decision.based_on_revision as usize - 1];
            if decision.basis.len() != decision.check_ids.len()
                || decision.actor.trim().is_empty()
                || decision.reason.trim().is_empty()
            {
                return Err(invalid("判断の条件と記入者・理由が一致しません"));
            }
            let mut seen = std::collections::BTreeSet::new();
            for basis in &decision.basis {
                let check = old
                    .checks
                    .iter()
                    .find(|c| c.id == basis.check_id)
                    .ok_or_else(|| invalid("判断の条件が参照版にありません"))?;
                if !seen.insert(&basis.check_id)
                    || !decision.check_ids.contains(&basis.check_id)
                    || basis.check_version != check.version
                {
                    return Err(invalid("判断の条件版・対応づけが不正です"));
                }
                let mut evidence_ids = std::collections::BTreeSet::new();
                for view in &basis.evidence {
                    let evidence = old
                        .evidence
                        .iter()
                        .find(|e| e.id == view.id && e.check_id == check.id)
                        .ok_or_else(|| invalid("判断の根拠が参照版にありません"))?;
                    if !evidence_ids.insert(&view.id)
                        || view.run_id != evidence.manifest.correlation.run_id
                        || view.observed.condition_id != check.id
                        || view.observed.condition_version != evidence.check_version
                    {
                        return Err(invalid("判断の根拠・試行・条件版が一致しません"));
                    }
                }
                if basis
                    .selected_evidence
                    .as_ref()
                    .is_some_and(|id| !evidence_ids.contains(id))
                {
                    return Err(invalid("判断が選んだ根拠がありません"));
                }
                for question in &basis.unanswered {
                    if !check
                        .questions
                        .iter()
                        .any(|q| q.id == question.id && same(q, question).unwrap_or(false))
                    {
                        return Err(invalid("判断の未回答事項が参照版にありません"));
                    }
                }
            }
        }
    }
    Ok(())
}
pub fn snapshot(directory: &Path) -> io::Result<Backup> {
    let latest = workspace::load(directory)?;
    if latest.revision > 1000 {
        return Err(invalid("1000版を超える事例は分割して保存してください"));
    }
    let mut revisions = vec![];
    let mut bytes = 0;
    // 最新版より後の並行更新は含めず、開始時に読んだ固定版までを保存する。
    for version in 1..=latest.revision {
        let path = directory
            .join("revisions")
            .join(format!("{version:020}.json"));
        bytes += fs::metadata(&path)?.len();
        if bytes > MAX_BACKUP {
            return Err(invalid("バックアップが32MiBを超えています"));
        }
        revisions.push(crate::recording::read_json(&path)?);
    }
    let result = Backup {
        schema_version: 1,
        case_id: latest.id,
        revisions,
    };
    validate(&result)?;
    Ok(result)
}
pub fn read(path: &Path) -> io::Result<Backup> {
    if fs::metadata(path)?.len() > MAX_BACKUP {
        return Err(invalid("バックアップが32MiBを超えています"));
    }
    let backup = crate::recording::read_json(path)?;
    validate(&backup)?;
    Ok(backup)
}
pub fn restore(directory: &Path, backup: &Backup) -> io::Result<workspace::WorkspaceView> {
    validate(backup)?;
    if let Some(parent) = directory.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir(directory)?;
    write_json_new(
        &directory.join("restore.pending"),
        &serde_json::json!({"schema_version":1,"case_id":backup.case_id}),
    )?;
    fs::create_dir(directory.join("revisions"))?;
    for w in &backup.revisions {
        workspace::save_revision(directory, w)?;
    }
    fs::remove_file(directory.join("restore.pending"))?;
    Ok(workspace::view(workspace::load(directory)?))
}
