//! 公開GitHubの読取・解析API。対象コードもNode.jsも起動しない。
use crate::error::invalid;
// 固定パターンは初回だけコンパイルし、JSと同じASCIIの識別子・語境界を使う。
macro_rules! rx {
    ($pattern:literal) => {{
        static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            let pattern = $pattern.replace("(?-u)", "").replace("(?m-u)", "(?m)")
                .replace(r"\w", "[A-Za-z0-9_]").replace(r"\b", r"(?-u:\b)");
            regex::Regex::new(&pattern).expect("固定した解析パターンは有効な正規表現")
        });
        &*RE
    }};
}
mod analysis;
mod budget;
mod comprehension;
mod github;
mod inquiry;
mod overview;
mod plan;
mod repository_map;
mod snapshot;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{fs, io};
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReaderRequest {
    Read {
        url: String,
        reference: String,
    },
    Analyze {
        snapshot: Value,
        path: Option<String>,
        line: Option<u32>,
    },
    File {
        snapshot: Value,
        path: String,
    },
    /// 読取と同時に概要を作る。
    Open {
        url: String,
        reference: String,
    },
    Overview {
        snapshot: Value,
    },
    Architecture {
        snapshot: Value,
    },
    ArchitectureSource {
        snapshot: Value,
    },
    /// 質問・選んだ関数に関係する箇所を集める。必要な未読ファイルを最大8件読む。
    Inquire {
        snapshot: Value,
        question: String,
        path: Option<String>,
        line: Option<u32>,
    },
    Function {
        snapshot: Value,
        path: String,
        line: u32,
    },
    Plan {
        snapshot: Value,
    },
}
/// GitHubのAPIの利用上限を広げるための認証情報。サーバーの起動者が指定したファイルだけから読む。
fn github_token() -> io::Result<Option<String>> {
    let Some(path) = std::env::var_os("EEL_GITHUB_TOKEN_FILE") else {
        return Ok(None);
    };
    let text = fs::read_to_string(&path).map_err(|e| {
        io::Error::new(
            e.kind(),
            "EEL_GITHUB_TOKEN_FILE に指定したファイルを読めません",
        )
    })?;
    parse_token(&text).map(Some)
}
/// GitHubのトークンは長さや形式が変わるため、中身は解釈しない。
/// 要求ヘッダーへ入れるので、空白・改行・制御文字だけを拒否する。
fn parse_token(text: &str) -> io::Result<String> {
    let token = text.trim();
    if token.is_empty() || token.len() > MAX_TOKEN || !token.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "EEL_GITHUB_TOKEN_FILE の内容を確認してください。空白・改行を含まない{MAX_TOKEN}文字以内の1行にしてください"
            ),
        ));
    }
    Ok(token.to_owned())
}
const MAX_TOKEN: usize = 4096;

pub fn read(request: ReaderRequest) -> io::Result<Value> {
    if serde_json::to_vec(&request)
        .map_err(io::Error::other)?
        .len()
        > 8 * 1024 * 1024
    {
        return Err(invalid("読取APIの入力は8MiB以内です"));
    }
    let limit = match &request {
        ReaderRequest::Read { .. } | ReaderRequest::Analyze { .. } | ReaderRequest::File { .. } => {
            8 * 1024 * 1024
        }
        _ => 16 * 1024 * 1024,
    };
    let budget = budget::Budget::new();
    let output = read_with(request, || github::Github::new(budget.until), &budget)?;
    if serde_json::to_vec(&output).map_err(io::Error::other)?.len() > limit {
        return Err(invalid("読取結果の上限に達しました"));
    }
    Ok(output)
}
fn read_with<F: github::Fetch>(
    request: ReaderRequest,
    fetch: impl Fn() -> io::Result<F>,
    budget: &budget::Budget,
) -> io::Result<Value> {
    use serde_json::json;
    let mut snapshot = match &request {
        ReaderRequest::Read { url, reference } | ReaderRequest::Open { url, reference } => {
            github::scan(&fetch()?, url, reference)?
        }
        ReaderRequest::Analyze { snapshot, .. }
        | ReaderRequest::File { snapshot, .. }
        | ReaderRequest::Overview { snapshot }
        | ReaderRequest::Architecture { snapshot }
        | ReaderRequest::ArchitectureSource { snapshot }
        | ReaderRequest::Inquire { snapshot, .. }
        | ReaderRequest::Function { snapshot, .. }
        | ReaderRequest::Plan { snapshot } => {
            serde_json::from_value::<snapshot::Snapshot>(snapshot.clone())
                .map_err(|_| invalid("この画面から保存した読取記録を選んでください。"))?
                .validate()?
        }
    };
    if let ReaderRequest::File { path, .. } = &request {
        github::file(&fetch()?, &mut snapshot, path)?;
    }
    let result = match request {
        ReaderRequest::Read { .. } | ReaderRequest::Analyze { .. } | ReaderRequest::File { .. } => {
            let model = analysis::analyze(&snapshot, budget);
            let (path, line) = match &request {
                ReaderRequest::Analyze { path, line, .. } => {
                    (path.as_deref(), line.map(|n| n as usize))
                }
                ReaderRequest::File { path, .. } => (Some(path.as_str()), None),
                _ => (None, None),
            };
            let guide = match (path, line) {
                (Some(p), Some(l)) if l > 0 => comprehension::explain(&model, p, l),
                _ => Value::Null,
            };
            let callers = if let Some(p) = path.filter(|_| !guide.is_null()) {
                comprehension::callers(
                    &model,
                    p,
                    comprehension::number(&guide["range"]["start"]),
                    400,
                    80,
                )
            } else {
                Value::Null
            };
            Ok(
                json!({"schema_version":1,"snapshot":snapshot,"model":model,"guide":guide,"callers":callers,"limits":"作者の説明と簡易解析。実行経路・業務上の意味は未確認。未読の範囲と候補を確認してください。"}),
            )
        }
        ReaderRequest::Inquire {
            question,
            path,
            line,
            ..
        } => {
            let focus = path
                .as_deref()
                .zip(line.filter(|l| *l > 0).map(|n| n as usize));
            let mut result = inquiry::inquire(
                &snapshot,
                &analysis::analyze(&snapshot, budget),
                &question,
                focus,
            );
            let mut loaded = vec![];
            let mut failed = vec![];
            let mut client = None;
            for _ in 0..2 {
                let needed = comprehension::array(&result["needed_files"])
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|p| !snapshot.files.contains_key(*p))
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                let before = loaded.len();
                for path in needed {
                    if loaded.len() >= 8 {
                        break;
                    }
                    if client.is_none() {
                        client = Some(fetch()?);
                    }
                    match github::file(
                        client.as_ref().expect("取得処理を初期化済み"),
                        &mut snapshot,
                        &path,
                    ) {
                        Ok(()) => loaded.push(path),
                        Err(e) => failed.push(json!({"path":path,"reason":e.to_string()})),
                    }
                }
                if loaded.len() == before {
                    break;
                }
                result = inquiry::inquire(
                    &snapshot,
                    &analysis::analyze(&snapshot, budget),
                    &question,
                    focus,
                );
            }
            result["loaded_files"] = json!(loaded);
            result["failed_files"] = json!(failed);
            Ok(json!({"schema_version":1,"snapshot":snapshot,"result":result}))
        }
        ReaderRequest::Function { path, line, .. } => {
            let mut detail =
                inquiry::detail(&analysis::analyze(&snapshot, budget), &path, line as usize);
            let mut loaded = vec![];
            if detail["found"] != true {
                let needed = comprehension::array(&detail["needed_files"])
                    .iter()
                    .take(2)
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                if !needed.is_empty() {
                    let client = fetch()?;
                    for p in needed {
                        github::file(&client, &mut snapshot, &p)?;
                        loaded.push(p);
                    }
                    detail = inquiry::detail(
                        &analysis::analyze(&snapshot, budget),
                        &path,
                        line as usize,
                    );
                }
            }
            detail["loaded_files"] = json!(loaded);
            Ok(json!({"schema_version":1,"snapshot":snapshot,"result":detail}))
        }
        ReaderRequest::Architecture { .. } | ReaderRequest::ArchitectureSource { .. } => {
            if snapshot.inventory.iter().any(|e| {
                !snapshot.files.contains_key(&e.path)
                    && snapshot::architecture_priority(&e.path).is_some()
            }) {
                github::architecture(&fetch()?, &mut snapshot)?;
            }
            let result = if matches!(request, ReaderRequest::ArchitectureSource { .. }) {
                Value::Null
            } else {
                repository_map::build(&snapshot)?
            };
            Ok(json!({"schema_version":1,"result":result,"snapshot":snapshot}))
        }
        ReaderRequest::Plan { .. } => Ok(
            json!({"schema_version":1,"snapshot":snapshot,"result":plan::plan(&snapshot,&analysis::analyze(&snapshot, budget))}),
        ),
        _ => Ok(
            json!({"schema_version":1,"snapshot":snapshot,"result":overview::overview(&snapshot,&analysis::analyze(&snapshot, budget))}),
        ),
    };
    if !budget.check() {
        return Err(invalid(
            "コード解析が待機時間または解析量の上限に達しました。対象を分けてください。",
        ));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::parse_token;

    #[test]
    fn long_opaque_tokens_are_accepted_and_header_breaking_text_is_rejected() {
        // GitHub Actionsのトークンは2026年10月から500文字前後のJWT形式になった。
        let long = format!(
            "ghs_12345_{}.{}.{}",
            "a".repeat(200),
            "b-c_".repeat(60),
            "d".repeat(80)
        );
        assert_eq!(parse_token(&format!("{long}\n")).unwrap(), long);
        assert_eq!(
            parse_token("ghs_0123456789abcdef").unwrap(),
            "ghs_0123456789abcdef"
        );
        for bad in [
            "",
            "  \n",
            "ghs_a\r\nX-Injected: 1",
            "ghs_a b",
            &"x".repeat(4097),
        ] {
            assert!(parse_token(bad).is_err(), "{bad:?}");
        }
    }
}

#[cfg(test)]
mod parity {
    use super::*;
    use serde_json::json;
    // 旧JS版の期待結果を保存した架空データ。cargo testではNodeもネットワークも使わない。
    #[test]
    fn rust_matches_saved_browser_analysis() {
        for (name, text) in [
            (
                "scanner",
                include_str!("../tests/fixtures/reader/scanner.json"),
            ),
            (
                "navigation",
                include_str!("../tests/fixtures/reader/navigation.json"),
            ),
            (
                "partial",
                include_str!("../tests/fixtures/reader/partial.json"),
            ),
            ("notes", include_str!("../tests/fixtures/reader/notes.json")),
            (
                "unicode",
                include_str!("../tests/fixtures/reader/unicode.json"),
            ),
            (
                "config",
                include_str!("../tests/fixtures/reader/config.json"),
            ),
            (
                "nested",
                include_str!("../tests/fixtures/reader/nested.json"),
            ),
            (
                "unsupported",
                include_str!("../tests/fixtures/reader/unsupported.json"),
            ),
        ] {
            let baseline: Value = serde_json::from_str(text).unwrap();
            let s = serde_json::from_value::<snapshot::Snapshot>(baseline["snapshot"].clone())
                .unwrap()
                .validate()
                .unwrap();
            let budget = budget::Budget::new();
            let m = analysis::analyze(&s, &budget);
            same(&format!("{name}.model"), &json!(m), &baseline["model"]);
            same(
                &format!("{name}.overview"),
                &overview::overview(&s, &m),
                &baseline["overview"],
            );
            same(
                &format!("{name}.plan"),
                &plan::plan(&s, &m),
                &baseline["plan"],
            );
            for f in comprehension::array(&baseline["functions"]) {
                let p = comprehension::string(&f["path"]);
                let l = comprehension::number(&f["line"]);
                same(
                    &format!("{name}.{p}:{l}.guide"),
                    &comprehension::explain(&m, p, l),
                    &f["guide"],
                );
                same(
                    &format!("{name}.{p}:{l}.callers"),
                    &comprehension::callers(&m, p, l, 400, 80),
                    &f["callers"],
                );
                same(
                    &format!("{name}.{p}:{l}.detail"),
                    &inquiry::detail(&m, p, l),
                    &f["detail"],
                );
            }
            for q in comprehension::array(&baseline["questions"]) {
                let qtext = comprehension::string(&q["question"]);
                same(
                    &format!("{name}.inquire.{qtext}"),
                    &inquiry::inquire(&s, &m, qtext, None),
                    &q["result"],
                );
            }
        }
    }
    fn same(path: &str, actual: &Value, expected: &Value) {
        match (actual, expected) {
            (Value::Object(a), Value::Object(b)) => {
                for (k, v) in b {
                    same(&format!("{path}.{k}"), a.get(k).unwrap_or(&Value::Null), v);
                }
                assert_eq!(
                    a.len(),
                    b.len(),
                    "{path}: keys {:?} != {:?}",
                    a.keys(),
                    b.keys()
                );
            }
            (Value::Array(a), Value::Array(b)) => {
                assert_eq!(a.len(), b.len(), "{path}: array length");
                for (i, (a, b)) in a.iter().zip(b).enumerate() {
                    same(&format!("{path}[{i}]"), a, b);
                }
            }
            _ => assert_eq!(actual, expected, "{path}"),
        }
    }
}

#[cfg(test)]
mod boundaries {
    use super::*;
    use serde_json::json;
    struct Offline;
    impl github::Fetch for Offline {
        fn get(&self, _: &str, _: usize) -> io::Result<Vec<u8>> {
            panic!("解析だけの操作で通信してはいけない");
        }
    }
    struct Source;
    impl github::Fetch for Source {
        fn get(&self, url: &str, _: usize) -> io::Result<Vec<u8>> {
            assert!(url.starts_with(&format!(
                "https://raw.githubusercontent.com/example/project/{}/",
                "a".repeat(40)
            )));
            Ok(b"export function save() {}".to_vec())
        }
    }
    fn snapshot() -> Value {
        let b: Value =
            serde_json::from_str(include_str!("../tests/fixtures/reader/scanner.json")).unwrap();
        b["snapshot"].clone()
    }
    #[test]
    fn analysis_needs_no_client_and_expired_work_is_not_returned_as_success() {
        let request = ReaderRequest::Analyze {
            snapshot: snapshot(),
            path: Some("src/main.rs".into()),
            line: Some(3),
        };
        let result = read_with(
            request,
            || -> io::Result<Offline> { panic!("HTTPクライアントは不要") },
            &budget::Budget::new(),
        )
        .unwrap();
        assert_eq!(result["guide"]["name"], "main");
        let budget = budget::Budget::new();
        budget.stop();
        let request = ReaderRequest::Overview {
            snapshot: snapshot(),
        };
        assert!(
            read_with(request, || Ok(Offline), &budget)
                .unwrap_err()
                .to_string()
                .contains("上限")
        );
        let mut s = snapshot();
        s["files"]["src/main.rs"] = json!(format!(
            "use crate::{}x{};",
            "{".repeat(129),
            "}".repeat(129)
        ));
        assert!(
            read(ReaderRequest::Analyze {
                snapshot: s,
                path: None,
                line: None
            })
            .unwrap_err()
            .to_string()
            .contains("上限")
        );
    }
    #[test]
    fn inquiry_adds_php_and_templates_from_old_inventory_at_the_same_commit() {
        use std::sync::Mutex;
        struct PhpSource(Mutex<Vec<String>>);
        impl github::Fetch for PhpSource {
            fn get(&self, url: &str, _: usize) -> io::Result<Vec<u8>> {
                self.0.lock().unwrap().push(url.into());
                Ok(b"<?php\nsqlInsert('form_soap', $value);\n".to_vec())
            }
        }
        let source = PhpSource(Mutex::new(vec![]));
        impl github::Fetch for &PhpSource {
            fn get(&self, url: &str, limit: usize) -> io::Result<Vec<u8>> {
                (*self).get(url, limit)
            }
        }
        let paths = [
            "interface/forms/soap/save.php",
            "interface/forms/soap/report.php",
            "interface/forms/soap/templates/soap_form.twig",
            "vendor/soap.php",
            "interface/forms/soap/link.php",
        ];
        let s = json!({"schema":1,"fullName":"example/project","commit":"a".repeat(40),
            "inventory":paths.iter().map(|p|json!({"path":p,"mode":if p.ends_with("link.php"){"120000"}else{"100644"}})).collect::<Vec<_>>(),
            "files":{},"omitted":[]});
        let r = read_with(
            ReaderRequest::Inquire {
                snapshot: s,
                question: "SOAPを保存して開き直す処理".into(),
                path: None,
                line: None,
            },
            || Ok(&source),
            &budget::Budget::new(),
        )
        .unwrap();
        assert_eq!(r["result"]["loaded_files"].as_array().unwrap().len(), 3);
        assert!(
            source
                .0
                .lock()
                .unwrap()
                .iter()
                .all(|u| u.contains(&format!("/{}/", "a".repeat(40))))
        );
        assert!(r["result"]["findings"].as_array().unwrap().iter().any(|f| {
            f["refs"]
                .as_array()
                .is_some_and(|refs| refs.iter().any(|r| r["path"] == paths[0]))
        }));
        assert!(r["result"]["functions"].as_array().unwrap().is_empty());
    }
    #[test]
    fn unread_function_is_loaded_and_inquiry_reads_at_most_eight_files() {
        let inventory = (0..12)
            .map(|i| json!({"path":format!("store{i}.js"),"mode":"100644","size":30}))
            .chain(std::iter::once(
                json!({"path":"app.js","mode":"100644","size":600}),
            ))
            .collect::<Vec<_>>();
        let code = (0..12)
            .map(|i| format!("import {{ save as save{i} }} from './store{i}.js';"))
            .collect::<Vec<_>>()
            .join("\n");
        let s = json!({"schema":1,"fullName":"example/project","reference":"main","commit":"a".repeat(40),"inventory":inventory,"files":{"app.js":code},"omitted":(0..12).map(|i|json!({"path":format!("store{i}.js"),"reason":"初回の読取範囲外"})).collect::<Vec<_>>()});
        let r = read_with(
            ReaderRequest::Function {
                snapshot: s.clone(),
                path: "store0.js".into(),
                line: 1,
            },
            || Ok(Source),
            &budget::Budget::new(),
        )
        .unwrap();
        assert_eq!(r["result"]["found"], true);
        assert_eq!(r["result"]["loaded_files"], json!(["store0.js"]));
        let r = read_with(
            ReaderRequest::Inquire {
                snapshot: s,
                question: "保存".into(),
                path: None,
                line: None,
            },
            || Ok(Source),
            &budget::Budget::new(),
        )
        .unwrap();
        assert_eq!(r["result"]["loaded_files"].as_array().unwrap().len(), 8);
        assert!(r["result"]["failed_files"].as_array().unwrap().is_empty());
        assert_eq!(r["snapshot"]["files"].as_object().unwrap().len(), 9);
    }
}
