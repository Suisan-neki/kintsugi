//! 確認する観点を選ぶ。モードの選択は試験の成功や医療業務への適合を意味しない。
use crate::error::invalid;
use crate::workspace::{Check, Rule, Workspace};
use serde::{Deserialize, Serialize};
use std::io;

pub const CATALOG_VERSION: &str = "2026-10-18.1";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Domain {
    #[default]
    General,
    Medical,
}
impl Domain {
    pub fn label(self) -> &'static str {
        match self {
            Self::General => "汎用",
            Self::Medical => "医療",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub version: u32,
    pub domain: Domain,
    pub catalog_version: String,
    pub recorded_revision: u64,
}

#[derive(Debug, Serialize)]
pub struct Template {
    pub id: &'static str,
    pub domain: Domain,
    pub text: &'static str,
    pub plain: &'static str,
    pub technical: &'static str,
}

pub fn templates() -> Vec<Template> {
    vec![
        Template {
            id: "general-data",
            domain: Domain::General,
            text: "扱うデータと保存先を説明できる",
            plain: "何を入力し、どこに残るかを確認します。ログやバックアップも確認先の候補です。",
            technical: "入力、永続化、ログ、バックアップの境界と根拠を対応づけます。静的解析と実測を区別します。",
        },
        Template {
            id: "general-network",
            domain: Domain::General,
            text: "外部へ送る情報と接続先を確認できる",
            plain: "外へ送っている情報と送り先を確かめます。観測していない通信は未確認として残します。",
            technical: "設定上の接続先と試行中に観測した通信を分離し、未観測のプロトコルと期間を記録します。",
        },
        Template {
            id: "general-recovery",
            domain: Domain::General,
            text: "停止後に残るデータと復旧の手順を確認できる",
            plain: "止まった後に何が残るかを確認し、再起動して必要な状態に戻せるか試します。",
            technical: "永続状態、終了時の状態、再起動後の読出しを照合します。新規環境での再試験と既存状態の復旧を区別します。",
        },
        Template {
            id: "medical-display",
            domain: Domain::Medical,
            text: "表示する情報と表示を見る人を確認できる",
            plain: "患者や職員など、誰に何を見せる必要があるか確認します。実際の担当者への確認が必要です。",
            technical: "表示項目、閲覧主体、対象者との対応を業務条件として確認し、誤表示・取り違えを別に試験します。",
        },
        Template {
            id: "medical-access",
            domain: Domain::Medical,
            text: "必要な担当者の閲覧・変更範囲を確認できる",
            plain: "誰が見たり変更したりできるかを確認します。モードの選択だけで権限は設定されません。",
            technical: "認証と職種・利用者別の認可を分離して検証します。必要な権限は実際の担当者から確認します。",
        },
        Template {
            id: "medical-retention",
            domain: Domain::Medical,
            text: "記録を残す期間と削除・復旧の扱いを確認できる",
            plain: "何をいつまで残す必要があるか確認します。必要な期間はこの基盤が決めません。",
            technical: "保存・ログ・バックアップごとの保持と削除を確認します。期間と復旧条件は未確認の業務要件から補いません。",
        },
    ]
}

pub fn current(workspace: &Workspace) -> (Domain, u32) {
    workspace
        .profile_history
        .last()
        .map_or((Domain::General, 0), |s| (s.domain, s.version))
}

pub fn select(workspace: &mut Workspace, domain: Domain) -> io::Result<()> {
    if current(workspace).0 == domain && !workspace.profile_history.is_empty() {
        return Err(invalid("同じ確認モードが既に選択されています"));
    }
    if workspace.profile_history.len() >= 256 {
        return Err(invalid("確認モードの履歴は256件以内です"));
    }
    workspace.schema_version = workspace.schema_version.max(2);
    workspace.profile_history.push(Selection {
        version: current(workspace).1 + 1,
        domain,
        catalog_version: CATALOG_VERSION.into(),
        recorded_revision: workspace
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("事例の版の上限です"))?,
    });
    Ok(())
}

pub fn proposed_check(
    workspace: &Workspace,
    id: &str,
    selection_version: u32,
) -> io::Result<Check> {
    let (domain, version) = current(workspace);
    if selection_version != version {
        return Err(invalid(
            "確認モードが変わっています。最新版を読み直してください",
        ));
    }
    let template = templates()
        .into_iter()
        .find(|t| t.id == id && (t.domain == Domain::General || domain == Domain::Medical))
        .ok_or_else(|| invalid("現在のモードに該当する確認項目の候補がありません"))?;
    let check_id = format!("profile-{}", template.id);
    if workspace.checks.iter().any(|c| c.id == check_id) {
        return Err(invalid("この確認項目の候補は既に追加されています"));
    }
    Ok(Check {
        id: check_id,
        version: 1,
        text: template.text.into(),
        origin: format!(
            "基盤による未採用の確認候補。カタログ{}、{}モード選択版{}。担当者の回答・業務要件ではない",
            CATALOG_VERSION,
            domain.label(),
            version
        ),
        source: None,
        observation: None,
        rule: Rule::ManualOnly,
        dependencies: vec![],
        questions: vec![],
    })
}

pub fn validate(workspace: &Workspace) -> io::Result<()> {
    if workspace.schema_version == 1
        && (!workspace.profile_history.is_empty()
            || workspace
                .evidence
                .iter()
                .any(|e| e.review_profile.is_some()))
    {
        return Err(invalid("確認モードの履歴を持つ事例には形式2が必要です"));
    }
    if workspace.profile_history.len() > 256 {
        return Err(invalid("確認モードの履歴が多すぎます"));
    }
    let mut previous = 0;
    for (i, selection) in workspace.profile_history.iter().enumerate() {
        if selection.version as usize != i + 1
            || selection.catalog_version != CATALOG_VERSION
            || selection.recorded_revision <= 1
            || selection.recorded_revision <= previous
            || selection.recorded_revision > workspace.revision
        {
            return Err(invalid(
                "確認モードの版・カタログ・記録した事例版が一致しません",
            ));
        }
        previous = selection.recorded_revision;
    }
    for evidence in &workspace.evidence {
        if evidence
            .review_profile
            .as_ref()
            .is_some_and(|p| !workspace.profile_history.contains(p))
        {
            return Err(invalid("根拠が参照する確認モードの履歴がありません"));
        }
    }
    Ok(())
}
