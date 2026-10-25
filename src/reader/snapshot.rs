//! 公開リポジトリの版・本文・未読範囲。外部から受け取る記録を境界で検査する。
use super::invalid;
use serde::{Deserialize, Serialize};
use serde_json::Map;
use std::io;
pub const MAX_FILE_BYTES: usize = 128 * 1024;
pub const MAX_FILES: usize = 24;
#[derive(Debug, Deserialize, Serialize)]
pub struct Entry {
    pub path: String,
    #[serde(default)]
    pub size: Option<usize>,
    #[serde(default)]
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha: Option<String>,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct Omitted {
    pub path: String,
    pub reason: String,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct Snapshot {
    pub schema: u32,
    #[serde(rename = "fullName")]
    pub full_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub reference: String,
    pub commit: String,
    #[serde(rename = "scannedAt", default, skip_serializing_if = "Option::is_none")]
    pub scanned_at: Option<String>,
    #[serde(default)]
    pub truncated: bool,
    pub inventory: Vec<Entry>,
    pub files: Map<String, serde_json::Value>,
    pub omitted: Vec<Omitted>,
}
pub fn repository(value: &str) -> io::Result<String> {
    let u = url::Url::parse(value.trim())
        .map_err(|_| invalid("https://github.com/所有者/リポジトリ のURLを入力してください。"))?;
    if u.scheme() != "https"
        || u.host_str() != Some("github.com")
        || u.port().is_some()
        || !u.username().is_empty()
        || u.password().is_some()
        || u.query().is_some()
        || u.fragment().is_some()
    {
        return Err(invalid(
            "公開GitHubリポジトリのURLを入力してください。認証情報やクエリは指定できません。",
        ));
    }
    let c=rx!(r"^/([A-Za-z0-9-]+)/([A-Za-z0-9_.-]+)/?$").captures(u.path()).ok_or_else(|| invalid("ファイルやブランチのURLではなくリポジトリのURLを入力してください。ブランチは別の欄に指定できます。"))?;
    let repo = c[2].strip_suffix(".git").unwrap_or(&c[2]);
    if repo.is_empty() || repo == "." || repo == ".." {
        return Err(invalid("リポジトリ名を確認してください。"));
    }
    Ok(format!("{}/{repo}", &c[1]))
}
pub fn valid_path(p: &str) -> bool {
    !p.is_empty()
        && !p.starts_with('/')
        && !p.chars().any(|c| c.is_control() || c == '\\')
        && !p.split('/').any(|x| x.is_empty() || x == "." || x == "..")
}
pub fn sha(s: &str) -> bool {
    s.len() == 40
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
impl Snapshot {
    pub fn validate(self) -> io::Result<Self> {
        if self.schema != 1 || !sha(&self.commit) || self.inventory.len() > 100000 {
            return Err(invalid("この画面から保存した読取記録を選んでください。"));
        }
        if repository(&format!("https://github.com/{}", self.full_name))? != self.full_name {
            return Err(invalid("リポジトリ情報が不正です。"));
        }
        let mut paths = std::collections::HashSet::new();
        for f in &self.inventory {
            if !valid_path(&f.path) || !paths.insert(f.path.as_str()) {
                return Err(invalid("ファイルのパスが不正です。"));
            }
        }
        for (p, text) in &self.files {
            if !paths.contains(p.as_str())
                || !text.as_str().is_some_and(|s| s.len() <= MAX_FILE_BYTES)
            {
                return Err(invalid("読取記録のコードが不正です。"));
            }
        }
        Ok(self)
    }
}
pub fn source(p: &str) -> bool {
    rx!(r"(?i)\.(rs|[cm]?[jt]sx?|py)$").is_match(p)
}
pub fn readme(p: &str) -> bool {
    rx!(r"(?i)(^|/)readme(\.[a-z]+)*$").is_match(p)
}
pub fn manifest(p: &str) -> bool {
    rx!(r"(^|/)(Cargo\.toml|package\.json|pyproject\.toml|requirements[^/]*\.txt|setup\.py|setup\.cfg|Makefile|tox\.ini|pytest\.ini)$").is_match(p)
}
/// 構成の説明と、サービス・接続の定義。認証情報を含みうる.envは読取候補にしない。
pub fn architecture_config(p: &str) -> bool {
    rx!(r"(?i)(^|/)(docker-compose[^/]*|compose[^/]*)\.ya?ml$|(^|/)(Dockerfile[^/]*|architecture[^/]*\.md|system[-_]design\.md)$|\.(tf|tf\.json)$|(^|/)(cdk|infra|infrastructure|k8s|kubernetes)/.*\.(ts|ya?ml|json)$").is_match(p)
}
pub fn architecture_priority(p: &str) -> Option<u8> {
    if test_file(p) || !readable(p) {
        return None;
    }
    if architecture_config(p) { Some(0) }
    else if readme(p) || manifest(p) { Some(1) }
    else if rx!(r"(?i)(^|/)(main|app|server|index|api|db|database|storage|auth|cognito|client|config|settings)\.(rs|py|[cm]?[jt]sx?)$|(^|/)(App|main)\.vue$").is_match(p) { Some(2) }
    else if p.ends_with(".html") { Some(3) }
    else if source(p) { Some(4) }
    else { None }
}
pub fn readable(p: &str) -> bool {
    !rx!(r"(^|/)(node_modules|vendor|target|dist|build|\.git)(/|$)").is_match(p)
        && (source(p)
            // 本文の読取と、対応言語の静的解析は別。PHP等を解析済みにはしない。
            || rx!(r"(?i)\.(php|twig|sql)$").is_match(p)
            || readme(p)
            || manifest(p)
            || architecture_config(p)
            || p.ends_with(".html")
            || rx!(r"^\.github/workflows/[^/]+\.ya?ml$|(^|/)tests?/[^/]+\.t$").is_match(p))
}
pub fn test_file(p: &str) -> bool {
    rx!(r"(^|/)(tests?|__tests__|spec)(/|$)|\.(test|spec)\.[cm]?[jt]sx?$|(^|/)test_[^/]+\.py$|\.t$")
        .is_match(p)
}
pub fn priority(p: &str) -> u8 {
    if readme(p) && !p.contains('/') {
        0
    } else if manifest(p) && !p.contains('/') {
        1
    } else if architecture_config(p) || rx!(
        r"(^|/)(main\.(rs|py|[jt]sx?)|lib\.rs|server\.[cm]?[jt]s|app\.[jt]sx?|index\.[cm]?[jt]sx?)$"
    )
    .is_match(p)
    {
        2
    } else if rx!(r"(^|/)(tests?|__tests__|tools)(/|\.)|\.(test|spec)\.").is_match(p) {
        5
    } else if source(p) {
        3
    } else {
        4
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    #[test]
    fn invalid_addresses_and_forged_snapshot_paths_are_rejected() {
        assert_eq!(
            repository("https://github.com/example/project.git").unwrap(),
            "example/project"
        );
        for u in [
            "http://github.com/a/b",
            "https://evil.example/a/b",
            "https://github.com/a/b/tree/main",
            "https://user:secret@github.com/a/b",
            "https://github.com/a/b?token=x",
            "https://github.com/a/b#x",
        ] {
            assert!(repository(u).is_err(), "{u}");
        }
        let value: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/reader/scanner.json")).unwrap();
        let valid = value["snapshot"].clone();
        for p in [
            "../escape",
            "a/../b",
            "/absolute",
            "a//b",
            "a\\b",
            "a\0b",
            "a/./b",
        ] {
            let mut s = valid.clone();
            s["inventory"][0]["path"] = json!(p);
            assert!(
                serde_json::from_value::<Snapshot>(s)
                    .unwrap()
                    .validate()
                    .is_err(),
                "{p}"
            );
        }
        let mut s = valid.clone();
        let duplicate = s["inventory"][0].clone();
        s["inventory"].as_array_mut().unwrap().push(duplicate);
        assert!(
            serde_json::from_value::<Snapshot>(s)
                .unwrap()
                .validate()
                .is_err()
        );
        let mut s = valid.clone();
        s["files"]["src/main.rs"] = json!("x".repeat(MAX_FILE_BYTES + 1));
        assert!(
            serde_json::from_value::<Snapshot>(s)
                .unwrap()
                .validate()
                .is_err()
        );
        let mut s = valid.clone();
        s["files"]["src/main.rs"] = json!(4);
        assert!(
            serde_json::from_value::<Snapshot>(s)
                .unwrap()
                .validate()
                .is_err()
        );
        let mut s = valid;
        s["commit"] = json!("main");
        assert!(
            serde_json::from_value::<Snapshot>(s)
                .unwrap()
                .validate()
                .is_err()
        );
    }
}
