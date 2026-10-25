//! 固定したGitのコードをコピーし、明示されたローカル試験を実行する。
//! 計測対象は構造化された自己申告の観測。OSの隔離や実行経路の証明ではない。
use crate::{
    model::*,
    python_env::{ImportRoot, Launch, Program},
    recording::*,
    run_record::{finish_run, make_manifest},
    workspace::CodeReference,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const MAX_FILES: usize = 512;
const MAX_FILE: u64 = 4 * 1024 * 1024;
const MAX_TOTAL: u64 = 32 * 1024 * 1024;
const MAX_TRACE: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    Local,
    Bubblewrap,
}
impl ExecutionMode {
    pub fn ensure_web(self) -> io::Result<()> {
        if self != Self::Bubblewrap {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Web経由のlocal実行は禁止しています。bubblewrapで起動してください",
            ));
        }
        Ok(())
    }
    pub fn ensure_allowed(self) -> io::Result<()> {
        if self == Self::Local && !cfg!(debug_assertions) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "本番ビルドではlocal方式を実行できません。隔離方式を指定してください",
            ));
        }
        Ok(())
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Bubblewrap => "bubblewrap",
        }
    }
}
impl std::str::FromStr for ExecutionMode {
    type Err = io::Error;
    fn from_str(value: &str) -> io::Result<Self> {
        match value {
            "local" => Ok(Self::Local),
            "bubblewrap" => Ok(Self::Bubblewrap),
            _ => Err(invalid("実行方式はlocalかbubblewrapです")),
        }
    }
}

#[cfg(test)]
mod mode_tests {
    use super::ExecutionMode;
    #[test]
    fn release_build_never_permits_local_execution() {
        assert_eq!(
            ExecutionMode::Local.ensure_allowed().is_ok(),
            cfg!(debug_assertions)
        );
        assert!(ExecutionMode::Bubblewrap.ensure_allowed().is_ok());
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Criterion {
    pub assertion: String,
    pub subject: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Runtime {
    Node,
    Python3,
    /// 基盤がコンテナの公開ポートへHTTPで操作する。対象のNode/Pythonは起動しない。
    Http,
    Browser,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub schema_version: u32,
    pub check_id: String,
    pub source: CodeReference,
    pub runtime: Runtime,
    pub entrypoint: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub input: serde_json::Value,
    pub criterion: Criterion,
    pub timeout_ms: u64,
    /// 判定条件・故障・保存先の読み出しを指定する試験案。旧形式の設定にはない。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trial: Option<crate::trial::TrialSpec>,
    /// 起動のしかた。旧形式の設定では固定版の中のファイル。
    #[serde(default, skip_serializing_if = "Launch::is_file")]
    pub launch: Launch,
    /// Pythonの読み込み先と依存先。指定した場合は `-S -P` で起動し、PYTHONPATHを基盤が決める。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub python: Option<crate::python_env::PythonSetup>,
    /// 操作するプロセスより先に起動して待つプロセス。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<crate::server::ServerSpec>,
    /// 試験専用のコンテナ。ダイジェスト付きのイメージだけを起動する。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub containers: Option<crate::container::ContainerSpec>,
    /// コンテナへ基盤が行うHTTP操作。指定すると操作するプロセスは起動しない。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http: Option<crate::container::HttpOp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser: Option<crate::browser::BrowserSpec>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationOutcome {
    Pass,
    Fail,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub schema_version: u32,
    pub sequence: u64,
    pub correlation: Correlation,
    pub assertion: String,
    pub subject: String,
    pub outcome: ObservationOutcome,
    pub detail: String,
    pub path: String,
    pub line: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotFile {
    pub path: String,
    pub blob: String,
    pub mode: String,
    pub bytes: u64,
    pub lines: u32,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservationRead {
    Valid,
    Missing,
    Invalid,
    #[default]
    Unspecified,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionTrace {
    pub schema_version: u32,
    pub correlation: Correlation,
    pub recipe: Recipe,
    pub tree: String,
    pub files: Vec<SnapshotFile>,
    pub runtime_path: String,
    pub runtime_version: String,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub output_limit: bool,
    pub snapshot_unchanged: bool,
    pub observations: Vec<Observation>,
    #[serde(default)]
    pub observation_read: ObservationRead,
    pub diagnostics: Vec<String>,
    pub mode: String,
    pub stdout_excerpt: String,
    pub stderr_excerpt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trial: Option<crate::trial::TrialOutcome>,
    /// 依存先の固定コミット、基盤が作ったパッケージ情報、起動のしかたの解決結果。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub python: Option<crate::python_env::PythonTrace>,
    /// 試験専用コンテナの起動・準備・ログ・HTTP操作・掃除の記録。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub containers: Option<crate::container::ContainerPhase>,
}
struct ChildGuard(std::process::Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn invalid(text: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, text.into())
}
fn relative(value: &str) -> io::Result<()> {
    if value.is_empty()
        || value.len() > 1024
        || value.contains('\\')
        || value.chars().any(char::is_control)
        || !Path::new(value)
            .components()
            .all(|p| matches!(p, Component::Normal(_)))
        || value
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == ".." || p == ".git")
    {
        return Err(invalid("対象内の相対パスが必要です"));
    }
    Ok(())
}
fn text(value: &str) -> io::Result<()> {
    if value.trim().is_empty() || value.len() > 16384 {
        return Err(invalid("空でない16384バイト以内の値が必要です"));
    }
    Ok(())
}
pub fn validate_criterion(criterion: &Criterion) -> io::Result<()> {
    text(&criterion.assertion)?;
    text(&criterion.subject)
}
pub fn validate_recipe(recipe: &Recipe) -> io::Result<()> {
    crate::workspace::validate_source(&recipe.source)?;
    let python = recipe.python.is_some();
    let http = recipe.runtime == Runtime::Http;
    let browser = recipe.runtime == Runtime::Browser;
    if browser {
        let containers = recipe
            .containers
            .as_ref()
            .ok_or_else(|| invalid("ブラウザ試験にはコンテナが必要です"))?;
        let spec = recipe
            .browser
            .as_ref()
            .ok_or_else(|| invalid("ブラウザ試験には操作の手順が必要です"))?;
        crate::browser::validate(spec, containers)?;
        if recipe.http.is_some()
            || recipe.python.is_some()
            || recipe.server.is_some()
            || !recipe.args.is_empty()
        {
            return Err(invalid(
                "ブラウザ試験には別の起動方法と引数を同時に指定しません",
            ));
        }
    } else if recipe.browser.is_some() {
        return Err(invalid("ブラウザ操作はbrowserの試行だけで指定できます"));
    }
    if http {
        if recipe.containers.is_none() || recipe.http.is_none() {
            return Err(invalid(
                "HTTPの操作には、ダイジェスト付きのコンテナと操作先が必要です",
            ));
        }
        if recipe.python.is_some() || recipe.server.is_some() {
            return Err(invalid(
                "HTTPの操作と、Pythonの読み込み先・待つプロセスは同時に指定しません",
            ));
        }
    } else if !browser {
        crate::python_env::validate_launch(recipe.launch, &recipe.entrypoint, python, relative)?;
    }
    relative(&recipe.source.path)?;
    if let Some(setup) = &recipe.python {
        if recipe.runtime != Runtime::Python3 {
            return Err(invalid(
                "Pythonの読み込み先は、Python 3の試行だけで指定できます",
            ));
        }
        crate::python_env::validate(setup, &recipe.source.repository, relative)?;
    }
    if let Some(server) = &recipe.server {
        if recipe.trial.is_none() {
            return Err(invalid("待つプロセスを使う試行には、判定条件が必要です"));
        }
        crate::server::validate(server)?;
        crate::python_env::validate_launch(server.launch, &server.entrypoint, python, relative)?;
    }
    if let Some(containers) = &recipe.containers {
        crate::container::validate_spec(containers)?;
        for service in &containers.services {
            if service.web_build.as_ref().is_some_and(|b| {
                b.source.repository != recipe.source.repository
                    || b.source.commit != recipe.source.commit
            }) {
                return Err(invalid(
                    "ビルドしたソースと試行の固定コミットが一致しません",
                ));
            }
        }
        if recipe.trial.is_none() {
            return Err(invalid("コンテナを使う試行には、判定条件が必要です"));
        }
    }
    if let Some(http) = &recipe.http {
        let containers = recipe
            .containers
            .as_ref()
            .ok_or_else(|| invalid("HTTPの操作には、ダイジェスト付きのコンテナが必要です"))?;
        crate::container::validate_http(http, containers)?;
        if let Some(file) = &http.body_file {
            relative(file)?;
        }
    }
    validate_criterion(&recipe.criterion)?;
    text(&recipe.check_id)?;
    if recipe.schema_version != 1
        || !(1..=300000).contains(&recipe.timeout_ms)
        || recipe.args.len() > 64
        || recipe
            .args
            .iter()
            .any(|a| a.len() > 16384 || a.contains('\0'))
        || serde_json::to_vec(recipe).map_err(io::Error::other)?.len() > 192 * 1024
        || serde_json::to_vec(&recipe.input)
            .map_err(io::Error::other)?
            .len()
            > 128 * 1024
    {
        return Err(invalid(
            "試験設定の形式・入力サイズ・待機上限を確認してください",
        ));
    }
    if let Some(trial) = &recipe.trial {
        if (trial.fault == crate::trial::TrialFault::HttpRejected)
            != recipe
                .browser
                .as_ref()
                .is_some_and(|b| b.rejection.is_some())
        {
            return Err(invalid(
                "HTTP拒否の故障にはブラウザーの観測先を指定してください",
            ));
        }
        crate::trial::validate(
            trial,
            recipe.server.is_some(),
            python,
            recipe.containers.is_some(),
            relative,
        )?;
        for check in &trial.checks {
            use crate::trial::CheckMethod;
            let valid = match check.method {
                CheckMethod::BrowserCanvas => recipe.browser.as_ref().is_some_and(|b| {
                    ["variation=true", "variation=false"]
                        .contains(&check.text.as_deref().unwrap_or(""))
                        && b.actions.iter().any(|s| {
                            Some(s.id.as_str()) == check.target.as_deref()
                                && matches!(s.action, crate::browser::Action::ReadCanvas { .. })
                        })
                }),
                CheckMethod::ServerHttp => recipe.browser.as_ref().is_some_and(|b| {
                    let text = check.text.as_deref().unwrap_or("");
                    let expression = text.strip_prefix("status=").is_some_and(|s| {
                        s.len() == 3 && s.parse::<u16>().is_ok_and(|n| (100..=599).contains(&n))
                    }) || text.strip_prefix("sha256=").is_some_and(|s| {
                        s.len() == 64
                            && s.bytes()
                                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    });
                    expression
                        && b.http_reads
                            .iter()
                            .any(|p| Some(p.id.as_str()) == check.target.as_deref())
                }),
                CheckMethod::BrowserText => recipe.browser.as_ref().is_some_and(|b| {
                    b.actions.iter().any(|s| {
                        Some(s.id.as_str()) == check.target.as_deref()
                            && matches!(s.action, crate::browser::Action::ReadText { .. })
                    })
                }),
                CheckMethod::BrowserResponse => recipe.browser.as_ref().is_some_and(|b| {
                    let text = check.text.as_deref().unwrap_or("");
                    let valid_expression = text.strip_prefix("sha256=").is_some_and(|s| {
                        s.len() == 64
                            && s.bytes()
                                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    }) || ["body_match=true", "body_match=false"]
                        .contains(&text)
                        || text.strip_prefix("status=").is_some_and(|s| {
                            s.len() == 3 && s.parse::<u16>().is_ok_and(|n| (100..=599).contains(&n))
                        });
                    valid_expression
                        && b.responses
                            .iter()
                            .any(|w| Some(w.id.as_str()) == check.target.as_deref())
                }),
                CheckMethod::DatabaseRows => recipe.browser.as_ref().is_some_and(|b| {
                    b.database
                        .iter()
                        .any(|q| Some(q.id.as_str()) == check.target.as_deref())
                }),
                _ => true,
            };
            if !valid {
                return Err(invalid(
                    "判定条件のtargetに対応するブラウザ観測がありません",
                ));
            }
        }
    }
    Ok(())
}
fn git(root: &Path, args: &[&str]) -> io::Result<Vec<u8>> {
    let mut command = Command::new("git");
    command
        .args([
            "--no-replace-objects",
            "-c",
            "core.hooksPath=/dev/null",
            "-C",
        ])
        .arg(root)
        .args(args)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null());
    // 取得に必要なネットワーク設定だけをGitへ渡す。対象アプリへは渡さない。
    for name in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
        "GIT_SSL_CAINFO",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let output = command.output()?;
    if !output.status.success() {
        return Err(invalid(format!(
            "Gitの固定版を読み取れません: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(output.stdout)
}
fn git_text(root: &Path, args: &[&str]) -> io::Result<String> {
    Ok(String::from_utf8(git(root, args)?)
        .map_err(io::Error::other)?
        .trim()
        .into())
}
fn expected_origin(repository: &str, origin: &str) -> bool {
    origin == format!("https://github.com/{repository}")
        || origin == format!("https://github.com/{repository}.git")
        || origin == format!("git@github.com:{repository}.git")
}
/// 表示用にGitHubのリポジトリ名だけを返す。認証情報を含むURLは受け付けない。
pub fn repository_name(root: &Path) -> io::Result<String> {
    let origin = git_text(root, &["remote", "get-url", "origin"])?;
    let repository = origin
        .strip_prefix("https://github.com/")
        .or_else(|| origin.strip_prefix("git@github.com:"))
        .ok_or_else(|| invalid("GitHubのoriginを持つリポジトリが必要です"))?;
    let repository = repository.strip_suffix(".git").unwrap_or(repository);
    crate::workspace::validate_source(&CodeReference {
        repository: repository.into(),
        commit: "0".repeat(40),
        path: "README.md".into(),
        line: 1,
        end_line: 1,
    })?;
    Ok(repository.into())
}
/// 固定コミットのファイル一覧を作る。`prefix` を指定した場合は、そのディレクトリの中だけを対象にする。
fn inventory(
    root: &Path,
    repository: &str,
    commit: &str,
    prefix: Option<&str>,
) -> io::Result<(String, Vec<SnapshotFile>)> {
    let found = git_text(
        root,
        &["rev-parse", "--verify", &format!("{commit}^{{commit}}")],
    )?;
    if found != commit
        || !expected_origin(
            repository,
            &git_text(root, &["remote", "get-url", "origin"])?,
        )
    {
        return Err(invalid(format!(
            "{repository} のローカルのコミットまたはoriginが指定と一致しません"
        )));
    }
    let tree = git_text(root, &["rev-parse", &format!("{commit}^{{tree}}")])?;
    let entries = git(root, &["ls-tree", "-rz", "--full-tree", commit])?;
    let mut files = vec![];
    let mut total = 0;
    for entry in entries.split(|b| *b == 0).filter(|e| !e.is_empty()) {
        let entry = std::str::from_utf8(entry).map_err(io::Error::other)?;
        let (header, path) = entry
            .split_once('\t')
            .ok_or_else(|| invalid("Gitのファイル情報が不正です"))?;
        if prefix.is_some_and(|p| path != p && !path.starts_with(&format!("{p}/"))) {
            continue;
        }
        relative(path)?;
        let fields: Vec<_> = header.split_whitespace().collect();
        if fields.len() != 3 || fields[1] != "blob" || !["100644", "100755"].contains(&fields[0]) {
            return Err(invalid(
                "symlink・submodule・特殊ファイルはこの実行方式の対象外です",
            ));
        }
        let bytes = git_text(root, &["cat-file", "-s", fields[2]])?
            .parse::<u64>()
            .map_err(io::Error::other)?;
        if bytes > MAX_FILE {
            return Err(invalid("対象ファイルが4MiBを超えています"));
        }
        total += bytes;
        if total > MAX_TOTAL || files.len() >= MAX_FILES {
            return Err(invalid("固定版は512ファイル・各4MiB・合計32MiB以内です"));
        }
        let blob = git(root, &["cat-file", "blob", fields[2]])?;
        let lines = std::str::from_utf8(&blob)
            .map(|s| s.lines().count() as u32)
            .unwrap_or(0);
        files.push(SnapshotFile {
            path: path.into(),
            blob: fields[2].into(),
            mode: fields[0].into(),
            bytes,
            lines,
        });
    }
    Ok((tree, files))
}
/// ファイルとして起動するものが、対象の固定コミットにあるかを確かめる。
fn check_target_files(recipe: &Recipe, files: &[SnapshotFile]) -> io::Result<()> {
    let readback = recipe.trial.as_ref().and_then(|t| t.readback.as_ref());
    let starts = [
        (!matches!(recipe.runtime, Runtime::Http | Runtime::Browser))
            .then_some((recipe.launch, recipe.entrypoint.as_str())),
        readback.map(|r| (r.launch, r.entrypoint.as_str())),
        recipe
            .server
            .as_ref()
            .map(|s| (s.launch, s.entrypoint.as_str())),
    ];
    let has = |path: &str| files.iter().any(|f| f.path == path);
    if !has(&recipe.source.path)
        || starts
            .iter()
            .flatten()
            .any(|(launch, path)| launch.is_file() && !has(path))
    {
        return Err(invalid(
            "参照ファイルまたは起動ファイルが固定コミットにありません",
        ));
    }
    Ok(())
}
fn copy_snapshot(root: &Path, destination: &Path, files: &[SnapshotFile]) -> io::Result<()> {
    fs::create_dir(destination)?;
    for file in files {
        let bytes = git(root, &["cat-file", "blob", &file.blob])?;
        if bytes.len() as u64 != file.bytes {
            return Err(invalid("Git blobの長さが一致しません"));
        }
        let path = destination.join(&file.path);
        fs::create_dir_all(path.parent().ok_or_else(|| invalid("出力先がありません"))?)?;
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        out.write_all(&bytes)?;
        out.sync_all()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                path,
                fs::Permissions::from_mode(if file.mode == "100755" { 0o555 } else { 0o444 }),
            )?;
        }
    }
    Ok(())
}
fn snapshot_unchanged(root: &Path, snapshot: &Path, files: &[SnapshotFile]) -> bool {
    fn paths(directory: &Path, base: &Path, found: &mut BTreeSet<String>) -> io::Result<()> {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            if metadata.is_dir() {
                paths(&entry.path(), base, found)?;
            } else if metadata.is_file() {
                found.insert(
                    entry
                        .path()
                        .strip_prefix(base)
                        .map_err(io::Error::other)?
                        .to_string_lossy()
                        .into_owned(),
                );
                if found.len() > MAX_FILES {
                    return Err(invalid("試行中にファイルが増えました"));
                }
            } else {
                return Err(invalid("固定版に特殊ファイルが追加されました"));
            }
        }
        Ok(())
    }
    let mut found = BTreeSet::new();
    if paths(snapshot, snapshot, &mut found).is_err()
        || found != files.iter().map(|f| f.path.clone()).collect()
    {
        return false;
    }
    files.iter().all(|file| {
        let path = snapshot.join(&file.path);
        fs::symlink_metadata(&path).is_ok_and(|m| m.is_file() && m.len() == file.bytes)
            && git(root, &["cat-file", "blob", &file.blob])
                .ok()
                .zip(fs::read(&path).ok())
                .is_some_and(|(a, b)| a == b)
    })
}

/// PATHの先頭がpyenv等のシェルスクリプトでも、環境変数を消した試行で動く実体を使う。
fn runtime_path(runtime: &Runtime) -> io::Result<PathBuf> {
    let (name, probe): (_, &[&str]) = match runtime {
        Runtime::Node | Runtime::Browser => ("node", &["-p", "process.execPath"]),
        Runtime::Python3 => ("python3", &["-c", "import sys; print(sys.executable)"]),
        Runtime::Http => {
            return Err(invalid(
                "HTTPの操作はコンテナの公開ポートへ基盤が行います。Node.jsやPython 3の実行環境は使いません",
            ));
        }
    };
    let candidate = executable(name)?;
    // ホストが用意した実行環境に実体の場所だけを尋ねる。対象のコードは渡さない。
    let resolved = Command::new(&candidate)
        .args(probe)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| PathBuf::from(s.trim()))
        .filter(|p| p.is_absolute() && p.is_file());
    match resolved {
        Some(path) => fs::canonicalize(path),
        None => Ok(candidate),
    }
}
pub fn runtime_available(runtime: &Runtime) -> Result<(PathBuf, String), String> {
    if *runtime == Runtime::Http {
        return crate::container::engine()
            .map(|version| (PathBuf::from("docker"), format!("Docker {version}")));
    }
    let program = runtime_path(runtime).map_err(|e| e.to_string())?;
    let output = Command::new(&program)
        .arg("--version")
        .env_clear()
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{}を起動できません: {e}", program.display()))?;
    if !output.status.success() {
        return Err(format!(
            "{}は環境変数なしで起動できません: {}",
            program.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let version = String::from_utf8_lossy(if output.stdout.is_empty() {
        &output.stderr
    } else {
        &output.stdout
    })
    .trim()
    .to_owned();
    Ok((program, version))
}
fn executable(name: &str) -> io::Result<PathBuf> {
    let paths = std::env::var_os("PATH").ok_or_else(|| invalid("PATHを確認してください"))?;
    for directory in std::env::split_paths(&paths) {
        let path = directory.join(name);
        if path.is_file() {
            return fs::canonicalize(path);
        }
    }
    Err(invalid(format!("{name}を実行できません")))
}
fn load_observations(
    path: &Path,
    correlation: &Correlation,
    files: &[SnapshotFile],
) -> (Vec<Observation>, Vec<String>, ObservationRead) {
    let read = (|| -> io::Result<Vec<Observation>> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_file() || metadata.len() > MAX_TRACE {
            return Err(invalid("観測ファイルは通常ファイル・1MiB以内です"));
        }
        let mut input = String::new();
        File::open(path)?
            .take(MAX_TRACE + 1)
            .read_to_string(&mut input)?;
        if input.len() as u64 > MAX_TRACE {
            return Err(invalid("観測ファイルが1MiBを超えています"));
        }
        let mut observations = vec![];
        for (i, line) in input.lines().enumerate() {
            let o: Observation = serde_json::from_str(line).map_err(io::Error::other)?;
            if o.schema_version != 1
                || o.sequence != i as u64 + 1
                || &o.correlation != correlation
                || !files.iter().any(|f| f.path == o.path)
                || o.line == 0
                || files
                    .iter()
                    .find(|f| f.path == o.path)
                    .is_none_or(|f| o.line > f.lines)
                || observations.len() >= 2000
            {
                return Err(invalid("観測の順序・試行識別子・コード参照が不正です"));
            }
            text(&o.assertion)?;
            text(&o.subject)?;
            text(&o.detail)?;
            relative(&o.path)?;
            observations.push(o);
        }
        Ok(observations)
    })();
    match read {
        Ok(o) => (o, vec![], ObservationRead::Valid),
        Err(e) if e.kind() == io::ErrorKind::NotFound => (
            vec![],
            vec!["観測ファイルを取得できませんでした。条件の失敗とは断定しません".into()],
            ObservationRead::Missing,
        ),
        Err(e) => (
            vec![],
            vec![format!("観測を採用できません: {e}")],
            ObservationRead::Invalid,
        ),
    }
}

pub fn validate_trace(trace: &ExecutionTrace, manifest: &Manifest) -> io::Result<()> {
    validate_recipe(&trace.recipe)?;
    let hash = |value: &str| {
        value.len() == 40
            && value
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    };
    if trace.schema_version != 1
        || trace.correlation != manifest.correlation
        || !["local", "bubblewrap"].contains(&trace.mode.as_str())
        || manifest.mode != "pinned_execution"
        || manifest.condition.id != trace.recipe.check_id
        || manifest.timeout_ms != trace.recipe.timeout_ms
        || !hash(&trace.tree)
        || trace.files.len() > MAX_FILES
        || !trace
            .files
            .iter()
            .any(|f| f.path == trace.recipe.source.path)
        || !matches!(trace.recipe.runtime, Runtime::Http | Runtime::Browser)
            && trace.recipe.launch.is_file()
            && !trace
                .files
                .iter()
                .any(|f| f.path == trace.recipe.entrypoint)
        || trace.python.is_some() != trace.recipe.python.is_some()
        || trace.containers.is_some() != trace.recipe.containers.is_some()
        || (trace.observation_read == ObservationRead::Missing
            || trace.observation_read == ObservationRead::Invalid)
            && !trace.observations.is_empty()
        || trace.observations.len() > 2000
        || trace.diagnostics.len() > 100
        || trace.diagnostics.iter().any(|d| d.len() > 16384)
    {
        return Err(invalid("実行対象・観測・試行の対応が不正です"));
    }
    let mut seen = BTreeSet::new();
    let mut total = 0;
    for file in &trace.files {
        relative(&file.path)?;
        if file.bytes > MAX_FILE {
            return Err(invalid("固定版のファイルサイズが上限を超えています"));
        }
        total += file.bytes;
        if !seen.insert(&file.path)
            || file.bytes > MAX_FILE
            || total > MAX_TOTAL
            || !hash(&file.blob)
            || !["100644", "100755"].contains(&file.mode.as_str())
        {
            return Err(invalid("固定版のファイル情報が不正です"));
        }
    }
    for (i, o) in trace.observations.iter().enumerate() {
        if o.schema_version != 1
            || o.sequence != i as u64 + 1
            || o.correlation != trace.correlation
            || o.line == 0
            || trace
                .files
                .iter()
                .find(|f| f.path == o.path)
                .is_none_or(|f| o.line > f.lines)
        {
            return Err(invalid("観測の参照先・順序・識別子が一致しません"));
        }
        text(&o.assertion)?;
        text(&o.subject)?;
        text(&o.detail)?;
    }
    Ok(())
}

pub fn assess_trace(
    manifest: &Manifest,
    trace: &ExecutionTrace,
    criterion: &Criterion,
) -> Assessment {
    let mut result = Assessment {
        condition_id: manifest.condition.id.clone(), condition_version: manifest.condition.version,
        verdict: Verdict::Unknown, evidence: vec![], reason: String::new(),
        unconfirmed: vec!["対象プログラムが出力した観測です。独立した読み出し、参照した行への到達、外部依存先・医療業務の利用可否は確認していない".into()],
        next_action: "不足・食い違いの出所を確認し、新しい試行で観測する".into(),
    };
    result.unconfirmed.extend(trace.diagnostics.clone());
    if validate_trace(trace, manifest).is_err() {
        result.reason = "試行・観測・固定版の対応を照合できない".into();
        return result;
    }
    if let Some(spec) = &trace.recipe.trial {
        let Some(outcome) = &trace.trial else {
            result.reason = "試験案はあるが、判定条件ごとの結果が保存されていない".into();
            return result;
        };
        return crate::trial::assess(manifest, spec, outcome, trace.snapshot_unchanged);
    }
    if !trace.snapshot_unchanged {
        result.reason = "試行用の固定版が開始時と終了時で異なり、結果の適用先を確認できない".into();
        return result;
    }
    if trace.timed_out || trace.output_limit || trace.exit_code != Some(0) {
        result.reason =
            "対象の起動・実行・終了を確認できない。選択した方式と標準エラーを確認する".into();
        return result;
    }
    if trace.observation_read == ObservationRead::Invalid {
        result.reason = "試行は終了したが、観測の識別子・順序・参照先・形式を採用できない".into();
        return result;
    }
    if trace.observation_read == ObservationRead::Missing {
        result.reason =
            "試行は正常に終了したが、観測ファイルがない。条件の失敗とは断定しない".into();
        return result;
    }
    if !trace.diagnostics.is_empty() {
        result.reason = "観測の採用条件を確認できない".into();
        return result;
    }
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
    result.evidence = matching
        .iter()
        .map(|o| format!("observations.jsonl#{}", o.sequence))
        .collect();
    if pass && fail {
        result.reason = "同じ試行・観測名・対象の成功と失敗が食い違う".into();
    } else if pass {
        result.verdict = Verdict::Satisfied;
        result.reason = "選択した観測名・対象について、固定版の実行から成功の観測がある".into();
        result.next_action = "観測が条件の意味を十分に確かめるか、別経路の状態と照合する".into();
    } else if fail {
        result.verdict = Verdict::NotSatisfied;
        result.reason =
            "選択した観測名・対象について、固定版の実行から明示的な失敗の観測がある".into();
    } else {
        result.reason = "選択した観測名・対象に対応する記録がない。条件の失敗とは断定しない".into();
    }
    result
}

pub fn execute_with_mode(
    repository_root: &Path,
    output: &Path,
    recipe: Recipe,
    condition: Condition,
    previous_run: Option<String>,
    mode: ExecutionMode,
) -> io::Result<RunResult> {
    execute_with_dependencies(
        repository_root,
        None,
        output,
        recipe,
        condition,
        previous_run,
        mode,
    )
}

type FetchedDependency<'a> = (
    &'a crate::python_env::Dependency,
    PathBuf,
    String,
    Vec<SnapshotFile>,
);

/// 依存先を `dependency_base/owner/repo` に取得済みの固定コミットから読み込んで実行する。
pub fn execute_with_dependencies(
    repository_root: &Path,
    dependency_base: Option<&Path>,
    output: &Path,
    recipe: Recipe,
    condition: Condition,
    previous_run: Option<String>,
    mode: ExecutionMode,
) -> io::Result<RunResult> {
    mode.ensure_allowed()?;
    validate_recipe(&recipe)?;
    if mode == ExecutionMode::Local
        && recipe
            .trial
            .as_ref()
            .is_some_and(|t| t.fault == crate::trial::TrialFault::NetworkUnavailable)
    {
        return Err(invalid(
            "local方式では通信を遮断できません。Linuxのbubblewrap方式で試してください",
        ));
    }
    if mode == ExecutionMode::Bubblewrap && recipe.server.is_some() {
        return Err(invalid(
            "bubblewrap方式ではプロセスごとにネットワークを分けるため、待つプロセスとの通信を試せません。ローカルへは自動で切り替えません",
        ));
    }
    if mode == ExecutionMode::Bubblewrap && recipe.containers.is_some() {
        return Err(invalid(
            "bubblewrap方式ではコンテナの待ち受けをホストの127.0.0.1へ公開できません。ローカルへは自動で切り替えません",
        ));
    }
    let root = fs::canonicalize(repository_root)?;
    let (tree, files) = inventory(
        &root,
        &recipe.source.repository,
        &recipe.source.commit,
        (recipe.runtime == Runtime::Browser).then_some(recipe.source.path.as_str()),
    )?;
    check_target_files(&recipe, &files)?;
    let (program, runtime_version) = runtime_available(&recipe.runtime).map_err(invalid)?;
    let mut dependencies: Vec<FetchedDependency> = vec![];
    if let Some(setup) = &recipe.python {
        if !crate::python_env::supports_safe_path(&runtime_version) {
            return Err(invalid(format!(
                "Pythonの読み込み先を指定した試行には Python 3.11 以上が必要です（現在: {runtime_version}）"
            )));
        }
        for dependency in &setup.dependencies {
            let base = dependency_base.ok_or_else(|| {
                invalid("この起動方法では依存先を扱えません。アプリの試験用の取得を使ってください")
            })?;
            let dependency_root =
                fs::canonicalize(base.join(&dependency.repository)).map_err(|_| {
                    invalid(format!(
                        "依存先 {} のコミット {} を取得していません",
                        dependency.repository, dependency.commit
                    ))
                })?;
            let (dependency_tree, dependency_files) = inventory(
                &dependency_root,
                &dependency.repository,
                &dependency.commit,
                dependency.root.as_deref(),
            )?;
            if dependency_files.is_empty() {
                return Err(invalid(format!(
                    "依存先 {} の読み込み先 {} にファイルがありません",
                    dependency.repository,
                    dependency.root.as_deref().unwrap_or("（最上位）")
                )));
            }
            dependencies.push((
                dependency,
                dependency_root,
                dependency_tree,
                dependency_files,
            ));
        }
    }
    let (directory, correlation) = new_run_directory(output)?;
    let directory = fs::canonicalize(directory)?;
    let snapshot = directory.join("source");
    let work = directory.join("work");
    copy_snapshot(&root, &snapshot, &files)?;
    fs::create_dir(&work)?;
    let source_text = fs::read_to_string(snapshot.join(&recipe.source.path))?;
    if recipe.source.end_line as usize > source_text.lines().count() {
        return Err(invalid("コード参照の行範囲が固定版を超えています"));
    }
    let python = match &recipe.python {
        Some(setup) => Some(prepare_python(
            &root,
            &recipe,
            setup,
            &tree,
            &files,
            &dependencies,
            &directory,
        )?),
        None => None,
    };
    // 試行ごとに新しい架空の値を使い、既存のデータと区別する。
    let prefix = if recipe.runtime == Runtime::Browser {
        "EEL-TEST-"
    } else {
        "eel-test-"
    };
    let test_value = format!("{prefix}{}", correlation.run_id.trim_start_matches("run-"));
    let port = match recipe.server {
        Some(_) => Some(crate::server::free_port()?),
        None => None,
    };
    let mut session = match &recipe.containers {
        Some(spec) => Some(crate::container::Session::start(&correlation.run_id, spec)?),
        None => None,
    };
    let container_ports = session.as_ref().map(|s| s.ports()).unwrap_or_default();
    let source_root = if mode == ExecutionMode::Bubblewrap {
        "/source".to_owned()
    } else {
        snapshot.to_string_lossy().into_owned()
    };
    let substitute = |args: &[String]| -> Vec<String> {
        args.iter()
            .map(|a| {
                let mut value = crate::trial::substitute(a, &test_value)
                    .replace(crate::trial::SOURCE_ROOT, &source_root);
                if let Some(port) = port {
                    value = value.replace(crate::server::PORT, &port.to_string());
                }
                crate::container::substitute_ports(&value, &container_ports)
            })
            .collect()
    };
    let args = substitute(&recipe.args);
    write_json_new(
        &work.join("input.json"),
        &crate::trial::substitute_json(&recipe.input, &test_value),
    )?;
    // 認証などの準備は正常なDBで行い、その後に保存操作の故障を注入する。
    let mut browser = match (&recipe.browser, session.as_ref()) {
        (Some(spec), Some(session)) if session.all_ready() => {
            Some(crate::browser::Session::prepare(
                spec,
                session,
                &directory,
                &correlation.run_id,
                &test_value,
                recipe.timeout_ms,
            )?)
        }
        _ => None,
    };
    let mut fault = match &recipe.trial {
        Some(spec) => Some(crate::trial::apply_fault(spec, &work)?),
        None => None,
    };
    if let (Some(spec), Some(session), Some((_, effective))) =
        (recipe.trial.as_ref(), session.as_mut(), fault.as_mut())
    {
        if session.all_ready() && browser.as_ref().is_none_or(|b| b.prepared()) {
            match spec.fault {
                crate::trial::TrialFault::DatabaseStopped => {
                    *effective = session.stop_database()?;
                }
                crate::trial::TrialFault::DatabaseUnreachable => {
                    *effective = session.disconnect_database()?;
                }
                crate::trial::TrialFault::VolumeReadOnly => {
                    *effective = session.remount_database_read_only()?;
                }
                _ => {}
            }
        } else if matches!(
            spec.fault,
            crate::trial::TrialFault::DatabaseStopped
                | crate::trial::TrialFault::DatabaseUnreachable
                | crate::trial::TrialFault::VolumeReadOnly
        ) {
            *effective = false;
        }
    }
    let launch = Launcher {
        mode,
        program: &program,
        snapshot: &snapshot,
        work: &work,
        correlation: &correlation,
        python: python.as_ref().map(|p| &p.paths),
    };
    let operate = if matches!(recipe.runtime, Runtime::Http | Runtime::Browser) {
        None
    } else {
        Some(start_for(
            recipe.launch,
            &recipe.entrypoint,
            python.as_ref(),
            "operate",
        )?)
    };
    let mut manifest = make_manifest(correlation.clone(), condition)?;
    manifest.mode = "pinned_execution".into();
    manifest.timeout_ms = recipe.timeout_ms;
    manifest.previous_run = previous_run;
    manifest.initial_storage = if mode == ExecutionMode::Local {
        "固定コミットからsourceへコピー。workと入力は試行ごとに新規。外部ファイル・通信はOSで隔離していない"
    } else {
        "Linuxのnamespaceで外部ネットワークを分離する方式を要求。固定版は読み取り専用、workだけを書き込み共有。起動失敗時はローカルへ切り替えない"
    }.to_owned();
    if python.is_some() {
        manifest.initial_storage.push_str("。依存先は固定コミットの読み込み先だけをdepsへコピーし、インストール用のスクリプトは実行していない。パッケージ情報はpyproject.tomlの静的な値から基盤が作った");
    }
    if let Some(port) = port {
        manifest.initial_storage.push_str(&format!(
            "。待つプロセスとの通信用に127.0.0.1のポート{port}を基盤が割り当てた。待ち受けるアドレスは対象の引数で決まり、OSでは制限していない"
        ));
    }
    if session.is_some() {
        manifest.initial_storage.push_str(
            "。試験専用のコンテナ・ネットワーク・ボリュームをこの試行だけで作り、待ち受けは127.0.0.1だけに公開する。終了後に消す。既存のコンテナは使わない",
        );
    }
    manifest.observation_scope =
        "固定版と構造化された自己申告の観測、stdout/stderr・終了。実行経路と外部依存先は未確認"
            .into();
    manifest.command = match &operate {
        Some(operate) => launch.describe(operate, &args),
        None if recipe.runtime == Runtime::Browser => {
            vec!["browser".into(), "preparation -> fault -> actions".into()]
        }
        None => recipe
            .http
            .as_ref()
            .map(|op| {
                vec![
                    "http".into(),
                    op.method.clone(),
                    format!("{} {}", op.service, op.path),
                ]
            })
            .unwrap_or_default(),
    };
    write_json_new(&directory.join("manifest.json"), &manifest)?;
    write_json_new(&directory.join("recipe.json"), &recipe)?;
    let recorder = Recorder::new(&directory, correlation.clone())?;
    let server = match (&recipe.server, port) {
        (Some(spec), Some(port)) => Some(start_server(
            &launch,
            spec,
            &substitute(&spec.args),
            port,
            python.as_ref(),
            &recipe,
            fault.as_mut(),
            &directory,
            &recorder,
        )?),
        _ => None,
    };
    let proceed = server.as_ref().is_none_or(|(phase, _)| {
        phase.ready
            || recipe
                .trial
                .as_ref()
                .is_some_and(|t| t.fault == crate::trial::TrialFault::ServerNotStarted)
    }) && session.as_ref().is_none_or(|s| s.all_ready())
        && (recipe.runtime != Runtime::Browser || browser.as_ref().is_some_and(|b| b.prepared()));
    let mut http_result = None;
    let mut http_exit = None;
    let (status, timed_out, output_limit) = if proceed {
        if let Some(browser) = browser.as_mut() {
            let report = browser.run()?;
            http_exit = Some(if report.completed { 0 } else { 1 });
            recorder.record(
                Source::Process,
                EventKind::ProcessExited,
                report.detail.clone(),
            )?;
            (None, false, false)
        } else if let (Some(op), Some(session)) = (&recipe.http, session.as_ref()) {
            match session.request(op, &snapshot, recipe.timeout_ms) {
                Ok(result) => {
                    let code = result.status.unwrap_or(0);
                    fs::write(
                        directory.join("stdout.txt"),
                        format!("{}\n{}", result.detail, result.body_excerpt),
                    )?;
                    fs::write(directory.join("stderr.txt"), "")?;
                    recorder.record(
                        Source::Process,
                        EventKind::ProcessExited,
                        format!("HTTP {code}（コンテナ {}）", op.service),
                    )?;
                    http_exit = Some(if (200..400).contains(&code) { 0 } else { 1 });
                    http_result = Some(result);
                    (None, false, false)
                }
                Err(error) => {
                    fs::write(directory.join("stdout.txt"), "")?;
                    fs::write(directory.join("stderr.txt"), error.to_string())?;
                    recorder.record(
                        Source::Process,
                        EventKind::ProcessExited,
                        format!("HTTPの操作に失敗: {error}"),
                    )?;
                    http_exit = Some(1);
                    (None, false, false)
                }
            }
        } else if let Some(operate) = &operate {
            let stdout = File::create(directory.join("stdout.txt"))?;
            let stderr = File::create(directory.join("stderr.txt"))?;
            let mut command = launch.command(operate, &args)?;
            command.stdout(stdout).stderr(stderr);
            let child = ChildGuard(command.spawn()?);
            recorder.record(
                Source::Process,
                EventKind::ProcessStarted,
                format!(
                    "pid={}、固定コミット{}、起動{}（{}）",
                    child.0.id(),
                    recipe.source.commit,
                    recipe.entrypoint,
                    recipe.launch.label()
                ),
            )?;
            let (status, timed_out, output_limit) = wait(
                child,
                &directory,
                ["stdout.txt", "stderr.txt"],
                recipe.timeout_ms,
            )?;
            recorder.record(
                Source::Process,
                if timed_out {
                    EventKind::ProcessTimedOut
                } else {
                    EventKind::ProcessExited
                },
                format!("{status}、出力上限超過={output_limit}"),
            )?;
            (Some(status), timed_out, output_limit)
        } else {
            (None, false, false)
        }
    } else {
        (None, false, false)
    };
    let server = match server {
        Some((phase, child)) => Some(finish_server(phase, child, &directory, &recorder)?),
        None => None,
    };
    if !directory.join("stdout.txt").exists() {
        fs::write(directory.join("stdout.txt"), "")?;
        fs::write(directory.join("stderr.txt"), "")?;
    }
    let browser_report = browser.as_ref().map(|b| b.report());
    if recipe
        .trial
        .as_ref()
        .is_some_and(|t| t.fault == crate::trial::TrialFault::HttpRejected)
        && let Some((_, effective)) = fault.as_mut()
    {
        *effective = recipe
            .browser
            .as_ref()
            .zip(browser_report.as_ref())
            .is_some_and(|(spec, report)| crate::browser::rejection_observed(spec, report));
    }
    drop(browser);
    let database_reads = match (&recipe.browser, session.as_ref()) {
        (Some(spec), Some(session))
            if browser_report
                .as_ref()
                .is_some_and(|r| r.prepared && r.steps.iter().any(|s| s.phase == "action")) =>
        {
            spec.database
                .iter()
                .map(|q| session.read_database(q, &test_value))
                .collect()
        }
        _ => vec![],
    };
    let http_reads = match (&recipe.browser, session.as_ref()) {
        (Some(spec), Some(session))
            if browser_report
                .as_ref()
                .is_some_and(|r| r.prepared && r.steps.iter().any(|s| s.phase == "action")) =>
        {
            spec.http_reads
                .iter()
                .map(|p| crate::http_probe::read(p, session))
                .collect()
        }
        _ => vec![],
    };
    let database_query = session.as_ref().and_then(|s| s.query_database());
    let mut container_phase = session.as_ref().map(|s| s.snapshot());
    if let Some(phase) = container_phase.as_mut() {
        phase.database_query = database_query;
        phase.http = http_result.clone();
        phase.browser = browser_report;
        phase.database_reads = database_reads;
        phase.http_reads = http_reads;
    }
    let leftovers = match session.as_mut() {
        Some(session) => match session.cleanup() {
            Ok(left) => left,
            Err(error) => vec![error.to_string()],
        },
        None => vec![],
    };
    if let Some(phase) = container_phase.as_mut() {
        phase.leftovers.clone_from(&leftovers);
    }
    let completed = (status.is_some() || http_exit.is_some()) && !timed_out && !output_limit;
    let readback = match recipe.trial.as_ref().and_then(|t| t.readback.as_ref()) {
        Some(spec) if completed => {
            let readback_args = substitute(&spec.args);
            let start = start_for(spec.launch, &spec.entrypoint, python.as_ref(), "readback")?;
            let mut command = launch.command(&start, &readback_args)?;
            command
                .stdout(File::create(directory.join("readback-stdout.txt"))?)
                .stderr(File::create(directory.join("readback-stderr.txt"))?);
            let child = ChildGuard(command.spawn()?);
            recorder.record(
                Source::Process,
                EventKind::ProcessStarted,
                format!(
                    "読み出し: pid={}、起動ファイル{}",
                    child.0.id(),
                    spec.entrypoint
                ),
            )?;
            let (status, timed_out, output_limit) = wait(
                child,
                &directory,
                ["readback-stdout.txt", "readback-stderr.txt"],
                recipe.timeout_ms,
            )?;
            recorder.record(
                Source::Process,
                if timed_out {
                    EventKind::ProcessTimedOut
                } else {
                    EventKind::ProcessExited
                },
                format!("読み出し: {status}、出力上限超過={output_limit}"),
            )?;
            Some(crate::trial::Phase {
                command: launch.describe(&start, &readback_args),
                exit_code: status.code(),
                timed_out,
                output_limit,
                stdout_excerpt: excerpt(&directory.join("readback-stdout.txt"), 8192)?,
                stderr_excerpt: excerpt(&directory.join("readback-stderr.txt"), 8192)?,
            })
        }
        _ => None,
    };
    let (observations, mut diagnostics, observation_read) =
        load_observations(&work.join("observations.jsonl"), &correlation, &files);
    let stdout_excerpt = excerpt(&directory.join("stdout.txt"), 8192)?;
    let stderr_excerpt = excerpt(&directory.join("stderr.txt"), 8192)?;
    match (status, http_exit, proceed) {
        (_, Some(_), _) => {}
        (None, None, false) => diagnostics
            .push("待つプロセス・試験用コンテナ・ブラウザのいずれかが準備できなかったため、操作していない".into()),
        (Some(status), _, _) if !status.success() => diagnostics.push(format!(
            "起動・実行・終了を確認できませんでした: {}",
            stderr_excerpt.chars().take(2000).collect::<String>()
        )),
        _ => {}
    }
    if !leftovers.is_empty() {
        diagnostics.push(format!(
            "試行の後に試験用のコンテナまたはボリュームが残った: {}",
            leftovers.join("、")
        ));
    }
    let exit_code = status.and_then(|s| s.code()).or(http_exit);
    let trial = match (&recipe.trial, fault) {
        (Some(spec), Some((fault_applied, fault_effective))) => {
            let (state, state_text) = crate::trial::read_state(spec, &work, &test_value);
            let stdout = excerpt(&directory.join("stdout.txt"), MAX_TRACE as usize)?;
            let stderr = excerpt(&directory.join("stderr.txt"), MAX_TRACE as usize)?;
            let readback_stdout = match &readback {
                Some(_) => excerpt(&directory.join("readback-stdout.txt"), MAX_TRACE as usize)?,
                None => String::new(),
            };
            let server_output = match &server {
                Some(_) => format!(
                    "{}\n{}",
                    excerpt(&directory.join("server-stdout.txt"), MAX_TRACE as usize)?,
                    excerpt(&directory.join("server-stderr.txt"), MAX_TRACE as usize)?
                ),
                None => String::new(),
            };
            let results = crate::trial::evaluate(
                spec,
                &test_value,
                &crate::trial::Outputs {
                    exit_code,
                    completed,
                    stdout: &stdout,
                    stderr: &stderr,
                    readback: readback.as_ref().map(|p| (p, readback_stdout.as_str())),
                    state: state.as_ref(),
                    state_text: state_text.as_deref(),
                    server: server.as_ref().map(|p| (p, server_output.as_str())),
                    containers: container_phase.as_ref(),
                },
            );
            Some(crate::trial::TrialOutcome {
                test_value: test_value.clone(),
                fault: spec.fault,
                fault_applied,
                fault_effective,
                readback,
                state,
                results,
                server,
                containers: container_phase.clone(),
            })
        }
        _ => None,
    };
    let unchanged = snapshot_unchanged(&root, &snapshot, &files)
        && dependencies
            .iter()
            .enumerate()
            .all(|(i, (_, dependency_root, _, dependency_files))| {
                snapshot_unchanged(
                    dependency_root,
                    &directory.join("deps").join(i.to_string()),
                    dependency_files,
                )
            });
    let trace = ExecutionTrace {
        schema_version: 1,
        correlation,
        recipe,
        tree,
        files,
        runtime_path: program.to_string_lossy().into_owned(),
        runtime_version,
        exit_code,
        timed_out,
        output_limit,
        snapshot_unchanged: unchanged,
        observations,
        observation_read,
        diagnostics,
        mode: mode.label().into(),
        stdout_excerpt,
        stderr_excerpt,
        trial,
        python: python.map(|p| p.trace),
        containers: container_phase,
    };
    write_json_new(&directory.join("execution.json"), &trace)?;
    // 観測は複製して取り込める。状態の答え合わせはこの判定へ混ぜない。
    let mut out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join("observations.jsonl"))?;
    for observation in &trace.observations {
        serde_json::to_writer(&mut out, observation).map_err(io::Error::other)?;
        out.write_all(b"\n")?;
    }
    out.sync_all()?;
    finish_run(directory, manifest)
}

fn excerpt(path: &Path, limit: usize) -> io::Result<String> {
    let mut bytes = vec![];
    File::open(path)?
        .take(limit as u64)
        .read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn wait(
    mut child: ChildGuard,
    directory: &Path,
    outputs: [&str; 2],
    timeout_ms: u64,
) -> io::Result<(std::process::ExitStatus, bool, bool)> {
    let started = Instant::now();
    let mut timed_out = false;
    let mut output_limit = false;
    let status = loop {
        if let Some(status) = child.0.try_wait()? {
            break status;
        }
        output_limit = outputs
            .iter()
            .any(|p| fs::metadata(directory.join(p)).is_ok_and(|m| m.len() > MAX_TRACE));
        if started.elapsed() >= Duration::from_millis(timeout_ms) || output_limit {
            timed_out = !output_limit;
            child.0.kill()?;
            break child.0.wait()?;
        }
        thread::sleep(Duration::from_millis(5));
    };
    output_limit |= outputs
        .iter()
        .any(|p| fs::metadata(directory.join(p)).is_ok_and(|m| m.len() > MAX_TRACE));
    Ok((status, timed_out, output_limit))
}

/// Pythonの読み込み先。ホスト上の場所と、bubblewrapの中の場所を同じ順で持つ。
struct PythonPaths {
    deps: PathBuf,
    meta: PathBuf,
    host: Vec<PathBuf>,
    sandbox: Vec<PathBuf>,
}
struct PreparedPython {
    trace: crate::python_env::PythonTrace,
    paths: PythonPaths,
    programs: std::collections::BTreeMap<&'static str, Program>,
}
enum Start<'a> {
    File(&'a str),
    Program(&'a Program),
}
fn start_for<'a>(
    launch: Launch,
    entrypoint: &'a str,
    python: Option<&'a PreparedPython>,
    role: &str,
) -> io::Result<Start<'a>> {
    if launch.is_file() {
        return Ok(Start::File(entrypoint));
    }
    python
        .and_then(|p| p.programs.get(role))
        .map(Start::Program)
        .ok_or_else(|| invalid("起動のしかたを固定版の中で解決できません"))
}
fn package(
    role: &str,
    repository: &str,
    commit: &str,
    tree: &str,
    root: Option<&str>,
    files: usize,
) -> crate::python_env::PackageRecord {
    crate::python_env::PackageRecord {
        role: role.into(),
        repository: repository.into(),
        commit: commit.into(),
        tree: tree.into(),
        root: root.map(String::from),
        files,
        pyproject_blob: None,
        project: None,
        metadata: None,
        note: String::new(),
    }
}
/// 固定コミットの pyproject.toml を、作業ツリーを展開せずに読み、パッケージ情報を作る。
fn read_package(
    git_root: &Path,
    mut record: crate::python_env::PackageRecord,
    meta: &Path,
) -> io::Result<crate::python_env::PackageRecord> {
    let listed = git_text(
        git_root,
        &["ls-tree", &record.commit, "--", "pyproject.toml"],
    )?;
    let blob = listed
        .split_once('\t')
        .and_then(|(header, _)| header.split_whitespace().nth(2))
        .map(String::from);
    record.pyproject_blob = blob.clone();
    let Some(blob) = blob else {
        record.note = "pyproject.toml がないため、パッケージ情報を作っていない".into();
        return Ok(record);
    };
    let bytes = git(git_root, &["cat-file", "blob", &blob])?;
    if bytes.len() > 1024 * 1024 {
        record.note = "pyproject.toml が1MiBを超えるため読んでいない".into();
        return Ok(record);
    }
    match crate::python_env::read_project(&String::from_utf8_lossy(&bytes)) {
        Ok(Some(project)) => {
            record.metadata = Some(crate::python_env::write_metadata(meta, &project)?);
            record.note = format!(
                "pyproject.toml の静的な値から、{} {} のパッケージ情報を基盤が作った",
                project.name, project.version
            );
            record.project = Some(project);
        }
        Ok(None) => {
            record.note =
                "pyproject.toml に [project] がないため、パッケージ情報を作っていない".into();
        }
        Err(e) => record.note = format!("パッケージ情報を作っていない: {e}"),
    }
    Ok(record)
}
fn prepare_python(
    root: &Path,
    recipe: &Recipe,
    setup: &crate::python_env::PythonSetup,
    tree: &str,
    files: &[SnapshotFile],
    dependencies: &[FetchedDependency],
    directory: &Path,
) -> io::Result<PreparedPython> {
    let meta = directory.join("python-meta");
    let deps = directory.join("deps");
    fs::create_dir(&meta)?;
    fs::create_dir(&deps)?;
    let target = package(
        "target",
        &recipe.source.repository,
        &recipe.source.commit,
        tree,
        setup.root.as_deref(),
        files.len(),
    );
    let mut packages = vec![read_package(root, target, &meta)?];
    let mut host = vec![match &setup.root {
        Some(r) => directory.join("source").join(r),
        None => directory.join("source"),
    }];
    let mut sandbox = vec![match &setup.root {
        Some(r) => Path::new("/source").join(r),
        None => PathBuf::from("/source"),
    }];
    let mut import_path = vec![match &setup.root {
        Some(r) => format!("source/{r}"),
        None => "source".into(),
    }];
    for (i, (dependency, dependency_root, dependency_tree, dependency_files)) in
        dependencies.iter().enumerate()
    {
        let copy = deps.join(i.to_string());
        copy_snapshot(dependency_root, &copy, dependency_files)?;
        let record = package(
            "dependency",
            &dependency.repository,
            &dependency.commit,
            dependency_tree,
            dependency.root.as_deref(),
            dependency_files.len(),
        );
        packages.push(read_package(dependency_root, record, &meta)?);
        let relative = dependency.root.clone().unwrap_or_default();
        host.push(copy.join(&relative));
        sandbox.push(Path::new("/deps").join(i.to_string()).join(&relative));
        import_path.push(format!("deps/{i}/{relative}").trim_end_matches('/').into());
    }
    host.push(meta.clone());
    sandbox.push("/meta".into());
    import_path.push("python-meta".into());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for entry in fs::read_dir(&meta)? {
            fs::set_permissions(entry?.path(), fs::Permissions::from_mode(0o555))?;
        }
        fs::set_permissions(&meta, fs::Permissions::from_mode(0o555))?;
    }
    let sets: Vec<BTreeSet<String>> = std::iter::once(files)
        .chain(dependencies.iter().map(|d| d.3.as_slice()))
        .map(|f| f.iter().map(|f| f.path.clone()).collect())
        .collect();
    let roots: Vec<ImportRoot> = packages
        .iter()
        .zip(&sets)
        .map(|(p, files)| ImportRoot {
            repository: &p.repository,
            commit: &p.commit,
            root: p.root.as_deref(),
            files,
            project: p.project.as_ref(),
        })
        .collect();
    let readback = recipe.trial.as_ref().and_then(|t| t.readback.as_ref());
    let starts = [
        Some(("operate", recipe.launch, recipe.entrypoint.as_str())),
        recipe
            .server
            .as_ref()
            .map(|s| ("wait", s.launch, s.entrypoint.as_str())),
        readback.map(|r| ("readback", r.launch, r.entrypoint.as_str())),
    ];
    let mut programs = std::collections::BTreeMap::new();
    let mut launches = vec![];
    for (role, launch, name) in starts.into_iter().flatten() {
        if launch.is_file() {
            continue;
        }
        let (program, record) = crate::python_env::resolve(role, launch, name, &roots)?;
        programs.insert(role, program);
        launches.push(record);
    }
    Ok(PreparedPython {
        trace: crate::python_env::PythonTrace {
            flags: vec!["-S".into(), "-P".into()],
            import_path,
            packages,
            launches,
        },
        paths: PythonPaths {
            deps,
            meta,
            host,
            sandbox,
        },
        programs,
    })
}

/// 待つプロセスを起動し、準備ができるまで待つ。故障として起動しない場合は、ポートが応答しないことを確かめる。
#[allow(clippy::too_many_arguments)]
fn start_server(
    launch: &Launcher,
    spec: &crate::server::ServerSpec,
    args: &[String],
    port: u16,
    python: Option<&PreparedPython>,
    recipe: &Recipe,
    fault: Option<&mut (String, bool)>,
    directory: &Path,
    recorder: &Recorder,
) -> io::Result<(crate::server::ServerPhase, Option<ChildGuard>)> {
    let start = start_for(spec.launch, &spec.entrypoint, python, "wait")?;
    let mut phase = crate::server::ServerPhase {
        command: launch.describe(&start, args),
        port,
        started: false,
        ready: false,
        ready_after_ms: None,
        exited_before_stop: false,
        exit_code: None,
        stopped_by_platform: false,
        output_limit: false,
        stdout_excerpt: String::new(),
        stderr_excerpt: String::new(),
        detail: String::new(),
    };
    let stdout = File::create(directory.join("server-stdout.txt"))?;
    let stderr = File::create(directory.join("server-stderr.txt"))?;
    let not_started = recipe
        .trial
        .as_ref()
        .is_some_and(|t| t.fault == crate::trial::TrialFault::ServerNotStarted);
    if not_started {
        phase.detail = "故障として、待つプロセスを起動していない".into();
        if crate::server::listening(port)
            && let Some((applied, effective)) = fault
        {
            *effective = false;
            applied.push_str(&format!(
                "。ただし割り当てたポート{port}が応答したため、故障を再現できなかった"
            ));
        }
        return Ok((phase, None));
    }
    let mut command = launch.command(&start, args)?;
    command.stdout(stdout).stderr(stderr);
    let mut child = ChildGuard(command.spawn()?);
    phase.started = true;
    recorder.record(
        Source::Process,
        EventKind::ProcessStarted,
        format!(
            "待つプロセス: pid={}、起動{}（{}）、ポート{port}",
            child.0.id(),
            spec.entrypoint,
            spec.launch.label()
        ),
    )?;
    match crate::server::wait_ready(&mut child.0, port, spec.ready_timeout_ms)? {
        crate::server::Ready::Ready(ms) => {
            phase.ready = true;
            phase.ready_after_ms = Some(ms);
            phase.detail = format!(
                "起動から{ms}ミリ秒後に、ポート{port}が接続を受け付けた（確かめるため基盤が接続した）"
            );
        }
        crate::server::Ready::Exited(status) => {
            phase.detail = format!("接続を受け付ける前に終了した（{status}）");
        }
        crate::server::Ready::TimedOut => {
            phase.detail = format!(
                "{}ミリ秒以内に、ポート{port}が接続を受け付けなかった",
                spec.ready_timeout_ms
            );
        }
    }
    recorder.record(
        Source::Process,
        EventKind::ProcessStarted,
        format!("待つプロセス: {}", phase.detail),
    )?;
    Ok((phase, Some(child)))
}

/// 操作の後に待つプロセスを止め、出力を記録へ移す。
fn finish_server(
    mut phase: crate::server::ServerPhase,
    child: Option<ChildGuard>,
    directory: &Path,
    recorder: &Recorder,
) -> io::Result<crate::server::ServerPhase> {
    if let Some(mut child) = child {
        let (exited, code) = crate::server::stop(&mut child.0)?;
        phase.exited_before_stop = exited;
        phase.stopped_by_platform = !exited;
        phase.exit_code = code;
        recorder.record(
            Source::Process,
            EventKind::ProcessExited,
            if exited {
                format!("待つプロセス: 基盤が止める前に終了していた（終了コード{code:?}）")
            } else {
                "待つプロセス: 操作の後に基盤が止めた（SIGKILL）".into()
            },
        )?;
    }
    phase.output_limit = ["server-stdout.txt", "server-stderr.txt"]
        .iter()
        .any(|p| fs::metadata(directory.join(p)).is_ok_and(|m| m.len() > MAX_TRACE));
    phase.stdout_excerpt = excerpt(&directory.join("server-stdout.txt"), 8192)?;
    phase.stderr_excerpt = excerpt(&directory.join("server-stderr.txt"), 8192)?;
    Ok(phase)
}

struct Launcher<'a> {
    mode: ExecutionMode,
    program: &'a Path,
    snapshot: &'a Path,
    work: &'a Path,
    correlation: &'a Correlation,
    python: Option<&'a PythonPaths>,
}
impl Launcher<'_> {
    /// 実行環境に渡す、起動するものの部分（`-S -P` と、ファイル・`-m`・`-c`）。
    fn start_args(&self, start: &Start, source: &Path) -> Vec<std::ffi::OsString> {
        let mut out: Vec<std::ffi::OsString> = vec![];
        if self.python.is_some() {
            out.extend(["-S".into(), "-P".into()]);
        }
        match start {
            Start::File(entrypoint) => out.push(source.join(entrypoint).into()),
            Start::Program(program) => out.extend(program.args().into_iter().map(Into::into)),
        }
        out
    }
    /// 記録用の、ホスト上で見たコマンド。
    fn describe(&self, start: &Start, args: &[String]) -> Vec<String> {
        std::iter::once(self.program.to_string_lossy().into_owned())
            .chain(
                self.start_args(start, self.snapshot)
                    .iter()
                    .map(|a| a.to_string_lossy().into_owned()),
            )
            .chain(args.iter().cloned())
            .collect()
    }
    /// 試行用の環境変数だけを渡して起動する。標準入出力の接続は呼び出し側で行う。
    fn command(&self, start: &Start, args: &[String]) -> io::Result<Command> {
        let mut command;
        let (observation_path, input_path, work_path);
        if self.mode == ExecutionMode::Bubblewrap {
            if !cfg!(target_os = "linux") {
                return Err(invalid(
                    "bubblewrapはLinuxだけに対応します。ローカルへ自動で切り替えません",
                ));
            }
            command = Command::new(executable("bwrap")?);
            command.args([
                "--unshare-all",
                "--new-session",
                "--die-with-parent",
                "--cap-drop",
                "ALL",
            ]);
            for system in ["/usr", "/lib", "/lib64"] {
                if Path::new(system).exists() {
                    command.args(["--ro-bind", system, system]);
                }
            }
            let runtime_root = self
                .program
                .parent()
                .and_then(Path::parent)
                .ok_or_else(|| invalid("ランタイムの場所を確認してください"))?;
            if !self.program.starts_with("/usr") {
                command.arg("--ro-bind").arg(runtime_root).arg(runtime_root);
            }
            command
                .args([
                    "--proc",
                    "/proc",
                    "--dev",
                    "/dev",
                    "--tmpfs",
                    "/tmp",
                    "--ro-bind",
                ])
                .arg(self.snapshot)
                .arg("/source")
                .arg("--bind")
                .arg(self.work)
                .arg("/work");
            if let Some(python) = self.python {
                command
                    .arg("--ro-bind")
                    .arg(&python.deps)
                    .arg("/deps")
                    .arg("--ro-bind")
                    .arg(&python.meta)
                    .arg("/meta");
            }
            command
                .args(["--chdir", "/work", "--"])
                .arg(self.program)
                .args(self.start_args(start, Path::new("/source")));
            observation_path = PathBuf::from("/work/observations.jsonl");
            input_path = PathBuf::from("/work/input.json");
            work_path = PathBuf::from("/work");
            // 固定版は読み取り専用。workだけを共有し、外部ネットワークを分離する。
        } else {
            command = Command::new(self.program);
            command.args(self.start_args(start, self.snapshot));
            observation_path = self.work.join("observations.jsonl");
            input_path = self.work.join("input.json");
            work_path = self.work.to_path_buf();
        }
        let bubblewrap = self.mode == ExecutionMode::Bubblewrap;
        command
            .args(args)
            .current_dir(self.work)
            .env_clear()
            .env(
                "PATH",
                self.program.parent().unwrap_or(Path::new("/usr/bin")),
            )
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env("HOME", &work_path)
            .env(
                "TMPDIR",
                if bubblewrap {
                    Path::new("/tmp")
                } else {
                    self.work
                },
            )
            .env("EEL_RUN_ID", &self.correlation.run_id)
            .env("EEL_RECORD_ID", &self.correlation.record_id)
            .env("EEL_ATTEMPT_ID", &self.correlation.attempt_id)
            .env("EEL_OBSERVATIONS", &observation_path)
            .env("EEL_INPUT", &input_path)
            .env("EEL_WORK_DIR", &work_path)
            .env(
                "EEL_SOURCE_ROOT",
                if bubblewrap {
                    Path::new("/source")
                } else {
                    self.snapshot
                },
            )
            .stdin(Stdio::null());
        if let Some(python) = self.python {
            let paths = if bubblewrap {
                &python.sandbox
            } else {
                &python.host
            };
            command.env(
                "PYTHONPATH",
                std::env::join_paths(paths).map_err(|e| invalid(e.to_string()))?,
            );
        }
        Ok(command)
    }
}

pub fn assess_run(
    directory: &Path,
    manifest: &Manifest,
    events: &[Event],
) -> io::Result<Assessment> {
    if manifest.mode == "pinned_execution" {
        let trace: ExecutionTrace = read_json(&directory.join("execution.json"))?;
        Ok(assess_trace(manifest, &trace, &trace.recipe.criterion))
    } else {
        Ok(Assessment {
            condition_id: manifest.condition.id.clone(),
            condition_version: manifest.condition.version,
            verdict: Verdict::Unknown,
            evidence: events
                .iter()
                .filter(|e| e.correlation == manifest.correlation)
                .map(|e| format!("events.jsonl#{}", e.sequence))
                .collect(),
            reason: "この実行方式に対応する判定規則はありません".into(),
            unconfirmed: vec!["プロセスの終了だけでは業務条件の成立は確認できない".into()],
            next_action: "現在の固定版実行または報告観測へ対応づけて再検証する".into(),
        })
    }
}

/// 管理用ディレクトリ `base/owner/repo` へ固定コミットを追加で取得する。
/// checkoutや対象コードの起動は行わず、既存の取得済みコミットも消さない。
pub fn fetch_commit(base: &Path, repository: &str, commit: &str) -> io::Result<PathBuf> {
    crate::workspace::validate_source(&CodeReference {
        repository: repository.into(),
        commit: commit.into(),
        path: "README.md".into(),
        line: 1,
        end_line: 1,
    })?;
    let destination = base.join(repository);
    let origin = format!("https://github.com/{repository}.git");
    if !destination.exists() {
        fs::create_dir_all(&destination)?;
        git(&destination, &["init", "-q", "--bare"])?;
        git(&destination, &["remote", "add", "origin", &origin])?;
    } else if !expected_origin(
        repository,
        &git_text(&destination, &["remote", "get-url", "origin"])?,
    ) {
        return Err(invalid("管理用ディレクトリのoriginが一致しません"));
    }
    let present = git_text(
        &destination,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{commit}^{{commit}}"),
        ],
    )
    .is_ok_and(|found| found == commit);
    if !present {
        git(
            &destination,
            &[
                "-c",
                "credential.helper=",
                "-c",
                "credential.interactive=false",
                "fetch",
                "--depth=1",
                "--no-tags",
                "origin",
                commit,
            ],
        )?;
        if git_text(
            &destination,
            &["rev-parse", "--verify", &format!("{commit}^{{commit}}")],
        )? != commit
        {
            return Err(invalid("取得したコミットが指定と一致しません"));
        }
        // 取得したコミットをgcで消さないよう参照を残す。
        git(
            &destination,
            &["update-ref", &format!("refs/eel/{commit}"), commit],
        )?;
    }
    fs::canonicalize(destination)
}

/// 公開GitHubの固定コミットを取得する。checkoutや対象コードの起動は行わない。
pub fn prepare_repository(destination: &Path, recipe: &Recipe) -> io::Result<()> {
    validate_recipe(recipe)?;
    if let Some(parent) = destination.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir(destination)?;
    git(destination, &["init", "-q"])?;
    let origin = format!("https://github.com/{}.git", recipe.source.repository);
    git(destination, &["remote", "add", "origin", &origin])?;
    git(
        destination,
        &[
            "-c",
            "credential.helper=",
            "-c",
            "credential.interactive=false",
            "fetch",
            "--depth=1",
            "--no-tags",
            "origin",
            &recipe.source.commit,
        ],
    )?;
    if git_text(
        destination,
        &[
            "rev-parse",
            "--verify",
            &format!("{}^{{commit}}", recipe.source.commit),
        ],
    )? != recipe.source.commit
    {
        return Err(invalid("取得したコミットが試験設定と一致しません"));
    }
    git(
        destination,
        &["update-ref", "refs/heads/prepared", &recipe.source.commit],
    )?;
    git(
        destination,
        &["symbolic-ref", "HEAD", "refs/heads/prepared"],
    )?;
    write_json_new(
        &destination.join("prepared.json"),
        &serde_json::json!({"schema_version":1,"repository":recipe.source.repository,"commit":recipe.source.commit,"checkout":false,"executed":false}),
    )
}
