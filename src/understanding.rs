//! 固定コミットの読取記録を基盤側に保存し、概要・質問・試験準備の結果を返す。
//! 結果が示すコード参照は、保存した読取記録のファイルと行に照合する。照合は説明の正しさを保証しない。
use crate::reader::{self, ReaderRequest};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    fs, io,
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum UnderstandRequest {
    /// GitHubのURLと任意のブランチ・タグ・コミットから読み、コミットを固定する。
    Open {
        url: String,
        reference: String,
    },
    Overview {
        repository: String,
        commit: String,
    },
    Architecture {
        repository: String,
        commit: String,
    },
    Inquire {
        repository: String,
        commit: String,
        question: String,
        path: Option<String>,
        line: Option<u32>,
    },
    Function {
        repository: String,
        commit: String,
        path: String,
        line: u32,
    },
    File {
        repository: String,
        commit: String,
        path: String,
    },
    Plan {
        repository: String,
        commit: String,
    },
}

#[derive(Clone)]
pub struct Store {
    base: PathBuf,
    lock: Arc<Mutex<()>>,
    diagram_lock: Arc<Mutex<()>>,
}

fn invalid(text: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, text.into())
}

fn validate(repository: &str, commit: &str) -> io::Result<()> {
    crate::workspace::validate_source(&crate::workspace::CodeReference {
        repository: repository.into(),
        commit: commit.into(),
        path: "README.md".into(),
        line: 1,
        end_line: 1,
    })
}

impl Store {
    pub(crate) fn diagram_guard(&self) -> io::Result<std::sync::MutexGuard<'_, ()>> {
        self.diagram_lock
            .lock()
            .map_err(|_| io::Error::other("図の生成状態を読み取れません"))
    }
    pub fn architecture_source(&self, repository: &str, commit: &str) -> io::Result<Value> {
        let output = reader::read(ReaderRequest::ArchitectureSource {
            snapshot: self.load(repository, commit)?,
        })?;
        self.save(&output["snapshot"])?;
        self.load(repository, commit)
    }
    pub(crate) fn diagram_directory(&self, repository: &str, commit: &str) -> io::Result<PathBuf> {
        validate(repository, commit)?;
        Ok(self
            .base
            .join(".system-diagrams")
            .join(repository)
            .join(commit))
    }
    pub fn new(base: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(&base)?;
        Ok(Self {
            base: fs::canonicalize(base)?,
            lock: Arc::new(Mutex::new(())),
            diagram_lock: Arc::new(Mutex::new(())),
        })
    }
    fn path(&self, repository: &str, commit: &str) -> io::Result<PathBuf> {
        validate(repository, commit)?;
        Ok(self.base.join(repository).join(format!("{commit}.json")))
    }
    pub fn load(&self, repository: &str, commit: &str) -> io::Result<Value> {
        let path = self.path(repository, commit)?;
        if !path.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "{repository} のコミット {commit} はまだ読み取っていません。URLを入力して読み直してください"
                ),
            ));
        }
        if fs::metadata(&path)?.len() > 16 * 1024 * 1024 {
            return Err(invalid("読取記録が16MiBを超えています"));
        }
        crate::recording::read_json(&path)
    }
    /// 公開リポジトリの読取記録の写し。同じコミットの記録は、読んだファイルを足して置き換える。
    fn save(&self, snapshot: &Value) -> io::Result<()> {
        let repository = snapshot["fullName"]
            .as_str()
            .ok_or_else(|| invalid("読取記録のリポジトリ名がありません"))?;
        let commit = snapshot["commit"]
            .as_str()
            .ok_or_else(|| invalid("読取記録のコミットがありません"))?;
        let path = self.path(repository, commit)?;
        let _guard = self
            .lock
            .lock()
            .map_err(|_| io::Error::other("読取記録の状態を読み取れません"))?;
        let mut merged = snapshot.clone();
        if path.is_file()
            && let Ok(previous) = crate::recording::read_json::<Value>(&path)
            && let (Some(old), Some(files)) = (
                previous["files"].as_object(),
                merged["files"].as_object_mut(),
            )
        {
            for (name, text) in old {
                files.entry(name.clone()).or_insert_with(|| text.clone());
            }
            let read: Vec<String> = files.keys().cloned().collect();
            if let Some(omitted) = merged["omitted"].as_array_mut() {
                omitted.retain(|o| {
                    !o["path"]
                        .as_str()
                        .is_some_and(|p| read.iter().any(|r| r == p))
                });
            }
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension(format!("{}.pending", std::process::id()));
        fs::write(
            &temporary,
            serde_json::to_vec(&merged).map_err(io::Error::other)?,
        )?;
        fs::rename(&temporary, &path)
    }

    pub fn handle(&self, request: UnderstandRequest) -> io::Result<Value> {
        let (snapshot, output) = match request {
            UnderstandRequest::Open { url, reference } => {
                // 40桁のコミットを指定し、同じコミットを読んだ記録があれば、GitHubへ問い合わせずに使う。
                let stored = repository_of(&url).and_then(|repository| {
                    is_commit(&reference)
                        .then(|| self.load(&repository, &reference).ok())
                        .flatten()
                });
                let output = match stored {
                    Some(snapshot) => reader::read(ReaderRequest::Overview { snapshot })?,
                    None => reader::read(ReaderRequest::Open { url, reference })?,
                };
                (output["snapshot"].clone(), output)
            }
            UnderstandRequest::Overview { repository, commit } => {
                let snapshot = self.load(&repository, &commit)?;
                let output = reader::read(ReaderRequest::Overview { snapshot })?;
                (output["snapshot"].clone(), output)
            }
            UnderstandRequest::Architecture { repository, commit } => {
                let snapshot = self.load(&repository, &commit)?;
                let output = reader::read(ReaderRequest::Architecture { snapshot })?;
                (output["snapshot"].clone(), output)
            }
            UnderstandRequest::Inquire {
                repository,
                commit,
                question,
                path,
                line,
            } => {
                if question.trim().is_empty() && path.is_none() {
                    return Err(invalid("質問を入力するか、関数を選んでください"));
                }
                if question.len() > 6000 || question.chars().count() > 2000 {
                    return Err(invalid("質問は2000文字・6000バイト以内です"));
                }
                let snapshot = self.load(&repository, &commit)?;
                let output = reader::read(ReaderRequest::Inquire {
                    snapshot,
                    question,
                    path,
                    line,
                })?;
                (output["snapshot"].clone(), output)
            }
            UnderstandRequest::Function {
                repository,
                commit,
                path,
                line,
            } => {
                let snapshot = self.load(&repository, &commit)?;
                let output = reader::read(ReaderRequest::Function {
                    snapshot,
                    path,
                    line,
                })?;
                (output["snapshot"].clone(), output)
            }
            UnderstandRequest::Plan { repository, commit } => {
                let snapshot = self.load(&repository, &commit)?;
                let output = reader::read(ReaderRequest::Plan { snapshot })?;
                (output["snapshot"].clone(), output)
            }
            UnderstandRequest::File {
                repository,
                commit,
                path,
            } => {
                let mut snapshot = self.load(&repository, &commit)?;
                if snapshot["files"][&path].is_null() {
                    let output = reader::read(ReaderRequest::File {
                        snapshot,
                        path: path.clone(),
                    })?;
                    snapshot = output["snapshot"].clone();
                    self.save(&snapshot)?;
                }
                let text = snapshot["files"][&path]
                    .as_str()
                    .ok_or_else(|| invalid("ファイルを読み取れませんでした"))?;
                return Ok(json!({
                    "schema_version": 1,
                    "source": source_summary(&snapshot),
                    "file": {
                        "path": path,
                        "text": text,
                        "lines": text.lines().count(),
                        "url": github_url(&snapshot, &path, 1),
                    },
                }));
            }
        };
        self.save(&snapshot)?;
        let mut result = output["result"].clone();
        let check = verify_refs(&mut result, &snapshot);
        Ok(json!({
            "schema_version": 1,
            "source": source_summary(&snapshot),
            "result": result,
            "references": check,
        }))
    }
}

fn is_commit(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn repository_of(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://github.com/")?;
    let mut parts = rest.trim_end_matches('/').split('/');
    let owner = parts.next()?;
    let repo = parts.next()?.trim_end_matches(".git");
    parts.next().is_none().then(|| format!("{owner}/{repo}"))
}

fn github_url(snapshot: &Value, path: &str, line: u64) -> String {
    let encoded: Vec<String> = path
        .split('/')
        .map(|part| {
            part.bytes()
                .map(|b| {
                    if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                        (b as char).to_string()
                    } else {
                        format!("%{b:02X}")
                    }
                })
                .collect()
        })
        .collect();
    format!(
        "https://github.com/{}/blob/{}/{}#L{line}",
        snapshot["fullName"].as_str().unwrap_or_default(),
        snapshot["commit"].as_str().unwrap_or_default(),
        encoded.join("/")
    )
}

/// 画面の表示用に、読取記録からコード本文を除いた情報を返す。
pub fn source_summary(snapshot: &Value) -> Value {
    let files = snapshot["files"].as_object();
    let read: Vec<&String> = files.map(|f| f.keys().collect()).unwrap_or_default();
    let inventory: Vec<Value> = snapshot["inventory"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|f| {
                    let path = f["path"].as_str().unwrap_or_default();
                    json!({"path": path, "size": f["size"], "read": files.is_some_and(|m| m.contains_key(path))})
                })
                .collect()
        })
        .unwrap_or_default();
    json!({
        "repository": snapshot["fullName"],
        "commit": snapshot["commit"],
        "reference": snapshot["reference"],
        "description": snapshot["description"],
        "scanned_at": snapshot["scannedAt"],
        "truncated": snapshot["truncated"],
        "files_read": read,
        "omitted": snapshot["omitted"],
        "inventory": inventory,
    })
}

/// `refs` の各要素が、保存した読取記録のファイルと行を指すかを照合し、結果を書き込む。
pub fn verify_refs(value: &mut Value, snapshot: &Value) -> Value {
    let mut total = 0;
    let mut unverified = vec![];
    walk(value, snapshot, &mut total, &mut unverified);
    json!({
        "total": total,
        "verified": total - unverified.len(),
        "unverified": unverified,
        "meaning": "参照したファイルと行が、固定コミットの読取記録に存在することを基盤が確かめた。説明の内容が正しいことは保証しない",
    })
}
fn walk(value: &mut Value, snapshot: &Value, total: &mut usize, unverified: &mut Vec<Value>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::Array(refs)) = map.get_mut("refs") {
                for r in refs.iter_mut() {
                    *total += 1;
                    check(r, snapshot);
                    if r["verified"] != true {
                        unverified.push(r.clone());
                    }
                }
            }
            for (key, child) in map.iter_mut() {
                if key != "refs" {
                    walk(child, snapshot, total, unverified);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                walk(item, snapshot, total, unverified);
            }
        }
        _ => {}
    }
}
fn check(r: &mut Value, snapshot: &Value) {
    let path = r["path"].as_str().unwrap_or_default().to_owned();
    let line = r["line"].as_u64().unwrap_or(0);
    let end = r["end_line"].as_u64().unwrap_or(line);
    let Some(text) = snapshot["files"][&path].as_str() else {
        r["verified"] = json!(false);
        r["reason"] = json!("このファイルは読取記録にない");
        return;
    };
    let count = text.lines().count() as u64;
    if line == 0 || end < line || end > count.max(1) {
        r["verified"] = json!(false);
        r["reason"] = json!(format!("行範囲がファイルの{count}行を超える"));
        return;
    }
    r["verified"] = json!(true);
    r["excerpt"] = json!(
        text.lines()
            .nth(line as usize - 1)
            .unwrap_or_default()
            .trim()
            .chars()
            .take(200)
            .collect::<String>()
    );
    r["url"] = json!(github_url(snapshot, &path, line));
}

/// AIの説明が示した参照を照合する。存在しない参照も消さずに、照合できなかった印を付ける。
pub fn verify_claims(claims: &mut [Value], snapshot: &Value) -> usize {
    let mut unverified = 0;
    for claim in claims {
        let mut verified = 0;
        if let Some(Value::Array(refs)) = claim.get_mut("refs") {
            for r in refs.iter_mut() {
                check(r, snapshot);
                if r["verified"] == true {
                    verified += 1;
                } else {
                    unverified += 1;
                }
            }
        }
        claim["has_verified_reference"] = json!(verified > 0);
    }
    unverified
}
