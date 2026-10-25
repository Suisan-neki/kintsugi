//! 固定版の依存導入・ビルドを試行専用コンテナで行う。業務条件の判定とは別の記録。
use crate::error::invalid;
use crate::{recording::*, workspace::CodeReference};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, Read},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinnedFile {
    pub path: String,
    pub sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Wheel {
    pub name: String,
    pub version: String,
    pub url: String,
    pub sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Setup {
    Pnpm {
        version: String,
        url: String,
        integrity: String,
        executable_url: String,
        executable_integrity: String,
        lockfile: String,
    },
    PythonWheels {
        wheels: Vec<Wheel>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub schema_version: u32,
    pub source: CodeReference,
    pub image: String,
    pub platform: String,
    pub setup: Setup,
    /// 固定コミットの中で、導入前後の一致を確認するファイル。
    pub locks: Vec<PinnedFile>,
    /// 設定JSONと同じディレクトリに置く実行用の補助ファイル。
    pub support: Vec<PinnedFile>,
    pub command: Vec<String>,
    pub command_phase: String,
    pub env: BTreeMap<String, String>,
    pub timeout_ms: u64,
    pub memory_mb: u32,
    pub cpus: u32,
    pub limits: Vec<String>,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    #[default]
    NotObserved,
    Succeeded,
    Failed,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub hash: String,
    pub hash_scope: String,
    #[serde(default)]
    pub lock_integrity: Option<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Report {
    pub preparation: Status,
    pub target: Status,
    pub packages: Vec<Package>,
    pub steps: Vec<serde_json::Value>,
    pub detail: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub schema_version: u32,
    pub recipe: Recipe,
    pub tree: String,
    pub source_archive_sha256: String,
    pub engine: Option<String>,
    pub image_id: Option<String>,
    pub runner_sha256: String,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub output_limit: bool,
    pub container_removed: bool,
    pub report: Report,
    pub diagnostics: Vec<String>,
}

fn relative(s: &str) -> bool {
    !s.is_empty()
        && !s.contains(['\\', ':', ','])
        && !s.chars().any(char::is_control)
        && s.split('/').all(|c| !["", ".", "..", ".git"].contains(&c))
        && Path::new(s)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}
pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn hash_valid(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn validate(recipe: &Recipe) -> io::Result<()> {
    crate::workspace::validate_source(&recipe.source)?;
    if recipe.schema_version != 1
        || crate::container::image_digest(&recipe.image).is_none()
        || recipe.image.starts_with('-')
        || recipe.platform != "linux/amd64"
        || !(1000..=7_200_000).contains(&recipe.timeout_ms)
        || !(256..=16384).contains(&recipe.memory_mb)
        || !(1..=8).contains(&recipe.cpus)
        || !["preparation", "target"].contains(&recipe.command_phase.as_str())
        || recipe.command.is_empty()
        || recipe.command.len() > 64
        || recipe
            .command
            .iter()
            .any(|s| s.is_empty() || s.contains('\0') || s.len() > 16384)
        || recipe.locks.is_empty()
        || recipe.locks.len() > 64
        || recipe.support.len() > 16
        || recipe.limits.is_empty()
    {
        return Err(invalid(
            "準備設定の版、ダイジェスト、Linux amd64、資源上限、コマンド、固定ファイルと限界を確認してください",
        ));
    }
    for files in [&recipe.locks, &recipe.support] {
        let mut names = std::collections::BTreeSet::new();
        for f in files {
            if !relative(&f.path) || !hash_valid(&f.sha256) || !names.insert(&f.path) {
                return Err(invalid(
                    "固定ファイルには重複しない相対パスとSHA-256が必要です",
                ));
            }
        }
    }
    for (key, value) in &recipe.env {
        if key.is_empty()
            || !key
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
            || ["PATH", "PYTHONPATH", "PYTHONHOME", "LD_PRELOAD", "HOME"].contains(&key.as_str())
            || value.contains('\0')
        {
            return Err(invalid("準備設定の環境変数が不正です"));
        }
    }
    match &recipe.setup {
        Setup::Pnpm {
            version,
            url,
            integrity,
            executable_url,
            executable_integrity,
            lockfile,
        } => {
            if version.is_empty()
                || !version.bytes().all(|b| b.is_ascii_digit() || b == b'.')
                || url != &format!("https://registry.npmjs.org/pnpm/-/pnpm-{version}.tgz")
                || !integrity.starts_with("sha512-")
                || integrity.len() != 95
                || executable_url
                    != &format!(
                        "https://registry.npmjs.org/@pnpm/exe.linux-x64/-/exe.linux-x64-{version}.tgz"
                    )
                || !executable_integrity.starts_with("sha512-")
                || executable_integrity.len() != 95
                || !recipe.locks.iter().any(|f| &f.path == lockfile)
            {
                return Err(invalid(
                    "pnpmの固定版、配布URL、SHA-512とロックファイルが必要です",
                ));
            }
        }
        Setup::PythonWheels { wheels } => {
            let mut names = std::collections::BTreeSet::new();
            if wheels.is_empty() || wheels.len() > 128 {
                return Err(invalid("ハッシュ付きwheelを1〜128件指定してください"));
            }
            for w in wheels {
                let url = url::Url::parse(&w.url).map_err(io::Error::other)?;
                if w.name.is_empty()
                    || w.version.is_empty()
                    || !names.insert(w.name.to_lowercase().replace('_', "-"))
                    || !hash_valid(&w.sha256)
                    || url.scheme() != "https"
                    || url.host_str().is_none()
                    || !url.username().is_empty()
                    || url.password().is_some()
                    || !url.path().ends_with(".whl")
                    || url.query().is_some()
                    || url.fragment().is_some()
                {
                    return Err(invalid(
                        "wheelの名前、固定版、HTTPS配布URLとSHA-256を確認してください",
                    ));
                }
            }
        }
    }
    Ok(())
}
fn bounded_file(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() || meta.len() > limit {
        return Err(invalid("記録が通常ファイルではないか、上限を超えています"));
    }
    let mut bytes = Vec::new();
    File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(invalid("記録の上限超過"));
    }
    Ok(bytes)
}
pub fn inspect(path: &Path) -> io::Result<serde_json::Value> {
    let recipe: Recipe =
        serde_json::from_slice(&bounded_file(path, 1024 * 1024)?).map_err(io::Error::other)?;
    validate(&recipe)?;
    let mut scripts = BTreeMap::new();
    for f in &recipe.support {
        let bytes = bounded_file(
            &path.parent().unwrap_or(Path::new(".")).join(&f.path),
            1024 * 1024,
        )?;
        if sha256(&bytes) != f.sha256 {
            return Err(invalid(format!("補助ファイルのハッシュ不一致: {}", f.path)));
        }
        scripts.insert(
            f.path.clone(),
            String::from_utf8(bytes).map_err(io::Error::other)?,
        );
    }
    Ok(
        serde_json::json!({"recipe":recipe,"support_contents":scripts,
        "execution_notice":"コンテナ内でパッケージの導入、対象のインストール用スクリプトと指定コマンドを実行します。ネットワークから配布ファイルを取得します。ホストのディレクトリ、認証情報、Dockerソケットは渡しません。",
        "runner":include_str!("../tools/environment-runner.py")}),
    )
}
fn command(program: &str, args: &[&str]) -> Command {
    let mut c = Command::new(program);
    c.args(args)
        .stdin(Stdio::null())
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default());
    c
}
fn small_output(mut c: Command) -> io::Result<String> {
    let o = c.output()?;
    if !o.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&o.stderr)
                .chars()
                .take(8192)
                .collect::<String>(),
        ));
    }
    Ok(String::from_utf8_lossy(&o.stdout).trim().to_owned())
}
fn git(root: &Path, args: &[&str]) -> io::Result<String> {
    let mut c = command(
        "git",
        &[
            "--no-replace-objects",
            "-c",
            "core.hooksPath=/dev/null",
            "-C",
        ],
    );
    c.arg(root)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1");
    small_output(c)
}
struct Container(String);
impl Drop for Container {
    fn drop(&mut self) {
        let _ = small_output(command("docker", &["rm", "-f", &self.0]));
    }
}
/// 未コミットの作業ファイルは使わず、Gitの固定コミットをアーカイブしてコンテナへ渡す。
pub fn run(recipe_path: &Path, root: &Path, output: &Path) -> io::Result<PathBuf> {
    let plan = inspect(recipe_path)?;
    let recipe: Recipe =
        serde_json::from_value(plan["recipe"].clone()).map_err(io::Error::other)?;
    let root = fs::canonicalize(root)?;
    let origin = git(&root, &["remote", "get-url", "origin"])?;
    if origin.trim_end_matches(".git").trim_end_matches('/')
        != format!("https://github.com/{}", recipe.source.repository)
    {
        return Err(invalid("対象リポジトリのoriginが準備設定と一致しません"));
    }
    let commit = &recipe.source.commit;
    if git(&root, &["rev-parse", &format!("{commit}^{{commit}}")])? != *commit {
        return Err(invalid("固定コミットがありません"));
    }
    let (directory, correlation) = new_run_directory(output)?;
    let directory = fs::canonicalize(directory)?;
    write_json_new(&directory.join("plan.json"), &plan)?;
    let mut record = Record {
        schema_version: 1,
        tree: git(&root, &["rev-parse", &format!("{commit}^{{tree}}")])?,
        source_archive_sha256: String::new(),
        recipe: recipe.clone(),
        engine: None,
        image_id: None,
        runner_sha256: sha256(include_bytes!("../tools/environment-runner.py")),
        exit_code: None,
        timed_out: false,
        output_limit: false,
        container_removed: true,
        report: Report::default(),
        diagnostics: vec![],
    };
    let result = execute(
        &directory,
        &correlation.run_id,
        &root,
        &plan,
        &recipe,
        &mut record,
    );
    if let Err(e) = result {
        record.diagnostics.push(e.to_string());
        if record.report.preparation != Status::Succeeded {
            record.report.preparation = Status::Failed;
        }
    }
    write_json_new(&directory.join("environment.json"), &record)?;
    Ok(directory)
}
fn execute(
    directory: &Path,
    id: &str,
    root: &Path,
    plan: &serde_json::Value,
    recipe: &Recipe,
    record: &mut Record,
) -> io::Result<()> {
    let archive = directory.join("source.tar");
    git(
        root,
        &[
            "archive",
            "--format=tar",
            "-o",
            archive
                .to_str()
                .ok_or_else(|| invalid("パスがUTF-8ではありません"))?,
            &recipe.source.commit,
        ],
    )?;
    if fs::metadata(&archive)?.len() > 512 * 1024 * 1024 {
        return Err(invalid("ソースは512MiBまでです"));
    }
    record.source_archive_sha256 = sha256(&fs::read(&archive)?);
    let inputs = directory.join("inputs");
    fs::create_dir(&inputs)?;
    write_json_new(&inputs.join("recipe.json"), recipe)?;
    fs::write(
        inputs.join("runner.py"),
        include_bytes!("../tools/environment-runner.py"),
    )?;
    for (path, contents) in plan["support_contents"]
        .as_object()
        .ok_or_else(|| invalid("補助ファイルがありません"))?
    {
        let file = inputs.join("support").join(path);
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(
            file,
            contents
                .as_str()
                .ok_or_else(|| invalid("補助ファイルはテキストです"))?,
        )?;
    }
    // Hostの資格情報や環境変数をDockerや対象へ継承しない。
    record.engine = Some(small_output(command(
        "docker",
        &["version", "--format", "{{.Server.Version}}"],
    ))?);
    let mut pull = command(
        "docker",
        &["pull", "--platform", &recipe.platform, &recipe.image],
    );
    let (exit, timed, limited) = wait_logged(&mut pull, directory, "pull", 600_000)?;
    if exit != Some(0) || timed || limited {
        return Err(io::Error::other("固定イメージを取得できません"));
    }
    record.image_id = Some(small_output(command(
        "docker",
        &["image", "inspect", "--format", "{{.Id}}", &recipe.image],
    ))?);
    let name = format!("eel-env-{id}");
    let _guard = Container(name.clone());
    small_output(command(
        "docker",
        &[
            "create",
            "--name",
            &name,
            "--platform",
            &recipe.platform,
            "--label",
            "eel.managed=1",
            "--cap-drop=ALL",
            "--security-opt=no-new-privileges",
            "--pids-limit=512",
            "--memory",
            &format!("{}m", recipe.memory_mb),
            "--cpus",
            &recipe.cpus.to_string(),
            "--workdir",
            "/work",
            "--entrypoint",
            "python3",
            &recipe.image,
            "/eel/runner.py",
        ],
    ))?;
    record.container_removed = false;
    small_output(command(
        "docker",
        &["cp", &inputs.to_string_lossy(), &format!("{name}:/eel")],
    ))?;
    small_output(command(
        "docker",
        &[
            "cp",
            &archive.to_string_lossy(),
            &format!("{name}:/eel/source.tar"),
        ],
    ))?;
    let (exit, timed, limited) = wait_logged(
        &mut command("docker", &["start", "-a", &name]),
        directory,
        "execution",
        recipe.timeout_ms,
    )?;
    record.exit_code = exit;
    record.timed_out = timed;
    record.output_limit = limited;
    // timeout時も先に止め、途中までの状態を読み出す。
    let _ = small_output(command("docker", &["stop", "-t", "2", &name]));
    let results = directory.join("result");
    small_output(command(
        "docker",
        &["cp", &format!("{name}:/result"), &results.to_string_lossy()],
    ))?;
    record.report = serde_json::from_slice(&bounded_file(
        &results.join("report.json"),
        16 * 1024 * 1024,
    )?)
    .map_err(io::Error::other)?;
    if timed || limited || exit != Some(0) {
        if record.report.preparation == Status::Succeeded && recipe.command_phase == "target" {
            record.report.target = Status::Failed;
        } else {
            record.report.preparation = Status::Failed;
        }
    }
    small_output(command("docker", &["rm", "-f", &name]))?;
    record.container_removed = true;
    Ok(())
}
fn wait_logged(
    c: &mut Command,
    directory: &Path,
    prefix: &str,
    timeout_ms: u64,
) -> io::Result<(Option<i32>, bool, bool)> {
    let stdout = directory.join(format!("{prefix}.stdout.log"));
    let stderr = directory.join(format!("{prefix}.stderr.log"));
    let mut child = c
        .stdout(File::create(&stdout)?)
        .stderr(File::create(&stderr)?)
        .spawn()?;
    let start = Instant::now();
    loop {
        let limited =
            fs::metadata(&stdout)?.len() + fs::metadata(&stderr)?.len() > 32 * 1024 * 1024;
        let timed = start.elapsed() >= Duration::from_millis(timeout_ms);
        if limited || timed {
            let _ = child.kill();
            let _ = child.wait();
            return Ok((None, timed, limited));
        }
        if let Some(status) = child.try_wait()? {
            return Ok((status.code(), false, false));
        }
        thread::sleep(Duration::from_millis(100));
    }
}
pub fn load(directory: &Path) -> io::Result<Record> {
    serde_json::from_slice(&bounded_file(
        &directory.join("environment.json"),
        20 * 1024 * 1024,
    )?)
    .map_err(io::Error::other)
}
pub fn compare(before: &Record, after: &Record) -> serde_json::Value {
    let packages = |r: &Record| {
        r.report
            .packages
            .iter()
            .map(|p| {
                (
                    p.name.clone(),
                    p.version.clone(),
                    p.hash_scope.clone(),
                    p.hash.clone(),
                    p.lock_integrity.clone(),
                )
            })
            .collect::<std::collections::BTreeSet<_>>()
    };
    let a = packages(before);
    let b = packages(after);
    let entries = |values: Vec<_>| {
        values.into_iter().map(|p: &(String,String,String,String,Option<String>)| serde_json::json!({"name":p.0,"version":p.1,"hash_scope":p.2,"hash":p.3,"lock_integrity":p.4})).collect::<Vec<_>>()
    };
    serde_json::json!({"source_changed":before.recipe.source!=after.recipe.source,"image_changed":before.recipe.image!=after.recipe.image,
        "recipe_changed":serde_json::to_value(&before.recipe).ok()!=serde_json::to_value(&after.recipe).ok(),
        "runner_changed":before.runner_sha256!=after.runner_sha256,
        "preparation_before":before.report.preparation,"preparation_after":after.report.preparation,
        "target_before":before.report.target,"target_after":after.report.target,
        "packages_comparable":before.report.preparation==Status::Succeeded && after.report.preparation==Status::Succeeded,
        "removed_or_changed":entries(a.difference(&b).collect()),
        "added_or_changed":entries(b.difference(&a).collect()),
        "limit":"準備と指定コマンドの比較です。業務条件やモデルの性能を判定しません。"})
}
