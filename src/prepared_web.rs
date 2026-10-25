//! 準備済みの静的ファイルを、ハッシュで照合して試験用Webサーバーへ渡す。
use crate::error::invalid;
use crate::{environment, recording::*, workspace::CodeReference};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebBuild {
    pub run_id: String,
    pub manifest_sha256: String,
    pub source: CodeReference,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildBasis {
    pub run_id: String,
    pub source: CodeReference,
    pub preparation_image: String,
    pub runner_sha256: String,
    pub manifest_sha256: String,
    pub packages_sha256: String,
    pub files: usize,
    pub bytes: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuiltFile {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}
pub struct Snapshot {
    pub directory: PathBuf,
    pub basis: BuildBasis,
}
impl Drop for Snapshot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn relative(s: &str) -> bool {
    !s.is_empty()
        && !s.contains(['\\', '\0'])
        && !s.starts_with('/')
        && s.split('/').all(|p| !["", ".", ".."].contains(&p))
}
pub fn validate(build: &WebBuild) -> io::Result<()> {
    crate::workspace::validate_source(&build.source)?;
    if !build.run_id.starts_with("run-")
        || build.run_id.len() > 128
        || !build
            .run_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || !hash(&build.manifest_sha256)
    {
        return Err(invalid("ビルドの試行IDとファイル一覧のSHA-256が必要です"));
    }
    Ok(())
}
fn file(root: &Path, path: &str, max: u64) -> io::Result<Vec<u8>> {
    if !relative(path) {
        return Err(invalid("生成物の相対パスが不正です"));
    }
    let mut p = root.to_path_buf();
    for part in path.split('/') {
        p.push(part);
        if fs::symlink_metadata(&p)?.file_type().is_symlink() {
            return Err(invalid("生成物のリンクは使いません"));
        }
    }
    let meta = fs::metadata(&p)?;
    if !meta.is_file() || meta.len() > max {
        return Err(invalid("生成物の種類か大きさが不正です"));
    }
    let mut bytes = Vec::new();
    fs::File::open(p)?.take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(invalid("生成物が上限を超えました"));
    }
    Ok(bytes)
}
/// rootはサーバーの起動時に指定する。レシピからホスト上の任意のパスを指定させない。
pub fn snapshot(root: &Path, build: &WebBuild) -> io::Result<Snapshot> {
    validate(build)?;
    let root = fs::canonicalize(root)?;
    let directory = root.join(&build.run_id);
    if !fs::symlink_metadata(&directory)?.is_dir() {
        return Err(invalid("準備した試行が通常のディレクトリではありません"));
    }
    let record: environment::Record =
        serde_json::from_slice(&file(&directory, "environment.json", 20 * 1024 * 1024)?)
            .map_err(io::Error::other)?;
    environment::validate(&record.recipe)?;
    if record.schema_version != 1
        || record.recipe.source != build.source
        || record.recipe.command_phase != "preparation"
        || record.report.preparation != environment::Status::Succeeded
        || record.exit_code != Some(0)
        || record.timed_out
        || record.output_limit
        || !record.container_removed
        || !record.diagnostics.is_empty()
        || record.report.packages.is_empty()
    {
        return Err(invalid("成功した固定版のビルド記録と対応づけられません"));
    }
    let manifest = file(&directory, "result/build-files.json", 8 * 1024 * 1024)?;
    if environment::sha256(&manifest) != build.manifest_sha256 {
        return Err(invalid("生成物の一覧が指定した版と一致しません"));
    }
    let files: Vec<BuiltFile> = serde_json::from_slice(&manifest).map_err(io::Error::other)?;
    if files.is_empty() || files.len() > 10000 || !files.iter().any(|f| f.path == "index.html") {
        return Err(invalid(
            "Webの生成物にはindex.htmlを含む1〜10000件の一覧が必要です",
        ));
    }
    let packages = file(&directory, "result/packages.json", 16 * 1024 * 1024)?;
    let inventory: Vec<environment::Package> =
        serde_json::from_slice(&packages).map_err(io::Error::other)?;
    if inventory != record.report.packages {
        return Err(invalid("ビルドの導入一覧と実行記録が一致しません"));
    }
    let (temp, _) = new_run_directory(&std::env::temp_dir().join("eel-prepared-web"))?;
    let mut snapshot = Snapshot {
        directory: temp,
        basis: BuildBasis {
            run_id: build.run_id.clone(),
            source: build.source.clone(),
            preparation_image: record.recipe.image,
            runner_sha256: record.runner_sha256,
            manifest_sha256: build.manifest_sha256.clone(),
            packages_sha256: environment::sha256(&packages),
            files: files.len(),
            bytes: 0,
        },
    };
    let mut names = BTreeSet::new();
    for f in files {
        if !relative(&f.path)
            || !hash(&f.sha256)
            || !names.insert(f.path.clone())
            || f.bytes > 64 * 1024 * 1024
        {
            return Err(invalid("生成物のパス・ハッシュ・重複・大きさが不正です"));
        }
        snapshot.basis.bytes = snapshot
            .basis
            .bytes
            .checked_add(f.bytes)
            .ok_or_else(|| invalid("生成物が大きすぎます"))?;
        if snapshot.basis.bytes > 1024 * 1024 * 1024 {
            return Err(invalid("生成物は合計1GiBまでです"));
        }
        let bytes = file(
            &directory,
            &format!("result/dist/{}", f.path),
            64 * 1024 * 1024,
        )?;
        if bytes.len() as u64 != f.bytes || environment::sha256(&bytes) != f.sha256 {
            return Err(invalid("生成物がビルド時の内容と一致しません"));
        }
        let output = snapshot.directory.join(&f.path);
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        // コピー元が後で変わっても、照合したこのバイト列だけをコンテナへ渡す。
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output)?;
        file.write_all(&bytes)?;
    }
    Ok(snapshot)
}
