//! 試験案の判定条件・故障・状態の読み出し。
//! 対象プログラムの報告（終了コード・出力）と、基盤が保存先を読み出した結果を分けて記録する。
use crate::error::invalid;
use crate::model::{Assessment, Manifest, Verdict};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::Path,
};

pub const TEST_VALUE: &str = "{{test_value}}";
/// 引数の中で、固定版（読み取り専用の複製）の最上位の場所に置き換える文字列。
pub const SOURCE_ROOT: &str = "{{source}}";
const MAX_STATE: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrialFault {
    Normal,
    StorageReadOnly,
    StorageIsDirectory,
    SkipStateRead,
    NetworkUnavailable,
    StorageDirectoryReadOnly,
    ServerNotStarted,
    DatabaseStopped,
    DatabaseUnreachable,
    VolumeReadOnly,
    HttpRejected,
}
impl TrialFault {
    pub fn label(self) -> &'static str {
        match self {
            Self::HttpRejected => "宣言したHTTP要求を試験用サーバーで拒否する",
            Self::Normal => "故障なし（通常時）",
            Self::StorageReadOnly => "保存先を書き込み不可にする",
            Self::StorageIsDirectory => "保存先に同名のディレクトリを置く",
            Self::SkipStateRead => "基盤による保存先の読み出しを止める（観測の欠落）",
            Self::NetworkUnavailable => "外部への通信を遮断する",
            Self::StorageDirectoryReadOnly => "保存先のディレクトリを書き込み不可にする",
            Self::ServerNotStarted => "待つプロセス（受信側など）を起動しない",
            Self::DatabaseStopped => "データベースのコンテナを止める",
            Self::DatabaseUnreachable => "対象とデータベースの間の通信を切る",
            Self::VolumeReadOnly => "保存先のボリュームを読み取り専用にする",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Readback {
    #[serde(default, skip_serializing_if = "crate::python_env::Launch::is_file")]
    pub launch: crate::python_env::Launch,
    pub entrypoint: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckMethod {
    ExitCode,
    Stdout,
    Stderr,
    StoredFile,
    ReadbackOutput,
    ServerOutput,
    ContainerLog,
    DatabaseQuery,
    BrowserText,
    BrowserResponse,
    BrowserCanvas,
    ServerHttp,
    DatabaseRows,
}
impl CheckMethod {
    /// 誰の観測かを示す。対象の報告と基盤の読み出しを同じ扱いにしない。
    pub fn source_label(self) -> &'static str {
        match self {
            Self::ExitCode | Self::Stdout | Self::Stderr => {
                "対象プログラムの報告（終了コード・出力）"
            }
            Self::StoredFile => "基盤が保存先を直接読み出した結果",
            Self::ReadbackOutput => "別のプロセスで対象の読み出し機能を動かした結果",
            Self::ServerOutput => "待つプロセス（受信側など）の出力",
            Self::ContainerLog => "コンテナのログ（対象の応答とは別）",
            Self::BrowserCanvas => "canvasの画面写真から読み出した画素",
            Self::ServerHttp => "基盤が試験用サーバーへ直接要求して読み出した応答",
            Self::BrowserText => "試験用ブラウザで読み出した表示の文字列",
            Self::BrowserResponse => "試験操作中にブラウザが受け取ったHTTP応答",
            Self::DatabaseRows => "基盤が試験値に一致するDBの行を直接読み出した結果",
            Self::DatabaseQuery => "基盤がデータベースを直接読み出した結果",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Expectation {
    Zero,
    Nonzero,
    Contains,
    NotContains,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialCheck {
    pub id: String,
    /// 確認する範囲。例: 受付、書き込み、後からの読み出し、失敗の通知。
    pub scope: String,
    pub method: CheckMethod,
    pub expect: Expectation,
    #[serde(default)]
    pub text: Option<String>,
    /// コンテナのログを見るときに、対象のコンテナ名。省略時は全てのログ。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrialSpec {
    pub schema_version: u32,
    pub fault: TrialFault,
    #[serde(default)]
    pub storage: Option<String>,
    #[serde(default)]
    pub readback: Option<Readback>,
    pub checks: Vec<TrialCheck>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Observed {
    Pass,
    Fail,
    NotObserved,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub id: String,
    pub observed: Observed,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Phase {
    pub command: Vec<String>,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub output_limit: bool,
    pub stdout_excerpt: String,
    pub stderr_excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateRead {
    pub performed: bool,
    pub path: String,
    /// file・missing・directory・unreadable・skipped のいずれか。
    pub status: String,
    pub bytes: Option<u64>,
    pub contains_test_value: Option<bool>,
    pub excerpt: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrialOutcome {
    pub test_value: String,
    pub fault: TrialFault,
    pub fault_applied: String,
    pub fault_effective: bool,
    pub readback: Option<Phase>,
    pub state: Option<StateRead>,
    pub results: Vec<CheckResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<crate::server::ServerPhase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub containers: Option<crate::container::ContainerPhase>,
}

pub fn validate(
    spec: &TrialSpec,
    has_server: bool,
    python: bool,
    has_containers: bool,
    relative: impl Fn(&str) -> io::Result<()>,
) -> io::Result<()> {
    if spec.schema_version != 1 || spec.checks.is_empty() || spec.checks.len() > 16 {
        return Err(invalid("判定条件は1〜16件で指定してください"));
    }
    if let Some(storage) = &spec.storage {
        relative(storage)?;
    }
    if let Some(readback) = &spec.readback {
        crate::python_env::validate_launch(
            readback.launch,
            &readback.entrypoint,
            python,
            &relative,
        )?;
        if readback.args.len() > 64
            || readback
                .args
                .iter()
                .any(|a| a.len() > 16384 || a.contains('\0'))
        {
            return Err(invalid("読み出しの引数を確認してください"));
        }
    }
    let mut ids = BTreeSet::new();
    for check in &spec.checks {
        if check.id.is_empty()
            || check.id.len() > 64
            || !check
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            || !ids.insert(check.id.as_str())
        {
            return Err(invalid(
                "判定条件の識別子は重複しない英数字・ハイフン・下線です",
            ));
        }
        if check.scope.trim().is_empty() || check.scope.len() > 200 {
            return Err(invalid("判定条件ごとに確認する範囲を記入してください"));
        }
        let text_ok = check
            .text
            .as_ref()
            .is_some_and(|t| !t.is_empty() && t.len() <= 1024);
        match (check.method, check.expect) {
            (CheckMethod::ExitCode, Expectation::Zero | Expectation::Nonzero)
                if check.text.is_none() => {}
            (CheckMethod::ExitCode, _) => {
                return Err(invalid(
                    "終了コードの条件は0か0以外を選び、文字列は指定しません",
                ));
            }
            (_, Expectation::Contains | Expectation::NotContains) if text_ok => {}
            _ => {
                return Err(invalid(
                    "出力・保存先・読み出しの条件には、含む・含まないと1024バイト以内の文字列が必要です",
                ));
            }
        }
        if check.method == CheckMethod::StoredFile && spec.storage.is_none() {
            return Err(invalid("保存先を読み出す条件には保存先のパスが必要です"));
        }
        if check.method == CheckMethod::ReadbackOutput && spec.readback.is_none() {
            return Err(invalid("読み出しの条件には読み出しのコマンドが必要です"));
        }
        if check.method == CheckMethod::ServerOutput && !has_server {
            return Err(invalid(
                "待つプロセスの出力の条件には、待つプロセスが必要です",
            ));
        }
        if matches!(
            check.method,
            CheckMethod::ContainerLog
                | CheckMethod::DatabaseQuery
                | CheckMethod::BrowserCanvas
                | CheckMethod::ServerHttp
                | CheckMethod::BrowserText
                | CheckMethod::BrowserResponse
                | CheckMethod::DatabaseRows
        ) && !has_containers
        {
            return Err(invalid(
                "コンテナのログとデータベースの直接読み出しには、コンテナが必要です",
            ));
        }
    }
    if spec.fault == TrialFault::ServerNotStarted && !has_server {
        return Err(invalid("この故障には待つプロセスが必要です"));
    }
    if matches!(
        spec.fault,
        TrialFault::DatabaseStopped | TrialFault::DatabaseUnreachable | TrialFault::VolumeReadOnly
    ) && !has_containers
    {
        return Err(invalid("この故障にはコンテナが必要です"));
    }
    if spec.fault == TrialFault::StorageDirectoryReadOnly
        && !spec.storage.as_ref().is_some_and(|s| s.contains('/'))
    {
        return Err(invalid(
            "この故障には、ディレクトリを含む保存先のパス（例: received/file）が必要です",
        ));
    }
    if spec
        .checks
        .iter()
        .all(|c| c.method == CheckMethod::ExitCode)
    {
        return Err(invalid(
            "終了コードだけでは動作を判定できません。出力・保存先・読み出しの条件を一つ以上加えてください",
        ));
    }
    if matches!(
        spec.fault,
        TrialFault::StorageReadOnly | TrialFault::StorageIsDirectory | TrialFault::SkipStateRead
    ) && spec.storage.is_none()
    {
        return Err(invalid("この故障には保存先のパスが必要です"));
    }
    Ok(())
}

pub fn substitute(value: &str, test_value: &str) -> String {
    value.replace(TEST_VALUE, test_value)
}
pub fn substitute_json(value: &serde_json::Value, test_value: &str) -> serde_json::Value {
    match value {
        serde_json::Value::String(s) => serde_json::Value::String(substitute(s, test_value)),
        serde_json::Value::Array(a) => a.iter().map(|v| substitute_json(v, test_value)).collect(),
        serde_json::Value::Object(o) => o
            .iter()
            .map(|(k, v)| (k.clone(), substitute_json(v, test_value)))
            .collect::<serde_json::Map<_, _>>()
            .into(),
        other => other.clone(),
    }
}

/// 対象を起動する前に、作業領域へ故障を適用する。適用できたかを返す。
pub fn apply_fault(spec: &TrialSpec, work: &Path) -> io::Result<(String, bool)> {
    let target = spec.storage.as_ref().map(|s| work.join(s));
    match (spec.fault, target) {
        (TrialFault::HttpRejected, _) => Ok((
            "レシピの試験用サーバー設定で要求を拒否する。実際のHTTP応答で効いたか確認する".into(),
            false,
        )),
        (TrialFault::Normal, _) => Ok(("故障は加えていない".into(), true)),
        (TrialFault::SkipStateRead, _) => Ok((
            "対象は通常どおり動かし、基盤による保存先の読み出しだけを行わない".into(),
            true,
        )),
        (TrialFault::NetworkUnavailable, _) => Ok((
            "bubblewrapで外部ネットワークを共有せずに起動する".into(),
            true,
        )),
        // 効いたかどうかは、操作の直前にポートが応答しないことで確かめる。
        (TrialFault::ServerNotStarted, _) => Ok((
            "待つプロセスを起動せずに、操作するプロセスだけを動かす".into(),
            true,
        )),
        (TrialFault::DatabaseStopped, _) => Ok((
            "コンテナが準備できたあと、データベースのコンテナを止めてから操作する".into(),
            true,
        )),
        (TrialFault::DatabaseUnreachable, _) => Ok((
            "コンテナが準備できたあと、データベースを試験用ネットワークから外してから操作する"
                .into(),
            true,
        )),
        (TrialFault::VolumeReadOnly, _) => Ok((
            "コンテナが準備できたあと、データベースの保存先ボリュームを読み取り専用で載せ直す"
                .into(),
            true,
        )),
        (TrialFault::StorageDirectoryReadOnly, Some(path)) => {
            let parent = path
                .parent()
                .filter(|p| *p != work)
                .ok_or_else(|| invalid("保存先のディレクトリが必要です"))?;
            fs::create_dir_all(parent)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(parent, fs::Permissions::from_mode(0o555))?;
            }
            let probe = parent.join(".eel-write-probe");
            let effective = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&probe)
                .is_err();
            if !effective {
                let _ = fs::remove_file(&probe);
            }
            Ok((
                format!(
                    "試行用の作業領域に、保存先のディレクトリ {} を空の読み取り専用ディレクトリとして先に作った{}",
                    parent.strip_prefix(work).unwrap_or(parent).display(),
                    if effective {
                        ""
                    } else {
                        "。ただしこの権限では書き込めてしまい、故障を再現できなかった"
                    }
                ),
                effective,
            ))
        }
        (TrialFault::StorageReadOnly, Some(path)) => {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::File::create(&path)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o444))?;
            }
            // 管理者権限などで書き込めてしまう場合は、故障を適用できなかったと記録する。
            let effective = fs::OpenOptions::new().write(true).open(&path).is_err();
            Ok((
                format!(
                    "試行用の作業領域に、保存先 {} を空の読み取り専用ファイルとして先に作った{}",
                    spec.storage.as_deref().unwrap_or(""),
                    if effective {
                        ""
                    } else {
                        "。ただしこの権限では書き込めてしまい、故障を再現できなかった"
                    }
                ),
                effective,
            ))
        }
        (TrialFault::StorageIsDirectory, Some(path)) => {
            fs::create_dir_all(&path)?;
            Ok((
                format!(
                    "試行用の作業領域に、保存先 {} と同じ名前のディレクトリを先に作った",
                    spec.storage.as_deref().unwrap_or("")
                ),
                true,
            ))
        }
        _ => Err(invalid("この故障には保存先のパスが必要です")),
    }
}

/// 対象の報告とは別に、基盤が保存先を読み出す。判定用の全文と、記録用の抜粋を返す。
pub fn read_state(
    spec: &TrialSpec,
    work: &Path,
    test_value: &str,
) -> (Option<StateRead>, Option<String>) {
    let Some(storage) = spec.storage.as_ref() else {
        return (None, None);
    };
    let mut full = None;
    let mut state = StateRead {
        performed: false,
        path: storage.clone(),
        status: "skipped".into(),
        bytes: None,
        contains_test_value: None,
        excerpt: String::new(),
        detail: String::new(),
    };
    if spec.fault == TrialFault::SkipStateRead {
        state.detail = "観測の欠落を起こすため、基盤は保存先を読み出していない".into();
        return (Some(state), None);
    }
    state.performed = true;
    let path = work.join(storage);
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            state.status = "missing".into();
            state.detail = "試行後の作業領域に保存先のファイルがない".into();
        }
        Err(e) => {
            state.status = "unreadable".into();
            state.detail = format!("保存先の情報を読めない: {e}");
        }
        Ok(m) if m.is_dir() => {
            state.status = "directory".into();
            state.detail = "保存先はディレクトリで、ファイルとして読めない".into();
        }
        Ok(m) if !m.is_file() => {
            state.status = "unreadable".into();
            state.detail = "保存先が通常のファイルではない".into();
        }
        Ok(m) if m.len() > MAX_STATE => {
            state.status = "unreadable".into();
            state.bytes = Some(m.len());
            state.detail = "保存先が1MiBを超えるため読み出していない".into();
        }
        Ok(m) => match fs::read(&path) {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                state.status = "file".into();
                state.bytes = Some(m.len());
                state.contains_test_value = Some(text.contains(test_value));
                state.excerpt = text.chars().take(2000).collect();
                state.detail = format!(
                    "基盤が試行後の保存先 {storage} を読み出した（{}バイト）",
                    m.len()
                );
                full = Some(text.into_owned());
            }
            Err(e) => {
                state.status = "unreadable".into();
                state.detail = format!("保存先を読めない: {e}");
            }
        },
    }
    (Some(state), full)
}

pub struct Outputs<'a> {
    pub exit_code: Option<i32>,
    pub completed: bool,
    pub stdout: &'a str,
    pub stderr: &'a str,
    pub readback: Option<(&'a Phase, &'a str)>,
    pub state: Option<&'a StateRead>,
    pub state_text: Option<&'a str>,
    /// 待つプロセスの記録と、標準出力・標準エラーを続けた全文。
    pub server: Option<(&'a crate::server::ServerPhase, &'a str)>,
    pub containers: Option<&'a crate::container::ContainerPhase>,
}

fn contains(expect: Expectation, haystack: &str, needle: &str) -> Observed {
    match (expect, haystack.contains(needle)) {
        (Expectation::Contains, true) | (Expectation::NotContains, false) => Observed::Pass,
        _ => Observed::Fail,
    }
}

pub fn evaluate(spec: &TrialSpec, test_value: &str, out: &Outputs) -> Vec<CheckResult> {
    spec.checks
        .iter()
        .map(|check| {
            let text = check.text.as_deref().map(|t| substitute(t, test_value)).unwrap_or_default();
            let (observed, detail) = match check.method {
                CheckMethod::ExitCode => match (out.completed, out.exit_code) {
                    (true, Some(code)) => {
                        let pass = (code == 0) == (check.expect == Expectation::Zero);
                        (if pass { Observed::Pass } else { Observed::Fail }, format!("終了コードは{code}"))
                    }
                    _ => (Observed::NotObserved, "終了を確認できず、終了コードを取得していない".into()),
                },
                CheckMethod::Stdout | CheckMethod::Stderr => {
                    if !out.completed {
                        (Observed::NotObserved, "対象の実行が完了していない".into())
                    } else {
                        let (name, value) = if check.method == CheckMethod::Stdout {
                            ("標準出力", out.stdout)
                        } else {
                            ("標準エラー", out.stderr)
                        };
                        let observed = contains(check.expect, value, &text);
                        (observed, format!("{name}に「{text}」が{}", if value.contains(&text) { "含まれる" } else { "含まれない" }))
                    }
                }
                CheckMethod::StoredFile => match out.state {
                    Some(state) if !state.performed => (Observed::NotObserved, state.detail.clone()),
                    Some(state) => match state.status.as_str() {
                        "file" => {
                            let has = out.state_text.is_some_and(|full| full.contains(&text));
                            let observed = contains(check.expect, if has { &text } else { "" }, &text);
                            (observed, format!("{}。「{text}」が{}", state.detail, if has { "含まれる" } else { "含まれない" }))
                        }
                        "missing" | "directory" => {
                            let observed = if check.expect == Expectation::NotContains { Observed::Pass } else { Observed::Fail };
                            (observed, state.detail.clone())
                        }
                        _ => (Observed::NotObserved, state.detail.clone()),
                    },
                    None => (Observed::NotObserved, "保存先が指定されていない".into()),
                },
                CheckMethod::ReadbackOutput => match out.readback {
                    Some((phase, stdout)) if !phase.timed_out && !phase.output_limit => {
                        let has = stdout.contains(&text);
                        if phase.exit_code != Some(0) && !has {
                            (Observed::NotObserved, format!("読み出しのプロセスが終了コード{:?}で終わり、結果を採用できない", phase.exit_code))
                        } else if phase.exit_code != Some(0) && check.expect == Expectation::NotContains {
                            (Observed::NotObserved, "読み出しのプロセスが正常に終わっていない".into())
                        } else {
                            (contains(check.expect, stdout, &text), format!("別のプロセスで読み出した標準出力に「{text}」が{}", if has { "含まれる" } else { "含まれない" }))
                        }
                    }
                    Some(_) => (Observed::NotObserved, "読み出しのプロセスが待機上限・出力上限に達した".into()),
                    None => (Observed::NotObserved, "読み出しを実行していない".into()),
                },
                CheckMethod::ServerOutput => match out.server {
                    Some((phase, _)) if !phase.started => (Observed::NotObserved, "待つプロセスを起動していない".into()),
                    Some((phase, _)) if phase.output_limit => (Observed::NotObserved, "待つプロセスの出力が上限に達した".into()),
                    Some((_, output)) => {
                        let has = output.contains(&text);
                        (contains(check.expect, output, &text), format!("待つプロセスの出力に「{text}」が{}", if has { "含まれる" } else { "含まれない" }))
                    }
                    None => (Observed::NotObserved, "待つプロセスの記録がない".into()),
                },
                CheckMethod::ContainerLog => match out.containers {
                    Some(phase) => {
                        let logs = phase
                            .services
                            .iter()
                            .filter(|s| check.target.as_deref().is_none_or(|t| t == s.name))
                            .map(|s| s.log_excerpt.as_str())
                            .collect::<Vec<_>>()
                            .join("\n");
                        if check
                            .target
                            .as_ref()
                            .is_some_and(|t| !phase.services.iter().any(|s| &s.name == t))
                        {
                            (Observed::NotObserved, "指定したコンテナのログがない".into())
                        } else {
                            let has = logs.contains(&text);
                            (
                                contains(check.expect, &logs, &text),
                                format!(
                                    "コンテナのログに「{text}」が{}",
                                    if has { "含まれる" } else { "含まれない" }
                                ),
                            )
                        }
                    }
                    None => (Observed::NotObserved, "コンテナの記録がない".into()),
                },
                CheckMethod::BrowserCanvas | CheckMethod::ServerHttp | CheckMethod::BrowserText | CheckMethod::BrowserResponse | CheckMethod::DatabaseRows =>
                    crate::browser::evaluate(check, out.containers, &text),
                CheckMethod::DatabaseQuery => {
                    match out.containers.and_then(|c| c.database_query.as_deref()) {
                        Some(query) => {
                            let has = query.contains(&text);
                            (
                                contains(check.expect, query, &text),
                                format!(
                                    "基盤がデータベースを直接読み出した結果に「{text}」が{}",
                                    if has { "含まれる" } else { "含まれない" }
                                ),
                            )
                        }
                        None => (
                            Observed::NotObserved,
                            "基盤はデータベースを直接読み出していない".into(),
                        ),
                    }
                },
            };
            CheckResult { id: check.id.clone(), observed, detail }
        })
        .collect()
}

/// 判定条件の結果をまとめる。同じ範囲で結果が分かれた場合は判定しない。
pub fn assess(
    manifest: &Manifest,
    spec: &TrialSpec,
    outcome: &TrialOutcome,
    snapshot_unchanged: bool,
) -> Assessment {
    let mut result = Assessment {
        condition_id: manifest.condition.id.clone(),
        condition_version: manifest.condition.version,
        verdict: Verdict::Unknown,
        evidence: outcome
            .results
            .iter()
            .map(|r| format!("execution.json#trial.results.{}", r.id))
            .collect(),
        reason: String::new(),
        unconfirmed: vec![
            "選んだ判定条件の外側（他の利用者・他の環境・長期の保存・実運用のデータ）は確認していない".into(),
            "対象プログラムの報告と、基盤による読み出しは別の観測として扱った".into(),
        ],
        next_action: String::new(),
    };
    if !snapshot_unchanged {
        result.reason =
            "試行用の固定版が開始時と終了時で異なり、どのコードの結果か確認できない".into();
        return result;
    }
    if !outcome.fault_effective {
        result.reason = format!("故障を適用できなかった: {}", outcome.fault_applied);
        return result;
    }
    if let Some(server) = &outcome.server
        && spec.fault != TrialFault::ServerNotStarted
    {
        if !server.ready {
            result.reason = format!(
                "待つプロセスが準備できず、操作するプロセスを動かしていない: {}",
                server.detail
            );
            result.next_action =
                "待つプロセスの標準エラー、起動のしかた、準備の待機上限を確認して再試験する".into();
            return result;
        }
        if server.exited_before_stop {
            result.reason = format!(
                "待つプロセスが、基盤が止める前に終了していた（終了コード{:?}）。どの時点まで動いていたか確認できない",
                server.exit_code
            );
            result.next_action = "待つプロセスの標準エラーを確認して再試験する".into();
            return result;
        }
    }
    if let Some(containers) = &outcome.containers
        && !matches!(
            spec.fault,
            TrialFault::DatabaseStopped
                | TrialFault::DatabaseUnreachable
                | TrialFault::VolumeReadOnly
        )
        && containers.services.iter().any(|s| !s.ready)
    {
        result.reason = format!(
            "試験用のコンテナが準備できず、操作していない: {}",
            containers
                .services
                .iter()
                .find(|s| !s.ready)
                .map(|s| s.detail.as_str())
                .unwrap_or(&containers.detail)
        );
        result.next_action =
            "コンテナのログ、イメージのダイジェスト、準備の待機上限を確認して再試験する".into();
        return result;
    }
    let scope_of: BTreeMap<_, _> = spec
        .checks
        .iter()
        .map(|c| (c.id.as_str(), c.scope.as_str()))
        .collect();
    let mut by_scope: BTreeMap<&str, BTreeSet<Observed>> = BTreeMap::new();
    for r in &outcome.results {
        if r.observed != Observed::NotObserved {
            by_scope
                .entry(scope_of[r.id.as_str()])
                .or_default()
                .insert(r.observed);
        }
    }
    let conflicts: Vec<_> = by_scope
        .iter()
        .filter(|(_, set)| set.len() > 1)
        .map(|(scope, _)| *scope)
        .collect();
    let failed: Vec<_> = outcome
        .results
        .iter()
        .filter(|r| r.observed == Observed::Fail)
        .collect();
    let missing: Vec<_> = outcome
        .results
        .iter()
        .filter(|r| r.observed == Observed::NotObserved)
        .collect();
    for m in &missing {
        result.unconfirmed.push(format!("{}: {}", m.id, m.detail));
    }
    if !conflicts.is_empty() {
        result.reason = format!(
            "同じ範囲（{}）について、観測した結果が食い違う",
            conflicts.join("、")
        );
        result.next_action =
            "食い違った観測の出所（対象の報告か基盤の読み出しか）と、対応するコードを確認する"
                .into();
    } else if !failed.is_empty() {
        result.verdict = Verdict::NotSatisfied;
        result.reason = format!(
            "期待と異なる結果を観測した: {}",
            failed
                .iter()
                .map(|r| format!("{}（{}）", r.id, r.detail))
                .collect::<Vec<_>>()
                .join("、")
        );
        result.next_action =
            "期待と異なった条件に対応するコードの分岐とエラー処理を確認し、改修するか期待を見直す"
                .into();
    } else if !missing.is_empty() {
        result.reason = format!(
            "必要な観測が得られていない: {}",
            missing
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>()
                .join("、")
        );
        result.next_action = "観測できなかった条件の取得方法（保存先のパス、読み出しのコマンド、待機上限）を確認して再試験する".into();
    } else {
        result.verdict = Verdict::Satisfied;
        let scopes: BTreeSet<_> = spec.checks.iter().map(|c| c.scope.as_str()).collect();
        result.reason = format!(
            "選んだ{}件の判定条件をすべて満たした。確認した範囲: {}",
            spec.checks.len(),
            scopes.into_iter().collect::<Vec<_>>().join("、")
        );
        result.next_action =
            "判定条件が期待の意味を十分に表しているかを、記入者と技術者で確認する".into();
    }
    result
}

/// 判定条件を、技術者以外も読める文にする。試験用の値はそのまま示さず言葉にする。
pub fn describe(check: &TrialCheck, spec: &TrialSpec) -> String {
    let text = check
        .text
        .as_deref()
        .map(|t| {
            if t == TEST_VALUE {
                "試験用の値".to_owned()
            } else {
                format!("「{}」", t.replace(TEST_VALUE, "（試験用の値）"))
            }
        })
        .unwrap_or_default();
    let has = if check.expect == Expectation::Contains {
        "が含まれる"
    } else {
        "が含まれない"
    };
    match check.method {
        CheckMethod::ExitCode if check.expect == Expectation::Zero => {
            "対象プログラムが終了コード0（正常終了）で終わる".into()
        }
        CheckMethod::ExitCode => "対象プログラムが0以外の終了コード（失敗）で終わる".into(),
        CheckMethod::Stdout => format!("対象の標準出力に{text}{has}"),
        CheckMethod::Stderr => format!("対象の標準エラー（エラー表示）に{text}{has}"),
        CheckMethod::StoredFile => format!(
            "試行後に基盤が保存先 {} を読み出すと、{text}{has}",
            spec.storage.as_deref().unwrap_or("（未指定）")
        ),
        CheckMethod::ReadbackOutput => {
            format!("書き込みのプロセスが終わった後、別のプロセスで読み出すと{text}{has}")
        }
        CheckMethod::ServerOutput => {
            format!("待つプロセス（受信側など）の出力に{text}{has}")
        }
        CheckMethod::BrowserCanvas => format!("canvasの画素の観測に{text}{has}"),
        CheckMethod::ServerHttp => format!("サーバーの直接読取の応答に{text}{has}"),
        CheckMethod::BrowserText => format!("画面の文字列に{text}{has}"),
        CheckMethod::BrowserResponse => format!("ブラウザが受け取ったHTTP応答に{text}{has}"),
        CheckMethod::DatabaseRows => format!("DBから直接読み出した行に{text}{has}"),
        CheckMethod::ContainerLog => format!("コンテナのログに{text}{has}"),
        CheckMethod::DatabaseQuery => {
            format!("基盤がデータベースを直接読み出した結果に{text}{has}")
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryItem {
    pub id: String,
    pub scope: String,
    pub source: String,
    pub expected: String,
    pub observed: Option<Observed>,
    pub observed_label: String,
    pub detail: String,
}

/// 技術者向けの詳細と非技術者向けの説明が参照する、一つの試行の要約。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    pub tried: String,
    pub fault: String,
    pub fault_applied: String,
    pub test_value: String,
    pub command: Vec<String>,
    pub readback_command: Option<Vec<String>>,
    pub items: Vec<SummaryItem>,
    pub state: Option<StateRead>,
    pub exit_code: Option<i32>,
    pub stdout_excerpt: String,
    pub stderr_excerpt: String,
    #[serde(default)]
    pub server: Option<crate::server::ServerPhase>,
    /// 依存先の「リポジトリ@コミット（読み込み先）」。
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub containers: Option<crate::container::ContainerPhase>,
}

pub fn observed_label(o: Observed) -> &'static str {
    match o {
        Observed::Pass => "期待どおり",
        Observed::Fail => "期待と異なる",
        Observed::NotObserved => "観測できていない",
    }
}

pub fn summarize(trace: &crate::execution::ExecutionTrace, command: &[String]) -> Option<Summary> {
    let spec = trace.recipe.trial.as_ref()?;
    let outcome = trace.trial.as_ref();
    let short = &trace.recipe.source.commit[..12.min(trace.recipe.source.commit.len())];
    let runtime = match trace.recipe.runtime {
        crate::execution::Runtime::Node => "Node.js",
        crate::execution::Runtime::Python3 => "Python 3",
        crate::execution::Runtime::Browser => "ブラウザ（コンテナ）",
        crate::execution::Runtime::Http => "HTTP（コンテナ）",
    };
    let items = spec
        .checks
        .iter()
        .map(|check| {
            let result = outcome.and_then(|o| o.results.iter().find(|r| r.id == check.id));
            SummaryItem {
                id: check.id.clone(),
                scope: check.scope.clone(),
                source: check.method.source_label().into(),
                expected: describe(check, spec),
                observed: result.map(|r| r.observed),
                observed_label: result
                    .map_or("記録がない", |r| observed_label(r.observed))
                    .into(),
                detail: result.map(|r| r.detail.clone()).unwrap_or_default(),
            }
        })
        .collect();
    let launched = match trace.recipe.launch {
        crate::python_env::Launch::File => trace.recipe.entrypoint.clone(),
        launch => format!("{} {}", launch.label(), trace.recipe.entrypoint),
    };
    let waiting = match (
        &trace.recipe.server,
        outcome.and_then(|o| o.server.as_ref()),
    ) {
        (Some(server), Some(phase)) if phase.started => format!(
            "。先に待つプロセス（{}）を起動し、ポート{}で待たせた",
            server.entrypoint, phase.port
        ),
        (Some(server), _) => format!("。待つプロセス（{}）は起動していない", server.entrypoint),
        (None, _) => String::new(),
    };
    let container_note = match outcome.and_then(|o| o.containers.as_ref()) {
        Some(phase) if !phase.services.is_empty() => {
            format!("。試験専用コンテナ {} 件を起動した", phase.services.len())
        }
        _ => String::new(),
    };
    let dependencies = trace
        .python
        .as_ref()
        .map(|p| {
            p.packages
                .iter()
                .filter(|r| r.role == "dependency")
                .map(|r| {
                    format!(
                        "{}@{}（{}）",
                        r.repository,
                        &r.commit[..12.min(r.commit.len())],
                        r.root.as_deref().unwrap_or("最上位")
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    Some(Summary {
        tried: format!(
            "{} のコミット {short} にある {launched} を、{runtime}（{}方式）で起動した{waiting}{container_note}",
            trace.recipe.source.repository, trace.mode
        ),
        fault: spec.fault.label().into(),
        fault_applied: outcome.map(|o| o.fault_applied.clone()).unwrap_or_default(),
        test_value: outcome.map(|o| o.test_value.clone()).unwrap_or_default(),
        command: command.to_vec(),
        readback_command: outcome.and_then(|o| o.readback.as_ref().map(|p| p.command.clone())),
        items,
        state: outcome.and_then(|o| o.state.clone()),
        exit_code: trace.exit_code,
        stdout_excerpt: trace.stdout_excerpt.clone(),
        stderr_excerpt: trace.stderr_excerpt.clone(),
        server: outcome.and_then(|o| o.server.clone()),
        dependencies,
        containers: outcome.and_then(|o| o.containers.clone()),
    })
}
