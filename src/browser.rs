//! 宣言した手順だけを試験用Chromiumで操作する。ログイン準備と試験操作を分ける。
use crate::error::invalid;
use crate::{
    container::{ContainerSpec, Session as Containers},
    recording::write_json_new,
};
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "by", rename_all = "snake_case", deny_unknown_fields)]
pub enum Locator {
    Label { text: String },
    Role { role: String, name: String },
    Css { selector: String },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    #[serde(default)]
    pub frames: Vec<String>,
    pub locator: Locator,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum Input {
    Literal { value: String },
    Secret { key: String },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Goto {
        path: String,
    },
    Fill {
        target: Target,
        value: Input,
    },
    Click {
        target: Target,
    },
    Expand {
        target: Target,
        content: Target,
    },
    /// 準備用の案内を閉じ、非表示を確認する。通常のクリック操作は再試行しない。
    Dismiss {
        target: Target,
        content: Target,
    },
    Select {
        target: Target,
        value: String,
    },
    ReadText {
        target: Target,
        #[serde(default)]
        contains: Option<String>,
    },
    ReadCanvas {
        target: Target,
        min_levels: u16,
        min_pixels: u32,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RejectionWatch {
    pub watch: String,
    pub status: u16,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub id: String,
    #[serde(flatten)]
    pub action: Action,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResponseWatch {
    pub id: String,
    pub path: String,
    pub method: String,
    #[serde(default)]
    pub contains: Option<String>,
    #[serde(default)]
    pub payload: Option<crate::http_probe::Payload>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseProbe {
    pub id: String,
    pub service: String,
    pub database: String,
    pub table: String,
    pub match_column: String,
    pub columns: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserSpec {
    pub service: String,
    pub tls: bool,
    pub start_path: String,
    pub expected_version: String,
    #[serde(default)]
    pub preparation: Vec<Step>,
    pub actions: Vec<Step>,
    #[serde(default)]
    pub responses: Vec<ResponseWatch>,
    #[serde(default)]
    pub database: Vec<DatabaseProbe>,
    #[serde(default)]
    pub http_reads: Vec<crate::http_probe::Probe>,
    #[serde(default)]
    pub rejection: Option<RejectionWatch>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanvasRead {
    #[serde(default)]
    pub measurement: String,
    pub width: u32,
    pub height: u32,
    pub levels: u16,
    pub non_black_pixels: u32,
    pub rgba_sha256: String,
    pub variation: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepRecord {
    pub id: String,
    pub phase: String,
    pub completed: bool,
    pub text: Option<String>,
    pub screenshot: Option<String>,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canvas: Option<CanvasRead>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseRecord {
    pub watch: String,
    pub method: String,
    pub path: String,
    pub status: u16,
    pub body_match: Option<bool>,
    pub request_step: Option<String>,
    #[serde(default)]
    pub payload_sha256: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserReport {
    pub schema_version: u32,
    pub run_id: String,
    pub version: String,
    #[serde(default)]
    pub launch_args: Vec<String>,
    pub prepared: bool,
    pub completed: bool,
    pub steps: Vec<StepRecord>,
    pub responses: Vec<ResponseRecord>,
    pub blocked_requests: u32,
    #[serde(default)]
    pub response_limit: bool,
    pub detail: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseRead {
    pub id: String,
    pub performed: bool,
    pub exit_code: Option<i32>,
    pub text: String,
    pub detail: String,
}

fn id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
fn bounded(s: &str) -> bool {
    !s.is_empty() && s.len() <= 2048 && !s.contains('\0')
}
fn path(s: &str) -> bool {
    s.starts_with('/')
        && !s.starts_with("//")
        && !s.contains(['\\', '\r', '\n', '\0'])
        && s.len() <= 2048
}
fn target(t: &Target) -> bool {
    t.frames.len() <= 8
        && t.frames.iter().all(|f| bounded(f))
        && match &t.locator {
            Locator::Label { text } => bounded(text),
            Locator::Role { role, name } => {
                [
                    "button", "textbox", "link", "combobox", "checkbox", "menuitem", "tab",
                    "heading",
                ]
                .contains(&role.as_str())
                    && bounded(name)
            }
            Locator::Css { selector } => bounded(selector),
        }
}
pub fn sql_identifier(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
pub fn validate(spec: &BrowserSpec, containers: &ContainerSpec) -> io::Result<()> {
    if !containers.services.iter().any(|s| s.name == spec.service)
        || !path(&spec.start_path)
        || spec.expected_version.is_empty()
        || spec.expected_version.len() > 64
        || !spec
            .expected_version
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b'.')
        || spec.actions.is_empty()
        || spec.actions.len() + spec.preparation.len() > 64
        || spec.responses.len() > 16
        || spec.database.len() > 16
        || spec.http_reads.len() > 16
    {
        return Err(invalid(
            "ブラウザの操作先、固定した版、手順の上限を確認してください",
        ));
    }
    let mut ids = std::collections::BTreeSet::new();
    for s in spec.preparation.iter().chain(&spec.actions) {
        if !id(&s.id) || !ids.insert(&s.id) {
            return Err(invalid("ブラウザの手順には重複しない識別子が必要です"));
        }
        let valid = match &s.action {
            Action::Goto { path: p } => path(p),
            Action::Fill { target: t, value } => {
                target(t)
                    && match value {
                        Input::Literal { value } => value.len() <= 16384 && !value.contains('\0'),
                        Input::Secret { key } => id(key),
                    }
            }
            Action::Click { target: t } => target(t),
            Action::Expand { target: t, content } => target(t) && target(content),
            Action::Dismiss { target: t, content } => {
                target(t)
                    && target(content)
                    && spec.preparation.iter().any(|prepared| prepared.id == s.id)
            }
            Action::ReadText {
                target: t,
                contains,
            } => target(t) && contains.as_ref().is_none_or(|s| bounded(s)),
            Action::Select { target: t, value } => target(t) && bounded(value),
            Action::ReadCanvas {
                target: t,
                min_levels,
                min_pixels,
            } => {
                target(t) && (2..=256).contains(min_levels) && (1..=2_000_000).contains(min_pixels)
            }
        };
        if !valid {
            return Err(invalid("ブラウザの選択要素と入力値を確認してください"));
        }
    }
    ids.clear();
    for w in &spec.responses {
        if !id(&w.id)
            || !ids.insert(&w.id)
            || !path(&w.path)
            || w.path.contains(['?', '#'])
            || !["GET", "POST", "PUT", "PATCH", "DELETE"].contains(&w.method.as_str())
            || w.contains.as_ref().is_some_and(|s| !bounded(s))
        {
            return Err(invalid("HTTP応答の観測先を確認してください"));
        }
    }
    ids.clear();
    for probe in &spec.http_reads {
        if !id(&probe.id)
            || !ids.insert(&probe.id)
            || !path(&probe.path)
            || probe.path.contains('#')
            || !containers.services.iter().any(|s| s.name == probe.service)
        {
            return Err(invalid("サーバーの直接読取先を確認してください"));
        }
    }
    if let Some(rejection) = &spec.rejection
        && (!(400..=599).contains(&rejection.status)
            || !spec.responses.iter().any(|w| w.id == rejection.watch))
    {
        return Err(invalid("HTTP拒否を確認する観測先と状態コードが必要です"));
    }
    ids.clear();
    for q in &spec.database {
        if !id(&q.id)
            || !ids.insert(&q.id)
            || !containers
                .services
                .iter()
                .any(|s| s.name == q.service && s.role == crate::container::ServiceRole::Database)
            || ![&q.database, &q.table, &q.match_column]
                .iter()
                .all(|s| sql_identifier(s))
            || q.columns.is_empty()
            || q.columns.len() > 16
            || !q.columns.iter().all(|c| sql_identifier(c))
        {
            return Err(invalid(
                "DBの読取先は試験用DBと英数字・下線の識別子で指定してください",
            ));
        }
    }
    Ok(())
}
pub fn rejection_observed(spec: &BrowserSpec, report: &BrowserReport) -> bool {
    let Some(rejection) = &spec.rejection else {
        return false;
    };
    let responses = report
        .responses
        .iter()
        .filter(|r| r.watch == rejection.watch)
        .collect::<Vec<_>>();
    report.prepared
        && !report.response_limit
        && !responses.is_empty()
        && responses.iter().all(|r| {
            r.status == rejection.status
                && r.request_step.as_ref().is_some_and(|id| {
                    spec.actions.iter().any(|s| &s.id == id)
                        && report
                            .steps
                            .iter()
                            .any(|s| &s.id == id && s.phase == "action")
                })
        })
}
/// 列名と検索値を別々に検査し、1件の試験値に一致する行だけを読む。任意のSQLは受け取らない。
pub fn query(q: &DatabaseProbe, test_value: &str) -> String {
    let columns = q
        .columns
        .iter()
        .map(|c| format!("`{c}`"))
        .collect::<Vec<_>>()
        .join(",");
    let hex = test_value
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    format!(
        "SELECT {columns} FROM `{}` WHERE `{}` = CONVERT(0x{hex} USING utf8mb4) LIMIT 100",
        q.table, q.match_column
    )
}
fn unavailable(run_id: &str, detail: &str) -> BrowserReport {
    BrowserReport {
        schema_version: 1,
        run_id: run_id.into(),
        version: String::new(),
        launch_args: vec![],
        prepared: false,
        completed: false,
        steps: vec![],
        responses: vec![],
        blocked_requests: 0,
        response_limit: false,
        detail: detail.into(),
    }
}
pub struct Session {
    child: crate::process::ChildGuard,
    directory: PathBuf,
    deadline: Instant,
    report: BrowserReport,
}
impl Session {
    pub fn prepare(
        spec: &BrowserSpec,
        containers: &Containers,
        directory: &Path,
        run_id: &str,
        test_value: &str,
        timeout_ms: u64,
    ) -> io::Result<Self> {
        let module = match std::env::var_os("EEL_PLAYWRIGHT_MODULE") {
            Some(p) => PathBuf::from(p),
            None => std::env::current_dir()?.join("node_modules/playwright/index.mjs"),
        };
        let module=fs::canonicalize(module).map_err(|_|invalid("ブラウザ試験にはサーバー側のPlaywrightが必要です。npm ciを実行するかEEL_PLAYWRIGHT_MODULEを指定してください"))?;
        let node = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|d| d.join("node"))
            .find(|p| p.is_file())
            .ok_or_else(|| invalid("ブラウザ試験にはNode.jsが必要です"))?;
        let directory = directory.join("browser");
        fs::create_dir(&directory)?;
        let directory = fs::canonicalize(directory)?;
        let origin = format!(
            "{}://127.0.0.1:{}",
            if spec.tls { "https" } else { "http" },
            containers.port_of(&spec.service)?
        );
        let origins = containers
            .ports()
            .values()
            .flat_map(|p| {
                [
                    format!("http://127.0.0.1:{p}"),
                    format!("https://127.0.0.1:{p}"),
                ]
            })
            .collect::<Vec<_>>();
        let config = serde_json::json!({"schema_version":1,"spec":spec,"run_id":run_id,"test_value":test_value,"origin":origin,"origins":origins,"timeout_ms":timeout_ms});
        write_json_new(&directory.join("request.json"), &config)?;
        fs::write(
            directory.join("runner.mjs"),
            include_str!("../tools/browser-runner.mjs"),
        )?;
        let mut cmd = Command::new(node);
        cmd.env_clear()
            .env("HOME", &directory)
            .env("EEL_PLAYWRIGHT_MODULE", module);
        if spec
            .actions
            .iter()
            .chain(&spec.preparation)
            .any(|s| matches!(s.action, Action::ReadCanvas { .. }))
        {
            let png = std::env::var_os("EEL_PNGJS_MODULE")
                .map(PathBuf::from)
                .unwrap_or(std::env::current_dir()?.join("node_modules/pngjs/lib/png.js"));
            cmd.env(
                "EEL_PNGJS_MODULE",
                fs::canonicalize(png).map_err(|_| {
                    invalid("canvasの画素の観測にはnpm ciでpngjsを用意してください")
                })?,
            );
        }
        // HOMEは試行用に分ける。Playwrightの導入済みブラウザはホストの固定したキャッシュから探す。
        if std::env::var_os("PLAYWRIGHT_BROWSERS_PATH").is_none() {
            let cache = if cfg!(target_os = "macos") {
                std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Library/Caches"))
            } else {
                std::env::var_os("XDG_CACHE_HOME")
                    .map(PathBuf::from)
                    .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".cache")))
            };
            if let Some(cache) = cache {
                cmd.env("PLAYWRIGHT_BROWSERS_PATH", cache.join("ms-playwright"));
            }
        }
        for key in [
            "PATH",
            "PLAYWRIGHT_BROWSERS_PATH",
            "CHROME_DEVEL_SANDBOX",
            "EEL_BROWSER_EXECUTABLE",
            "EEL_BROWSER_EXTRA_ARGS",
        ] {
            if let Some(v) = std::env::var_os(key) {
                cmd.env(key, v);
            }
        }
        // 子プロセスの作業場所は試行ごとに変わる。相対パスはサーバーの起動場所で解決する。
        if let Some(path) = std::env::var_os("EEL_BROWSER_SECRETS_FILE") {
            cmd.env(
                "EEL_BROWSER_SECRETS_FILE",
                fs::canonicalize(path).map_err(|_| {
                    invalid("EEL_BROWSER_SECRETS_FILEで指定した試験用の秘密値ファイルを読めません")
                })?,
            );
        }
        cmd.arg(directory.join("runner.mjs"))
            .arg(directory.join("request.json"))
            .current_dir(&directory)
            .stdin(Stdio::null())
            .stdout(fs::File::create(directory.join("stdout.txt"))?)
            .stderr(fs::File::create(directory.join("stderr.txt"))?);
        let child = crate::process::ChildGuard::spawn(&mut cmd)?;
        let mut session = Self {
            child,
            directory,
            deadline: Instant::now() + Duration::from_millis(timeout_ms.saturating_add(2000)),
            report: unavailable(run_id, "ブラウザの準備を確認していない"),
        };
        if let Some(report) = session.wait("ready.json")? {
            session.report = report;
        }
        Ok(session)
    }
    fn wait(&mut self, file: &str) -> io::Result<Option<BrowserReport>> {
        loop {
            let path = self.directory.join(file);
            if path.is_file() {
                let len = fs::metadata(&path)?.len();
                if len > 1024 * 1024 {
                    return Err(invalid("ブラウザの記録が1MiBを超えました"));
                }
                let report: BrowserReport = crate::recording::read_json(&path)?;
                if report.schema_version != 1 || report.run_id != self.report.run_id {
                    return Err(invalid("ブラウザ記録の試行が一致しません"));
                }
                return Ok(Some(report));
            }
            if self.child.try_wait()?.is_some() {
                self.report.detail="ブラウザの実行環境が記録を返さず終了した。ブラウザの導入と版を確認してください".into();
                return Ok(None);
            }
            if Instant::now() >= self.deadline {
                self.report.detail = "ブラウザ試験が待機上限に達した".into();
                return Ok(None);
            }
            if ["stdout.txt", "stderr.txt"]
                .iter()
                .any(|p| fs::metadata(self.directory.join(p)).is_ok_and(|m| m.len() > 64 * 1024))
            {
                return Err(invalid("ブラウザ試験の出力上限に達しました"));
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
    pub fn prepared(&self) -> bool {
        self.report.prepared
    }
    pub fn run(&mut self) -> io::Result<BrowserReport> {
        if self.report.prepared {
            write_json_new(
                &self.directory.join("continue.json"),
                &serde_json::json!({"run_id":self.report.run_id,"proceed":true}),
            )?;
            if let Some(report) = self.wait("result.json")? {
                self.report = report;
            }
        }
        Ok(self.report.clone())
    }
    pub fn report(&self) -> BrowserReport {
        self.report.clone()
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = fs::write(self.directory.join("stop"), b"stop");
        for attempt in 0..80 {
            // PlaywrightはChromiumを別グループで起動する。起動完了前も
            // SIGTERMのハンドラーで回収させ、2回目は終了を強制する。
            if matches!(attempt, 0 | 20) {
                let _ = crate::process::terminate(&mut self.child);
            }
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            thread::sleep(Duration::from_millis(25));
        }
        let _ = crate::process::kill_group(&mut self.child);
        let _ = self.child.wait();
    }
}

/// 一つの観測元だけで条件を確かめる。欠落や相反する応答を成功で補わない。
pub fn evaluate(
    check: &crate::trial::TrialCheck,
    phase: Option<&crate::container::ContainerPhase>,
    text: &str,
) -> (crate::trial::Observed, String) {
    use crate::trial::{CheckMethod, Expectation, Observed};
    let missing = || {
        (
            Observed::NotObserved,
            "指定した観測を取得できていない".to_owned(),
        )
    };
    let Some(phase) = phase else {
        return missing();
    };
    let Some(target) = check.target.as_deref() else {
        return missing();
    };
    let result = |has: bool| {
        (
            if has == (check.expect == Expectation::Contains) {
                Observed::Pass
            } else {
                Observed::Fail
            },
            format!(
                "観測 {target} に条件の文字列が{}",
                if has {
                    "含まれる"
                } else {
                    "含まれない"
                }
            ),
        )
    };
    match check.method {
        CheckMethod::BrowserCanvas => phase
            .browser
            .as_ref()
            .and_then(|b| {
                b.steps
                    .iter()
                    .find(|s| s.id == target && s.phase == "action" && s.completed)
            })
            .and_then(|s| s.canvas.as_ref())
            .map(|canvas| result(format!("variation={}", canvas.variation).contains(text)))
            .unwrap_or_else(missing),
        CheckMethod::ServerHttp => phase
            .http_reads
            .iter()
            .find(|r| r.id == target && r.performed)
            .map(|r| {
                if text.starts_with("sha256=") && r.sha256.is_none() {
                    missing()
                } else {
                    result(
                        format!(
                            "status={} sha256={}",
                            r.status.map(|s| s.to_string()).unwrap_or_default(),
                            r.sha256.as_deref().unwrap_or("")
                        )
                        .contains(text),
                    )
                }
            })
            .unwrap_or_else(missing),
        CheckMethod::BrowserText => {
            let Some(step) = phase.browser.as_ref().and_then(|b| {
                b.steps
                    .iter()
                    .find(|s| s.id == target && s.phase == "action" && s.completed)
            }) else {
                return missing();
            };
            step.text
                .as_ref()
                .map(|t| result(t.contains(text)))
                .unwrap_or_else(missing)
        }
        CheckMethod::BrowserResponse => {
            let Some(browser) = &phase.browser else {
                return missing();
            };
            if browser.response_limit {
                return (
                    Observed::NotObserved,
                    "HTTP応答の記録上限に達し、全ての応答を照合できない".into(),
                );
            }
            let responses = browser
                .responses
                .iter()
                .filter(|r| {
                    r.watch == target
                        && r.request_step.as_ref().is_some_and(|id| {
                            browser
                                .steps
                                .iter()
                                .any(|s| &s.id == id && s.phase == "action")
                        })
                })
                .collect::<Vec<_>>();
            if responses.is_empty() {
                return missing();
            }
            let values = responses
                .iter()
                .map(|r| {
                    format!(
                        "status={} body_match={} sha256={}",
                        r.status,
                        r.body_match
                            .map(|b| b.to_string())
                            .unwrap_or("unobserved".into()),
                        r.payload_sha256.as_deref().unwrap_or("unobserved")
                    )
                    .contains(text)
                })
                .collect::<Vec<_>>();
            if values.iter().any(|v| *v != values[0])
                || (text.starts_with("body_match=")
                    && responses.iter().any(|r| r.body_match.is_none()))
                || (text.starts_with("sha256=")
                    && responses.iter().any(|r| r.payload_sha256.is_none()))
            {
                (
                    Observed::NotObserved,
                    "HTTP応答の条件が相反するか、本文を観測できていない".into(),
                )
            } else {
                result(values[0])
            }
        }
        CheckMethod::DatabaseRows => phase
            .database_reads
            .iter()
            .find(|q| q.id == target && q.performed && q.exit_code == Some(0))
            .map(|q| result(q.text.contains(text)))
            .unwrap_or_else(missing),
        _ => missing(),
    }
}
