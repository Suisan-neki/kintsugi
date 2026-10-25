//! Pythonのパッケージとして対象を試すための準備。
//! 依存先は公開GitHubの固定コミットから、読み込み先のディレクトリだけを取り出して使う。
//! インストール用のスクリプト・ビルド処理は実行しない。パッケージ情報（dist-info）は、
//! pyproject.toml に静的に書かれた名前・版・コマンド・entry points だけから基盤が作る。
use crate::error::invalid;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::Path,
};

pub const MAX_DEPENDENCIES: usize = 8;
/// `-P`（作業ディレクトリを読み込み先にしない）が使える最初の版。
const SAFE_PATH_MINOR: u32 = 11;

/// 起動のしかた。`file` は固定版の中のファイル、`module` は `python -m`、
/// `script` は pyproject.toml の `[project.scripts]` に宣言されたコマンド。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Launch {
    #[default]
    File,
    Module,
    Script,
}
impl Launch {
    pub fn is_file(&self) -> bool {
        *self == Self::File
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::File => "ファイル",
            Self::Module => "モジュール（python -m）",
            Self::Script => "宣言されたコマンド（[project.scripts]）",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dependency {
    pub repository: String,
    pub commit: String,
    /// Pythonの読み込み先にするディレクトリ。省略時はリポジトリの最上位。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PythonSetup {
    /// 対象リポジトリの中で読み込み先にするディレクトリ。省略時は最上位。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
}

/// pyproject.toml の `[project]` から読み取った静的な値。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub version: String,
    pub requires_python: Option<String>,
    pub dependencies: Vec<String>,
    pub scripts: BTreeMap<String, String>,
    pub entry_points: BTreeMap<String, BTreeMap<String, String>>,
}

/// 一つの読み込み先（対象または依存先）の記録。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageRecord {
    /// target か dependency。
    pub role: String,
    pub repository: String,
    pub commit: String,
    pub tree: String,
    pub root: Option<String>,
    pub files: usize,
    /// 基盤が読んだ pyproject.toml の blob。ない場合は None。
    pub pyproject_blob: Option<String>,
    pub project: Option<Project>,
    /// 基盤が作ったパッケージ情報のディレクトリ名。作っていない場合は None。
    pub metadata: Option<String>,
    /// パッケージ情報を作った・作らなかった理由。
    pub note: String,
}

/// 起動のしかたを、固定版の中のどのファイルに解決したかの記録。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedLaunch {
    /// operate（操作するプロセス）・wait（待つプロセス）・readback（読み出し）。
    pub role: String,
    pub launch: Launch,
    pub name: String,
    pub repository: String,
    pub commit: String,
    pub path: String,
    /// script の場合の `モジュール:関数`。
    pub target: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PythonTrace {
    pub flags: Vec<String>,
    pub import_path: Vec<String>,
    pub packages: Vec<PackageRecord>,
    pub launches: Vec<ResolvedLaunch>,
}

pub fn validate(
    setup: &PythonSetup,
    target_repository: &str,
    relative: impl Fn(&str) -> io::Result<()>,
) -> io::Result<()> {
    if let Some(root) = &setup.root {
        relative(root)?;
    }
    if setup.dependencies.len() > MAX_DEPENDENCIES {
        return Err(invalid(format!(
            "依存先は{MAX_DEPENDENCIES}件以内で指定してください"
        )));
    }
    let mut seen = BTreeSet::new();
    for dependency in &setup.dependencies {
        crate::workspace::validate_source(&crate::workspace::CodeReference {
            repository: dependency.repository.clone(),
            commit: dependency.commit.clone(),
            path: "README.md".into(),
            line: 1,
            end_line: 1,
        })?;
        if dependency.repository == target_repository || !seen.insert(&dependency.repository) {
            return Err(invalid(
                "依存先は対象と異なるリポジトリを、重複せずに指定してください",
            ));
        }
        if let Some(root) = &dependency.root {
            relative(root)?;
        }
    }
    Ok(())
}

fn identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}
pub fn module_name(value: &str) -> bool {
    value.len() <= 200 && value.split('.').count() <= 16 && value.split('.').all(identifier)
}
pub fn script_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.starts_with(|c: char| c.is_ascii_alphanumeric())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

/// 起動のしかたと名前の組み合わせを、取得の前に確かめる。
pub fn validate_launch(
    launch: Launch,
    entrypoint: &str,
    python: bool,
    relative: impl Fn(&str) -> io::Result<()>,
) -> io::Result<()> {
    match launch {
        Launch::File => relative(entrypoint),
        _ if !python => Err(invalid(
            "モジュール・宣言されたコマンドで起動するには、Pythonの読み込み先の設定が必要です",
        )),
        Launch::Module if module_name(entrypoint) => Ok(()),
        Launch::Script if script_name(entrypoint) => Ok(()),
        Launch::Module => Err(invalid(
            "モジュール名は英数字・下線をドットでつないだ名前です",
        )),
        Launch::Script => Err(invalid(
            "コマンド名は英数字・ドット・ハイフン・下線の64文字以内です",
        )),
    }
}

fn safe_text(value: &str, limit: usize) -> bool {
    !value.is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
}
fn string_table(value: &toml::Value, what: &str) -> io::Result<BTreeMap<String, String>> {
    let table = value
        .as_table()
        .ok_or_else(|| invalid(format!("{what} は表である必要があります")))?;
    let mut out = BTreeMap::new();
    for (key, value) in table {
        let value = value
            .as_str()
            .ok_or_else(|| invalid(format!("{what} の値は文字列である必要があります")))?;
        if !script_name(key) || !safe_text(value, 200) {
            return Err(invalid(format!("{what} の名前・値を確認できません: {key}")));
        }
        out.insert(key.clone(), value.into());
    }
    Ok(out)
}

/// pyproject.toml を読む。`[project]` がない場合は None。
/// 名前・版などが `dynamic`（ビルド時に決まる値）の場合は、ビルドを実行しないため扱わない。
pub fn read_project(text: &str) -> io::Result<Option<Project>> {
    let table: toml::Table = text
        .parse()
        .map_err(|e| invalid(format!("pyproject.toml を読めません: {e}")))?;
    let Some(project) = table.get("project") else {
        return Ok(None);
    };
    let project = project
        .as_table()
        .ok_or_else(|| invalid("pyproject.toml の [project] が表ではありません"))?;
    let dynamic: Vec<&str> = project
        .get("dynamic")
        .and_then(toml::Value::as_array)
        .map(|a| a.iter().filter_map(toml::Value::as_str).collect())
        .unwrap_or_default();
    if let Some(field) = ["name", "version", "scripts", "entry-points"]
        .into_iter()
        .find(|f| dynamic.contains(f))
    {
        return Err(invalid(format!(
            "pyproject.toml の {field} がビルド時に決まる値（dynamic）のため、ビルドを実行しないこの方式では扱えません"
        )));
    }
    let text_field = |key: &str| -> io::Result<Option<String>> {
        match project.get(key) {
            None => Ok(None),
            Some(value) => value
                .as_str()
                .filter(|v| safe_text(v, 200))
                .map(|v| Some(v.to_owned()))
                .ok_or_else(|| invalid(format!("pyproject.toml の {key} を確認できません"))),
        }
    };
    let name = text_field("name")?
        .filter(|n| script_name(n))
        .ok_or_else(|| invalid("pyproject.toml に静的な名前（name）がありません"))?;
    let version = text_field("version")?
        .filter(|v| {
            v.len() <= 64
                && v.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._+!-".contains(&b))
        })
        .ok_or_else(|| invalid("pyproject.toml に静的な版（version）がありません"))?;
    let dependencies = match project.get("dependencies") {
        None => vec![],
        Some(value) => value
            .as_array()
            .and_then(|a| {
                a.iter()
                    .map(|v| v.as_str().filter(|s| safe_text(s, 200)).map(String::from))
                    .collect::<Option<Vec<_>>>()
            })
            .ok_or_else(|| invalid("pyproject.toml の dependencies を確認できません"))?,
    };
    let scripts = match project.get("scripts") {
        None => BTreeMap::new(),
        Some(value) => string_table(value, "[project.scripts]")?,
    };
    let mut entry_points = BTreeMap::new();
    if let Some(groups) = project.get("entry-points") {
        let groups = groups
            .as_table()
            .ok_or_else(|| invalid("[project.entry-points] は表である必要があります"))?;
        for (group, value) in groups {
            if !script_name(group) {
                return Err(invalid(format!(
                    "entry points の名前を確認できません: {group}"
                )));
            }
            entry_points.insert(
                group.clone(),
                string_table(value, "[project.entry-points]")?,
            );
        }
    }
    Ok(Some(Project {
        name,
        version,
        requires_python: text_field("requires-python")?,
        dependencies,
        scripts,
        entry_points,
    }))
}

/// importlib.metadata が探す名前（小文字、`-`・`_`・`.` の連続を `_` に）。
fn normalize(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if matches!(c, '-' | '_' | '.') {
            if !out.ends_with('_') {
                out.push('_');
            }
        } else {
            out.push(c.to_ascii_lowercase());
        }
    }
    out
}

/// 最小限のパッケージ情報（METADATA・entry_points.txt）を書く。作ったディレクトリ名を返す。
pub fn write_metadata(meta: &Path, project: &Project) -> io::Result<String> {
    let name = format!("{}-{}.dist-info", normalize(&project.name), project.version);
    let directory = meta.join(&name);
    fs::create_dir(&directory).map_err(|e| {
        if e.kind() == io::ErrorKind::AlreadyExists {
            invalid(format!(
                "同じ名前のパッケージ（{}）が複数あります",
                project.name
            ))
        } else {
            e
        }
    })?;
    let mut metadata = format!(
        "Metadata-Version: 2.1\nName: {}\nVersion: {}\n",
        project.name, project.version
    );
    if let Some(requires) = &project.requires_python {
        metadata.push_str(&format!("Requires-Python: {requires}\n"));
    }
    for dependency in &project.dependencies {
        metadata.push_str(&format!("Requires-Dist: {dependency}\n"));
    }
    let mut files = vec![
        ("METADATA", metadata),
        ("INSTALLER", "mugiurushi\n".to_owned()),
    ];
    let mut groups = vec![];
    if !project.scripts.is_empty() {
        groups.push(("console_scripts", &project.scripts));
    }
    groups.extend(project.entry_points.iter().map(|(g, e)| (g.as_str(), e)));
    if !groups.is_empty() {
        let mut text = String::new();
        for (group, entries) in groups {
            text.push_str(&format!("[{group}]\n"));
            for (key, value) in entries {
                text.push_str(&format!("{key} = {value}\n"));
            }
        }
        files.push(("entry_points.txt", text));
    }
    for (file, text) in files {
        let path = directory.join(file);
        fs::write(&path, text)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o444))?;
        }
    }
    Ok(name)
}

/// 起動のしかたを解決するための、一つの読み込み先。
pub struct ImportRoot<'a> {
    pub repository: &'a str,
    pub commit: &'a str,
    pub root: Option<&'a str>,
    pub files: &'a BTreeSet<String>,
    pub project: Option<&'a Project>,
}
impl ImportRoot<'_> {
    fn path(&self, relative: &str) -> String {
        match self.root {
            Some(root) => format!("{root}/{relative}"),
            None => relative.into(),
        }
    }
    fn find(&self, candidates: &[String]) -> Option<String> {
        candidates
            .iter()
            .map(|c| self.path(c))
            .find(|p| self.files.contains(p))
    }
}

/// 起動するもの。ファイルはこのモジュールの外で扱う。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Program {
    Module(String),
    Script {
        name: String,
        module: String,
        function: String,
    },
}
impl Program {
    /// Pythonへ渡す、`-S -P` の後の引数。
    pub fn args(&self) -> Vec<String> {
        match self {
            Self::Module(name) => vec!["-m".into(), name.clone()],
            // pip が作るコマンドと同じく、宣言された関数を呼び、戻り値を終了コードにする。
            Self::Script {
                name,
                module,
                function,
            } => vec![
                "-c".into(),
                format!(
                    "import sys\nfrom {module} import {function}\nsys.argv[0] = {name:?}\nsys.exit({function}())"
                ),
            ],
        }
    }
}

/// モジュール名・コマンド名を、読み込み先の中のファイルに解決する。最初に見つかった読み込み先を使う。
pub fn resolve(
    role: &str,
    launch: Launch,
    name: &str,
    roots: &[ImportRoot],
) -> io::Result<(Program, ResolvedLaunch)> {
    let module_files = |module: &str, main: bool| -> Vec<String> {
        let base = module.replace('.', "/");
        if main {
            vec![format!("{base}/__main__.py"), format!("{base}.py")]
        } else {
            vec![format!("{base}.py"), format!("{base}/__init__.py")]
        }
    };
    let record = |root: &ImportRoot, path: String, target: Option<String>| ResolvedLaunch {
        role: role.into(),
        launch,
        name: name.into(),
        repository: root.repository.into(),
        commit: root.commit.into(),
        path,
        target,
    };
    match launch {
        Launch::File => Err(invalid("ファイルの起動はここでは解決しません")),
        Launch::Module => {
            if !module_name(name) {
                return Err(invalid("モジュール名を確認してください"));
            }
            let candidates = module_files(name, true);
            roots
                .iter()
                .find_map(|root| root.find(&candidates).map(|p| record(root, p, None)))
                .map(|r| (Program::Module(name.into()), r))
                .ok_or_else(|| {
                    invalid(format!(
                        "モジュール {name} を、対象と依存先の固定コミットに見つけられません"
                    ))
                })
        }
        Launch::Script => {
            let (root, value) = roots
                .iter()
                .find_map(|root| {
                    root.project
                        .and_then(|p| p.scripts.get(name))
                        .map(|v| (root, v))
                })
                .ok_or_else(|| {
                    invalid(format!(
                        "コマンド {name} は、対象と依存先の pyproject.toml の [project.scripts] に宣言されていません"
                    ))
                })?;
            let (module, function) = value
                .split_once(':')
                .map(|(m, f)| (m.trim(), f.trim()))
                .filter(|(m, f)| module_name(m) && identifier(f))
                .ok_or_else(|| {
                    invalid(format!(
                        "コマンド {name} の宣言（{value}）は `モジュール:関数` の形ではありません"
                    ))
                })?;
            let candidates = module_files(module, false);
            let path = roots
                .iter()
                .find_map(|r| r.find(&candidates).map(|p| (r, p)))
                .ok_or_else(|| {
                    invalid(format!(
                        "コマンド {name} が指すモジュール {module} を見つけられません"
                    ))
                })?;
            Ok((
                Program::Script {
                    name: name.into(),
                    module: module.into(),
                    function: function.into(),
                },
                ResolvedLaunch {
                    repository: path.0.repository.into(),
                    commit: path.0.commit.into(),
                    ..record(root, path.1, Some(value.clone()))
                },
            ))
        }
    }
}

/// 実行環境の版の表示（例: `Python 3.14.5`）から、`-P` を使えるかを確かめる。
pub fn supports_safe_path(version: &str) -> bool {
    let mut parts = version
        .trim()
        .strip_prefix("Python ")
        .unwrap_or("")
        .split('.')
        .map(|p| p.parse::<u32>().ok());
    matches!(
        (parts.next(), parts.next()),
        (Some(Some(3)), Some(Some(minor))) if minor >= SAFE_PATH_MINOR
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const PYPROJECT: &str = r#"
[project]
name = "Example.Dicom-Lib"
version = "3.0.2"
requires-python = ">=3.10"
dependencies = ["other >=1, <2"]

[project.scripts]
example = "example.cli.main:main"

[project.entry-points.example_subcommands]
show = "example.cli.show:add_subparser"
"#;

    #[test]
    fn static_project_values_become_minimal_metadata() {
        let project = read_project(PYPROJECT)
            .expect("読める")
            .expect("[project] がある");
        assert_eq!(project.scripts["example"], "example.cli.main:main");
        let meta = std::env::temp_dir().join(format!("eel-python-env-{}", std::process::id()));
        let _ = fs::remove_dir_all(&meta);
        fs::create_dir_all(&meta).expect("試験用ディレクトリ");
        let name = write_metadata(&meta, &project).expect("書ける");
        assert_eq!(name, "example_dicom_lib-3.0.2.dist-info");
        let entry = fs::read_to_string(meta.join(&name).join("entry_points.txt")).expect("読める");
        assert!(entry.contains("[console_scripts]\nexample = example.cli.main:main\n"));
        assert!(entry.contains("[example_subcommands]\nshow = example.cli.show:add_subparser\n"));
        assert!(
            write_metadata(&meta, &project).is_err(),
            "同名は重ねて作らない"
        );
        let _ = fs::remove_dir_all(&meta);
    }

    #[test]
    fn values_decided_at_build_time_are_not_guessed() {
        let dynamic = "[project]\nname = \"x\"\ndynamic = [\"version\"]\n";
        assert!(read_project(dynamic).is_err());
        assert!(
            read_project("[tool.other]\nname = 1\n")
                .expect("読める")
                .is_none()
        );
        assert!(
            read_project("[project]\nname = \"x\"\n").is_err(),
            "版がない"
        );
    }

    #[test]
    fn launches_resolve_to_files_in_the_pinned_trees() {
        let project = read_project(PYPROJECT).expect("読める");
        let target: BTreeSet<String> = ["app/__init__.py", "app/__main__.py"]
            .map(String::from)
            .into();
        let dependency: BTreeSet<String> = ["src/example/__init__.py", "src/example/cli/main.py"]
            .map(String::from)
            .into();
        let roots = [
            ImportRoot {
                repository: "example/app",
                commit: "a",
                root: None,
                files: &target,
                project: None,
            },
            ImportRoot {
                repository: "example/lib",
                commit: "b",
                root: Some("src"),
                files: &dependency,
                project: project.as_ref(),
            },
        ];
        let (program, record) = resolve("operate", Launch::Module, "app", &roots).expect("解決");
        assert_eq!(program.args(), ["-m", "app"]);
        assert_eq!(record.path, "app/__main__.py");
        let (program, record) =
            resolve("readback", Launch::Script, "example", &roots).expect("解決");
        assert_eq!(record.repository, "example/lib");
        assert_eq!(record.path, "src/example/cli/main.py");
        assert!(program.args()[1].contains("from example.cli.main import main"));
        assert!(resolve("operate", Launch::Module, "missing", &roots).is_err());
        assert!(resolve("operate", Launch::Script, "app", &roots).is_err());
        assert!(resolve("operate", Launch::Module, "app; import os", &roots).is_err());
    }

    #[test]
    fn safe_path_needs_python_3_11() {
        assert!(supports_safe_path("Python 3.14.5"));
        assert!(supports_safe_path("Python 3.11.0"));
        assert!(!supports_safe_path("Python 3.10.12"));
        assert!(!supports_safe_path("v24.1.0"));
    }
}
