//! 版を固定した構成と、確認事項・発言・試行への参照。接続線自体を観測の証拠にしない。
use crate::error::invalid;
use crate::{
    trial::{CheckMethod, Observed},
    workspace::{CodeReference, Workspace, validate_source},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Environment {
    Trial,
    Deployed,
    Simulated,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BasisKind {
    Configuration,
    Human,
    Inference,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Basis {
    pub kind: BasisKind,
    pub note: String,
    pub source: Option<CodeReference>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElementKind {
    Person,
    System,
    App,
    Component,
    Database,
    Storage,
    Boundary,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Element {
    pub id: String,
    pub parent: Option<String>,
    pub kind: ElementKind,
    pub title: String,
    pub description: String,
    pub basis: Basis,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Relation {
    pub id: String,
    pub from: String,
    pub to: String,
    pub title: String,
    pub description: String,
    pub basis: Basis,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Model {
    pub schema_version: u32,
    pub id: String,
    pub version: u32,
    pub title: String,
    /// 構成を調べたGitの版。コンテナ内のコードと同一だと証明するものではない。
    pub source: CodeReference,
    pub environment: Environment,
    pub images: BTreeMap<String, String>,
    pub browser_version: Option<String>,
    pub elements: Vec<Element>,
    pub relations: Vec<Relation>,
    pub limits: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Target {
    Check {
        check_id: String,
        check_version: u32,
    },
    Statement {
        statement_id: String,
    },
    /// 一つの試行の中の、一つの観測条件。全体判定で部分の欠落を埋めない。
    Evidence {
        evidence_id: String,
        observation_id: String,
    },
    ReportedEvidence {
        evidence_id: String,
        assertion: String,
        subject: String,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Link {
    pub id: String,
    pub model_id: String,
    pub model_version: u32,
    pub object_id: String,
    pub target: Target,
}
#[derive(Debug, Serialize)]
pub struct LinkView {
    pub link: Link,
    pub current_model: bool,
    pub applicable: bool,
    pub method: Option<LinkMethod>,
    pub observed: Option<Observed>,
    pub detail: String,
}
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum LinkMethod {
    Execution(CheckMethod),
    Reported(ReportedMethod),
}
#[derive(Debug, PartialEq, Eq, Serialize)]
pub enum ReportedMethod {
    #[serde(rename = "reported_assertion")]
    Assertion,
}

fn id(s: &str) -> io::Result<()> {
    if s.is_empty()
        || s.len() > 128
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        return Err(invalid(
            "構成の識別子は英数字・下線・ハイフンの1〜128文字です",
        ));
    }
    Ok(())
}
fn text(s: &str) -> io::Result<()> {
    if s.trim().is_empty() || s.len() > 16384 {
        return Err(invalid("構成の説明は空でない16384バイト以内です"));
    }
    Ok(())
}
fn basis(b: &Basis) -> io::Result<()> {
    text(&b.note)?;
    if let Some(s) = &b.source {
        validate_source(s)?;
    }
    if b.kind == BasisKind::Configuration && b.source.is_none() {
        return Err(invalid("設定で確認した構成には固定版の根拠が必要です"));
    }
    Ok(())
}
impl Model {
    pub fn has_object(&self, object: &str) -> bool {
        self.elements.iter().any(|e| e.id == object)
            || self.relations.iter().any(|r| r.id == object)
    }
    pub fn validate(&self) -> io::Result<()> {
        id(&self.id)?;
        text(&self.title)?;
        validate_source(&self.source)?;
        if self.schema_version != 1
            || self.version == 0
            || self.elements.is_empty()
            || self.elements.len() > 100
            || self.relations.len() > 200
            || self.limits.is_empty()
            || self.limits.len() > 100
            || self.images.len() > 20
        {
            return Err(invalid(
                "構成は形式1・版1以上・要素1〜100・接続200以内・限界1〜100件です",
            ));
        }
        for limit in &self.limits {
            text(limit)?;
        }
        if let Some(v) = &self.browser_version {
            text(v)?;
        }
        for (service, image) in &self.images {
            id(service)?;
            let (_, digest) = image
                .rsplit_once("@sha256:")
                .ok_or_else(|| invalid("イメージのダイジェストが必要です"))?;
            if digest.len() != 64
                || !digest
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            {
                return Err(invalid("イメージのダイジェストが不正です"));
            }
        }
        let mut ids = BTreeSet::new();
        for e in &self.elements {
            id(&e.id)?;
            text(&e.title)?;
            text(&e.description)?;
            basis(&e.basis)?;
            if !ids.insert(&e.id) {
                return Err(invalid("構成の識別子が重複しています"));
            }
            let mut path = BTreeSet::from([e.id.as_str()]);
            let mut parent = e.parent.as_deref();
            while let Some(p) = parent {
                if !path.insert(p) {
                    return Err(invalid("構成の階層が循環しています"));
                }
                let ancestor = self
                    .elements
                    .iter()
                    .find(|a| a.id == p)
                    .ok_or_else(|| invalid("親要素がありません"))?;
                if !matches!(ancestor.kind, ElementKind::Boundary | ElementKind::System) {
                    return Err(invalid("親はシステムか境界です"));
                }
                parent = ancestor.parent.as_deref();
            }
        }
        for r in &self.relations {
            id(&r.id)?;
            text(&r.title)?;
            text(&r.description)?;
            basis(&r.basis)?;
            if !ids.insert(&r.id)
                || r.from == r.to
                || !self.elements.iter().any(|e| e.id == r.from)
                || !self.elements.iter().any(|e| e.id == r.to)
            {
                return Err(invalid("接続の識別子・端点が不正です"));
            }
        }
        Ok(())
    }
}
pub fn model<'a>(w: &'a Workspace, link: &Link) -> io::Result<&'a Model> {
    w.architecture_models
        .iter()
        .find(|m| m.id == link.model_id && m.version == link.model_version)
        .ok_or_else(|| invalid("参照する構成の版がありません"))
}
fn validate_link(w: &Workspace, l: &Link) -> io::Result<()> {
    id(&l.id)?;
    let m = model(w, l)?;
    if !m.has_object(&l.object_id) {
        return Err(invalid("構成の要素・接続がありません"));
    }
    match &l.target {
        Target::Check {
            check_id,
            check_version,
        } => {
            if *check_version == 0
                || !w
                    .checks
                    .iter()
                    .any(|c| c.id == *check_id && c.version >= *check_version)
            {
                return Err(invalid("参照する確認事項の版がありません"));
            }
        }
        Target::Statement { statement_id } => {
            if !w.statements.iter().any(|s| s.id == *statement_id) {
                return Err(invalid("参照する発言がありません"));
            }
        }
        Target::Evidence {
            evidence_id,
            observation_id,
        } => {
            let e = w
                .evidence
                .iter()
                .find(|e| e.id == *evidence_id)
                .ok_or_else(|| invalid("参照する試行がありません"))?;
            let trace = e
                .execution
                .as_ref()
                .ok_or_else(|| invalid("固定版実行の記録が必要です"))?;
            let spec = trace
                .recipe
                .trial
                .as_ref()
                .ok_or_else(|| invalid("試行の観測条件がありません"))?;
            if !spec.checks.iter().any(|c| c.id == *observation_id) {
                return Err(invalid("試行に指定した観測条件がありません"));
            }
            if trace.recipe.source != m.source
                || trace
                    .recipe
                    .containers
                    .as_ref()
                    .map(|c| {
                        c.services
                            .iter()
                            .map(|s| (s.name.clone(), s.image.clone()))
                            .collect::<BTreeMap<_, _>>()
                    })
                    .unwrap_or_default()
                    != m.images
                || trace
                    .recipe
                    .browser
                    .as_ref()
                    .map(|b| b.expected_version.clone())
                    != m.browser_version
            {
                return Err(invalid(
                    "試行の固定コード・イメージ・ブラウザの版が構成と一致しません",
                ));
            }
        }
        Target::ReportedEvidence {
            evidence_id,
            assertion,
            subject,
        } => {
            text(assertion)?;
            text(subject)?;
            let evidence = w
                .evidence
                .iter()
                .find(|e| e.id == *evidence_id)
                .ok_or_else(|| invalid("参照する段階の報告がありません"))?;
            let trace = evidence
                .reported
                .as_ref()
                .ok_or_else(|| invalid("報告観測の記録が必要です"))?;
            if !evidence
                .observation
                .as_ref()
                .is_some_and(|c| c.assertion == *assertion && c.subject == *subject)
            {
                return Err(invalid(
                    "この条件に対応づけた報告の観測名・対象と一致しません",
                ));
            }
            let images = trace
                .context
                .get("images")
                .and_then(|v| serde_json::from_value::<BTreeMap<String, String>>(v.clone()).ok());
            let browser = trace
                .context
                .get("browser_version")
                .and_then(|v| v.as_str())
                .map(str::to_owned);
            if trace.source.as_ref() != Some(&m.source)
                || images.as_ref() != Some(&m.images)
                || browser != m.browser_version
            {
                return Err(invalid(
                    "報告に宣言された固定コード・イメージ・ブラウザーの版が構成と一致しません",
                ));
            }
        }
    }
    Ok(())
}
pub fn validate(w: &Workspace) -> io::Result<()> {
    if w.architecture_models.len() > 100 || w.architecture_links.len() > 2000 {
        return Err(invalid("構成は100版・対応づけは2000件以内です"));
    }
    let mut versions = BTreeMap::new();
    for m in &w.architecture_models {
        m.validate()?;
        let previous = versions.entry(&m.id).or_insert(0);
        if m.version != *previous + 1 {
            return Err(invalid("構成は同じ識別子で版1から順に追加してください"));
        }
        *previous = m.version;
    }
    let mut ids = BTreeSet::new();
    for l in &w.architecture_links {
        validate_link(w, l)?;
        if !ids.insert(&l.id) {
            return Err(invalid("対応づけの識別子が重複しています"));
        }
    }
    Ok(())
}
/// 全体判定は使わず、対応する観測条件の結果だけを返す。
pub fn links(w: &Workspace) -> Vec<LinkView> {
    w.architecture_links
        .iter()
        .map(|l| {
            let current_model = !w
                .architecture_models
                .iter()
                .any(|m| m.id == l.model_id && m.version > l.model_version);
            let (applicable, method, observed, detail) = match &l.target {
                Target::Check {
                    check_id,
                    check_version,
                } => (
                    w.checks
                        .iter()
                        .any(|c| c.id == *check_id && c.version == *check_version),
                    None,
                    None,
                    "確認事項への参照。実行を観測した証拠ではありません".into(),
                ),
                Target::Statement { .. } => (
                    true,
                    None,
                    None,
                    "発言への参照。理解や合意の成立は判定しません".into(),
                ),
                Target::Evidence {
                    evidence_id,
                    observation_id,
                } => {
                    let e = w.evidence.iter().find(|e| e.id == *evidence_id);
                    let trace = e.and_then(|e| e.execution.as_ref());
                    let check = trace
                        .and_then(|t| t.recipe.trial.as_ref())
                        .and_then(|s| s.checks.iter().find(|c| c.id == *observation_id));
                    let result = trace
                        .and_then(|t| t.trial.as_ref())
                        .and_then(|s| s.results.iter().find(|r| r.id == *observation_id));
                    let applicable = e.is_some_and(|e| {
                        w.checks.iter().any(|c| {
                            c.id == e.check_id
                                && crate::workspace::evidence_invalidated_by(w, c, e).is_empty()
                        })
                    });
                    (
                        applicable,
                        check.map(|c| LinkMethod::Execution(c.method)),
                        result.map(|r| r.observed),
                        result.map_or("観測結果の記録なし".into(), |r| r.detail.clone()),
                    )
                }
                Target::ReportedEvidence {evidence_id,assertion,subject} => {
                    let e=w.evidence.iter().find(|e|e.id==*evidence_id);
                    let applicable=e.is_some_and(|e|w.checks.iter().any(|c|c.id==e.check_id && crate::workspace::evidence_invalidated_by(w,c,e).is_empty()));
                    let selected=e.and_then(|e|e.reported.as_ref().map(|trace|(e,trace)));
                    let (observed,detail)=selected.map_or((Observed::NotObserved,"段階の報告の記録なし".into()),|(e,trace)|{
                        let criterion=crate::execution::Criterion {assertion:assertion.clone(),subject:subject.clone()};
                        let assessment=crate::reported::assess(trace,&e.manifest.condition,&criterion);
                        let observed=match assessment.verdict {crate::model::Verdict::Satisfied=>Observed::Pass,crate::model::Verdict::NotSatisfied=>Observed::Fail,crate::model::Verdict::Unknown=>Observed::NotObserved};
                        (observed,format!("{}。対象内の段階報告であり、実行経路の独立した証明ではありません。段階の詳しい記録は証拠から確認できます。",assessment.reason))
                    });
                    (applicable,Some(LinkMethod::Reported(ReportedMethod::Assertion)),Some(observed),detail)
                }
            };
            LinkView {
                link: l.clone(),
                current_model,
                applicable,
                method,
                observed,
                detail,
            }
        })
        .collect()
}
