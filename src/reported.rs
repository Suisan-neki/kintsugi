//! 言語・実行環境を限定しない、外部から報告された観測。実行証明には使わない。
use crate::error::invalid;
use crate::{
    execution::{Criterion, ObservationOutcome},
    model::*,
    workspace::CodeReference,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportedObservation {
    pub sequence: u64,
    pub correlation: Correlation,
    pub assertion: String,
    pub subject: String,
    pub outcome: ObservationOutcome,
    pub detail: String,
    pub source: Option<CodeReference>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportedTrace {
    pub schema_version: u32,
    pub correlation: Correlation,
    pub producer: String,
    pub origin: String,
    pub completed: bool,
    pub source: Option<CodeReference>,
    pub context: BTreeMap<String, serde_json::Value>,
    pub observations: Vec<ReportedObservation>,
}

fn text(value: &str) -> io::Result<()> {
    if value.trim().is_empty() || value.len() > 16384 {
        return Err(invalid(
            "報告の出所・識別子・内容には空でない16384バイト以内の値が必要です",
        ));
    }
    Ok(())
}
pub fn validate(trace: &ReportedTrace) -> io::Result<()> {
    if trace.schema_version != 1
        || trace.observations.len() > 2000
        || serde_json::to_vec(trace).map_err(io::Error::other)?.len() > 1024 * 1024
    {
        return Err(invalid("報告観測はschema_version 1・2000件・1MiB以内です"));
    }
    for value in [
        &trace.producer,
        &trace.origin,
        &trace.correlation.run_id,
        &trace.correlation.record_id,
        &trace.correlation.attempt_id,
    ] {
        text(value)?;
    }
    if let Some(source) = &trace.source {
        crate::workspace::validate_source(source)?;
    }
    for key in trace.context.keys() {
        text(key)?;
    }
    for (i, observation) in trace.observations.iter().enumerate() {
        if observation.sequence != i as u64 + 1 || observation.correlation != trace.correlation {
            return Err(invalid(
                "報告観測の順序または試行・データ・試行内の識別子が一致しません",
            ));
        }
        for value in [
            &observation.assertion,
            &observation.subject,
            &observation.detail,
        ] {
            text(value)?;
        }
        if let Some(source) = &observation.source {
            crate::workspace::validate_source(source)?;
        }
    }
    Ok(())
}
pub fn assess(trace: &ReportedTrace, condition: &Condition, criterion: &Criterion) -> Assessment {
    let matching: Vec<_> = trace
        .observations
        .iter()
        .filter(|o| o.assertion == criterion.assertion && o.subject == criterion.subject)
        .collect();
    let pass = matching
        .iter()
        .any(|o| o.outcome == ObservationOutcome::Pass);
    let fail = matching
        .iter()
        .any(|o| o.outcome == ObservationOutcome::Fail);
    let (verdict, reason) = if validate(trace).is_err() || !trace.completed {
        (Verdict::Unknown, "報告の形式または完了を確認できない")
    } else if pass && fail {
        (
            Verdict::Unknown,
            "同じ報告の観測名・対象について成功と失敗が食い違う",
        )
    } else if pass {
        (
            Verdict::Satisfied,
            "選んだ観測名・対象の成功が報告されている",
        )
    } else if fail {
        (
            Verdict::NotSatisfied,
            "選んだ観測名・対象の明示的な失敗が報告されている",
        )
    } else {
        (
            Verdict::Unknown,
            "選んだ観測名・対象の報告がない。処理の失敗とは断定しない",
        )
    };
    Assessment {
        condition_id: condition.id.clone(),
        condition_version: condition.version,
        verdict,
        evidence: matching
            .iter()
            .map(|o| format!("reported-observations#{}", o.sequence))
            .collect(),
        reason: reason.into(),
        unconfirmed: vec![
            "外部から受け取った報告であり、基盤が対象コードを実行・独立検証した証明ではない".into(),
            "報告の出所・コード参照・実環境との一致、観測外の状態は別途確認が必要".into(),
        ],
        next_action: "報告の出所と対象を確認し、不足・食い違いに応じて次の観測または再試験を行う"
            .into(),
    }
}
