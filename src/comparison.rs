//! 同じ確認事項の二つの取り込みを比較する。判定はworkspaceの規則を共有する。
use crate::error::invalid;
use crate::workspace::{self, CodeReference, Evidence, EvidenceView, Workspace};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeSet, io};
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompareRequest {
    pub expected_revision: u64,
    pub check_id: String,
    pub before_id: String,
    pub after_id: String,
}
#[derive(Debug, Serialize)]
pub struct Difference {
    pub key: String,
    pub status: String,
    pub before: Option<Value>,
    pub after: Option<Value>,
}
#[derive(Debug, Serialize)]
pub struct TrialSummary {
    pub evidence: EvidenceView,
    pub source: Option<CodeReference>,
    pub execution_source: Option<CodeReference>,
    pub reported_producer: Option<String>,
}
#[derive(Debug, Serialize)]
pub struct TrialComparison {
    pub schema_version: u32,
    pub case_id: String,
    pub case_revision: u64,
    pub check_id: String,
    pub before: TrialSummary,
    pub after: TrialSummary,
    pub differences: Vec<Difference>,
    pub limits: Vec<String>,
}

pub fn compare(workspace: Workspace, request: CompareRequest) -> io::Result<TrialComparison> {
    workspace::validate(&workspace)?;
    if workspace.revision != request.expected_revision {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "事例が更新されています。最新版を読み直してください",
        ));
    }
    let find = |id: &str| {
        workspace
            .evidence
            .iter()
            .find(|e| e.id == id && e.check_id == request.check_id)
            .ok_or_else(|| invalid("同じ確認事項に取り込んだ試行を選んでください"))
    };
    let a = find(&request.before_id)?;
    let b = find(&request.after_id)?;
    if a.id == b.id || a.manifest.correlation.run_id == b.manifest.correlation.run_id {
        return Err(invalid("二つの異なる試行を選んでください"));
    }
    // 試行ごとに変わる架空の値は、比較の前に共通の表記へ戻す。
    let masked = |e: &Evidence| -> io::Result<Value> {
        let mut text = serde_json::to_string(e).map_err(io::Error::other)?;
        if let Some(outcome) = e.execution.as_ref().and_then(|t| t.trial.as_ref()) {
            text = text.replace(&outcome.test_value, crate::trial::TEST_VALUE);
        }
        serde_json::from_str(&text).map_err(io::Error::other)
    };
    let av = masked(a)?;
    let bv = masked(b)?;
    let mut differences = vec![];
    let mut add = |key: String, old: Option<Value>, new: Option<Value>| {
        let status = if old.is_none() || new.is_none() {
            "unknown"
        } else if old == new {
            "same"
        } else {
            "changed"
        };
        differences.push(Difference {
            key,
            status: status.into(),
            before: old,
            after: new,
        });
    };
    for key in [
        "/check_version",
        "/rule",
        "/source",
        "/observation",
        "/manifest/condition",
        "/manifest/source_fingerprint",
        "/manifest/program_version",
        "/manifest/mode",
        "/manifest/command",
        "/manifest/input",
        "/manifest/fault",
        "/manifest/os",
        "/manifest/architecture",
        "/manifest/timeout_ms",
        "/manifest/initial_storage",
        "/manifest/observation_scope",
    ] {
        let nullable = ["/manifest/input", "/manifest/fault"].contains(&key);
        let value = |v: &Value| v.pointer(key).filter(|v| nullable || !v.is_null()).cloned();
        add(key.into(), value(&av), value(&bv));
    }
    for parent in ["execution", "reported", "adapter"] {
        if av[parent].is_null() && bv[parent].is_null() {
            continue;
        }
        let fields: &[&str] = match parent {
            "execution" => &[
                "recipe",
                "tree",
                "runtime_path",
                "runtime_version",
                "mode",
                "exit_code",
                "trial",
            ],
            "reported" => &["producer", "origin", "source", "context"],
            _ => &["target"],
        };
        for field in fields {
            let value = |v: &Value| {
                let mut result = v[parent].get(*field)?.clone();
                if parent == "adapter"
                    && let Some(object) = result.as_object_mut()
                {
                    object.remove("correlation");
                }
                (!result.is_null()).then_some(result)
            };
            add(format!("/{parent}/{field}"), value(&av), value(&bv));
        }
    }
    let keys: BTreeSet<_> = a.dependencies.keys().chain(b.dependencies.keys()).collect();
    add(
        "/dependency_keys".into(),
        Some(serde_json::json!(a.dependencies.keys().collect::<Vec<_>>())),
        Some(serde_json::json!(b.dependencies.keys().collect::<Vec<_>>())),
    );
    for key in keys {
        let value = |e: &Evidence| {
            e.dependencies.get(key).and_then(|v| {
                if key.starts_with("execution-") || key.starts_with("report-") {
                    serde_json::from_str(v).ok()
                } else {
                    Some(Value::String(v.clone()))
                }
            })
        };
        add(format!("/dependencies/{key}"), value(a), value(b));
        add(
            format!("/dependency_origins/{key}"),
            a.dependency_origins
                .get(key)
                .map(|s| Value::String(s.clone())),
            b.dependency_origins
                .get(key)
                .map(|s| Value::String(s.clone())),
        );
    }
    let view = workspace::view(workspace.clone());
    let check = view
        .checks
        .iter()
        .find(|c| c.check_id == request.check_id)
        .ok_or_else(|| invalid("確認事項がありません"))?;
    let summary = |e: &Evidence| -> io::Result<TrialSummary> {
        Ok(TrialSummary {
            evidence: check
                .evidence
                .iter()
                .find(|v| v.id == e.id)
                .ok_or_else(|| invalid("根拠の判定がありません"))?
                .clone(),
            source: e.source.clone(),
            execution_source: e.execution.as_ref().map(|t| t.recipe.source.clone()),
            reported_producer: e.reported.as_ref().map(|t| t.producer.clone()),
        })
    };
    Ok(TrialComparison {
        schema_version: 1,
        case_id: workspace.id.clone(),
        case_revision: workspace.revision,
        check_id: request.check_id,
        before: summary(a)?,
        after: summary(b)?,
        differences,
        limits: vec![
            "取り込み時の記録の比較。観測時の判定と現在の条件への適用範囲を区別する".into(),
            "結果の変化からコード改修の因果効果を特定しない。記録外の依存先・実環境は未確認".into(),
        ],
    })
}
