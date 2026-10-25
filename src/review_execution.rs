//! 起動時に指定した一つのリポジトリを、localhostの画面から試す。
use crate::{
    execution::{self, ExecutionMode, Recipe},
    recording::unix_ms,
    workspace::{self, Rule},
};
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecuteRequest {
    pub expected_revision: u64,
    pub actor: String,
    pub reason: String,
    pub recipe: Recipe,
}

const APPROVAL_FILE: &str = ".web-execution-approval.json";

fn denied(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

fn approval(
    directory: &std::path::Path,
    request: &ExecuteRequest,
) -> io::Result<serde_json::Value> {
    Ok(serde_json::json!({
        "schema_version": 1,
        "case_directory": std::fs::canonicalize(directory)?,
        "mode": "bubblewrap",
        "request": serde_json::to_value(request).map_err(io::Error::other)?,
    }))
}

/// 端末からだけ呼ぶ。一つの事例版・実行要求を一回だけWebから実行できるよう承認する。
pub fn approve(directory: &std::path::Path, request: &ExecuteRequest) -> io::Result<()> {
    execution::validate_recipe(&request.recipe)?;
    if request.recipe.containers.is_some() || request.recipe.server.is_some() {
        return Err(denied(
            "Web実行の隔離方式ではコンテナと待受プロセスに対応していません",
        ));
    }
    if workspace::load(directory)?.revision != request.expected_revision {
        return Err(denied(
            "事例の版が一致しません。実行要求を作り直してください",
        ));
    }
    crate::recording::write_json_new(
        &directory.join(APPROVAL_FILE),
        &approval(directory, request)?,
    )
}

pub fn revoke(directory: &std::path::Path) -> io::Result<()> {
    std::fs::remove_file(directory.join(APPROVAL_FILE))
}

#[derive(Debug, Clone, Serialize)]
pub struct Job {
    pub id: String,
    pub check_id: String,
    pub status: String,
    pub started_unix_ms: u64,
    pub finished_unix_ms: Option<u64>,
    pub resulting_revision: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ExecutionStatus {
    pub enabled: bool,
    pub repository: Option<String>,
    pub repository_root: Option<PathBuf>,
    pub mode: Option<String>,
    pub job: Option<Job>,
    pub managed: bool,
    pub runtimes: Vec<RuntimeStatus>,
}
#[derive(Debug, Serialize)]
pub struct RuntimeStatus {
    pub name: String,
    pub available: bool,
    pub detail: String,
}
impl ExecutionStatus {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            repository: None,
            repository_root: None,
            mode: None,
            job: None,
            managed: false,
            runtimes: vec![],
        }
    }
}
#[derive(Default)]
struct State {
    sequence: u64,
    job: Option<Job>,
}
/// 実行できるリポジトリの範囲。
#[derive(Clone)]
enum Repositories {
    /// 起動時に指定した一つのリポジトリ。
    Fixed { root: PathBuf, name: String },
    /// 画面の「試験用に取得」で、固定コミットだけを取得したリポジトリ。
    Managed { base: PathBuf },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareRequest {
    pub repository: String,
    pub commit: String,
}

#[derive(Debug, Serialize)]
pub struct Prepared {
    pub repository: String,
    pub commit: String,
    pub checkout: bool,
    pub executed: bool,
    pub note: String,
}

#[derive(Clone)]
pub struct Controller {
    directory: PathBuf,
    repositories: Repositories,
    mode: ExecutionMode,
    state: Arc<Mutex<State>>,
    fetching: Arc<Mutex<()>>,
}
impl Controller {
    pub fn new(
        directory: PathBuf,
        repository_root: PathBuf,
        mode: ExecutionMode,
    ) -> io::Result<Self> {
        workspace::load(&directory)?;
        let root = std::fs::canonicalize(repository_root)?;
        let name = execution::repository_name(&root)?;
        Ok(Self {
            directory: std::fs::canonicalize(directory)?,
            repositories: Repositories::Fixed { root, name },
            mode,
            state: Arc::new(Mutex::new(State::default())),
            fetching: Arc::new(Mutex::new(())),
        })
    }
    /// 画面で選んだ公開リポジトリを、利用者が取得を指示した後に試せるようにする。
    pub fn managed(directory: PathBuf, base: PathBuf, mode: ExecutionMode) -> io::Result<Self> {
        workspace::load(&directory)?;
        std::fs::create_dir_all(&base)?;
        Ok(Self {
            directory: std::fs::canonicalize(directory)?,
            repositories: Repositories::Managed {
                base: std::fs::canonicalize(base)?,
            },
            mode,
            state: Arc::new(Mutex::new(State::default())),
            fetching: Arc::new(Mutex::new(())),
        })
    }
    /// 固定コミットの取得だけを行う。対象のコード・インストールスクリプトは起動しない。
    pub fn prepare(&self, request: PrepareRequest) -> io::Result<Prepared> {
        let Repositories::Managed { base } = &self.repositories else {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "この起動方法では、起動時に指定したリポジトリだけを試せます",
            ));
        };
        let _guard = self
            .fetching
            .lock()
            .map_err(|_| io::Error::other("取得の状態を読み取れません"))?;
        execution::fetch_commit(base, &request.repository, &request.commit).map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("固定コミットをGitHubから取得できませんでした: {e}"),
            )
        })?;
        Ok(Prepared {
            repository: request.repository,
            commit: request.commit,
            checkout: false,
            executed: false,
            note: "git fetchで固定コミットだけを取得した。作業ツリーの展開・対象コードの実行・依存関係のインストールは行っていない".into(),
        })
    }
    fn root_for(&self, repository: &str, commit: &str) -> io::Result<PathBuf> {
        match &self.repositories {
            Repositories::Fixed { root, name } if name == repository => Ok(root.clone()),
            Repositories::Fixed { .. } => Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "起動時に指定したリポジトリだけを実行できます",
            )),
            Repositories::Managed { base } => {
                let root = base.join(repository);
                if !root.join("HEAD").is_file() && !root.join(".git").exists() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!(
                            "{repository} のコミット {commit} は、まだ試験用に取得していません。「試験用に取得する」を先に実行してください"
                        ),
                    ));
                }
                Ok(root)
            }
        }
    }
    /// 利用者が内容を確かめて試験を始めた後に、依存先の固定コミットだけを取得する。
    /// checkout・インストール用のスクリプト・依存先のコードの起動は、ここでは行わない。
    fn fetch_dependencies(&self, recipe: &Recipe) -> io::Result<Option<PathBuf>> {
        let dependencies = recipe
            .python
            .as_ref()
            .map_or(&[][..], |p| p.dependencies.as_slice());
        match &self.repositories {
            _ if dependencies.is_empty() => Ok(None),
            Repositories::Fixed { .. } => Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "この起動方法では依存先を取得できません。アプリ（app）として起動してください",
            )),
            Repositories::Managed { base } => {
                let _guard = self
                    .fetching
                    .lock()
                    .map_err(|_| io::Error::other("取得の状態を読み取れません"))?;
                for dependency in dependencies {
                    execution::fetch_commit(base, &dependency.repository, &dependency.commit)
                        .map_err(|e| {
                            io::Error::new(
                                e.kind(),
                                format!(
                                    "依存先 {} の固定コミットをGitHubから取得できませんでした: {e}",
                                    dependency.repository
                                ),
                            )
                        })?;
                }
                Ok(Some(base.clone()))
            }
        }
    }
    fn lock(&self) -> io::Result<std::sync::MutexGuard<'_, State>> {
        self.state
            .lock()
            .map_err(|_| io::Error::other("実行状態を読み取れません"))
    }
    pub fn status(&self) -> io::Result<ExecutionStatus> {
        let (repository, repository_root, managed) = match &self.repositories {
            Repositories::Fixed { root, name } => (Some(name.clone()), Some(root.clone()), false),
            Repositories::Managed { .. } => (None, None, true),
        };
        let runtimes = [
            ("node", execution::Runtime::Node),
            ("python3", execution::Runtime::Python3),
            ("http", execution::Runtime::Http),
        ]
        .into_iter()
        .map(|(name, runtime)| {
            let (available, detail) = match execution::runtime_available(&runtime) {
                Ok((path, version)) => (true, format!("{version}（{}）", path.display())),
                Err(error) => (false, error),
            };
            RuntimeStatus {
                name: name.into(),
                available,
                detail,
            }
        })
        .collect();
        Ok(ExecutionStatus {
            enabled: self.mode == ExecutionMode::Bubblewrap,
            repository,
            repository_root,
            mode: Some(self.mode.label().into()),
            job: self.lock()?.job.clone(),
            managed,
            runtimes,
        })
    }
    /// HTTPの共通入口。画面から読めない端末承認と隔離方式の両方を必要とする。
    pub fn start_web(&self, request: ExecuteRequest) -> io::Result<Job> {
        self.mode.ensure_web()?;
        let path = self.directory.join(APPROVAL_FILE);
        let approved: serde_json::Value = crate::recording::read_json(&path).map_err(|_| {
            denied("端末での承認が必要です。実行要求を保存し、case approveで承認してください")
        })?;
        if approved != approval(&self.directory, &request)? {
            return Err(denied(
                "承認した実行要求と一致しません。事例の版・レシピ・記入者・理由を確認してください",
            ));
        }
        // 削除に成功した一つの要求だけが実行へ進む。再送・並行要求は承認を再利用できない。
        std::fs::remove_file(path).map_err(|_| denied("承認は使用済みか、取り消されています"))?;
        // 実行開始より前に削除を永続化し、再起動後の承認の復活を防ぐ。
        std::fs::File::open(&self.directory)?.sync_all()?;
        self.start(request)
    }
    /// 信頼された端末側の呼出し。HTTPハンドラーは必ずstart_webを使う。
    pub fn start(&self, request: ExecuteRequest) -> io::Result<Job> {
        self.mode.ensure_allowed()?;
        execution::validate_recipe(&request.recipe)?;
        for value in [&request.actor, &request.reason] {
            if value.trim().is_empty() || value.len() > 16384 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "記入者と理由は空でない16384バイト以内の値が必要です",
                ));
            }
        }
        let repository_root = self.root_for(
            &request.recipe.source.repository,
            &request.recipe.source.commit,
        )?;
        let workspace = workspace::load(&self.directory)?;
        if workspace.revision != request.expected_revision {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "事例が更新されています。最新版を読み直してください",
            ));
        }
        if !workspace.checks.iter().any(|check| {
            check.id == request.recipe.check_id
                && check.rule == Rule::ObservedAssertion
                && check.source.as_ref() == Some(&request.recipe.source)
                && check.observation.as_ref() == Some(&request.recipe.criterion)
        }) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "現在のコード参照と観測規則に対応する試験設定が必要です",
            ));
        }
        let job = {
            let mut state = self.lock()?;
            if state
                .job
                .as_ref()
                .is_some_and(|job| job.status == "running")
            {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "別の試験を実行中です。終了してから再試験してください",
                ));
            }
            state.sequence = state
                .sequence
                .checked_add(1)
                .ok_or_else(|| io::Error::other("試験番号が上限に達しました"))?;
            let job = Job {
                id: format!("job-{}", state.sequence),
                check_id: request.recipe.check_id.clone(),
                status: "running".into(),
                started_unix_ms: unix_ms()?,
                finished_unix_ms: None,
                resulting_revision: None,
                error: None,
            };
            state.job = Some(job.clone());
            job
        };
        let controller = self.clone();
        let spawn = thread::Builder::new()
            .name("review-execution".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let base = controller.fetch_dependencies(&request.recipe)?;
                    workspace::run_external_with_dependencies(
                        &controller.directory,
                        &repository_root,
                        base.as_deref(),
                        request.recipe,
                        controller.mode,
                        request.actor,
                        request.reason,
                        Some(request.expected_revision),
                    )
                }))
                .unwrap_or_else(|_| {
                    Err(io::Error::other(
                        "試験処理が中断しました。試行の記録と最新版を確認してください",
                    ))
                });
                if let Ok(mut state) = controller.lock()
                    && let Some(job) = state.job.as_mut()
                {
                    job.finished_unix_ms = unix_ms().ok();
                    match result {
                        Ok(view) => {
                            job.status = "completed".into();
                            job.resulting_revision = Some(view.workspace.revision);
                        }
                        Err(error) => {
                            job.status = "failed".into();
                            job.error = Some(error.to_string());
                        }
                    }
                }
            });
        if let Err(error) = spawn {
            if let Some(job) = self.lock()?.job.as_mut() {
                job.status = "failed".into();
                job.error = Some(error.to_string());
                job.finished_unix_ms = unix_ms().ok();
            }
            return Err(error);
        }
        Ok(job)
    }
}
