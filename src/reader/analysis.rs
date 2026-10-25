//! 宣言・参照宣言・呼び出し候補の簡易解析。完全な構文解析ではない。
use super::snapshot::{self, Snapshot};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::HashSet;
#[derive(Debug, Serialize)]
pub struct Symbol {
    pub name: String,
    pub line: usize,
    pub end: usize,
}
#[derive(Debug, Serialize)]
pub struct Binding {
    pub kind: String,
    pub local: String,
    pub imported: Option<String>,
    pub target: Option<String>,
    pub specifier: String,
    pub nested: bool,
    pub line: usize,
    pub end_line: usize,
    pub declaration: String,
}
#[derive(Debug, Serialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub line: usize,
    pub kind: &'static str,
}
#[derive(Debug, Serialize)]
pub struct File<'a> {
    pub path: String,
    pub text: String,
    pub symbols: Vec<Symbol>,
    pub imports: Vec<Edge>,
    pub bindings: Vec<Binding>,
    pub role: &'static str,
    #[serde(skip)]
    pub budget: &'a super::budget::Budget,
    #[serde(skip)]
    pub line_offsets: Vec<usize>,
}
#[derive(Debug, Serialize)]
pub struct Model<'a> {
    pub files: Vec<File<'a>>,
    pub entries: Vec<Value>,
    pub scripts: Vec<Value>,
    pub inventory: &'a [snapshot::Entry],
    pub inventory_truncated: bool,
    pub edges: Vec<Value>,
    #[serde(skip)]
    pub file_index: std::collections::HashMap<String, usize>,
    #[serde(skip)]
    pub guides: std::cell::RefCell<std::collections::HashMap<(String, usize), Value>>,
    #[serde(skip)]
    pub budget: &'a super::budget::Budget,
}
impl File<'_> {
    pub fn line_text(&self, line: usize) -> Option<&str> {
        let start = *self.line_offsets.get(line.checked_sub(1)?)?;
        let end = self
            .line_offsets
            .get(line)
            .map_or(self.text.len(), |n| n.saturating_sub(1));
        self.text.get(start..end)
    }
}
impl Model<'_> {
    pub fn file(&self, path: &str) -> Option<&File<'_>> {
        self.file_index.get(path).and_then(|i| self.files.get(*i))
    }
}
pub fn line_at(s: &str, n: usize) -> usize {
    s.as_bytes()[..n].iter().filter(|b| **b == b'\n').count() + 1
}
pub fn line_offset(s: &str, line: usize) -> usize {
    if line <= 1 {
        return 0;
    }
    s.match_indices('\n')
        .nth(line - 2)
        .map_or(s.len(), |(n, _)| n + 1)
}
pub fn clip(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}
pub fn mask(text: &str, rust: bool, python: bool) -> String {
    let b = text.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0;
    while i < b.len() {
        let start = i;
        if (!python && b[i..].starts_with(b"//")) || (python && b[i] == b'#') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if !python && b[i..].starts_with(b"/*") {
            i += 2;
            let mut depth = 1;
            while i < b.len() && depth > 0 {
                if rust && b[i..].starts_with(b"/*") {
                    depth += 1;
                    i += 2;
                } else if b[i..].starts_with(b"*/") {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
        } else if rust
            && (i == 0 || !b[i - 1].is_ascii_alphanumeric() && b[i - 1] != b'_')
            && (b[i] == b'r' || b[i..].starts_with(b"br"))
        {
            let r = i + usize::from(b[i] == b'b') + 1;
            let mut j = r;
            while j < b.len() && b[j] == b'#' {
                j += 1;
            }
            if j < b.len() && b[j] == b'"' {
                let mut close = vec![b'"'];
                close.extend_from_slice(&b[r..j]);
                i = j + 1;
                while i < b.len() && !b[i..].starts_with(&close) {
                    i += 1;
                }
                i = (i + close.len()).min(b.len());
            } else {
                i += 1;
                continue;
            }
        } else if matches!(b[i], b'"' | b'\'') || (!rust && !python && b[i] == b'`') {
            if rust && b[i] == b'\'' {
                // 文字リテラルのみ消す。ライフタイムは残す。UTF-8の1文字も扱う。
                let tail = &text[i + 1..];
                let end = if tail.starts_with('\\') {
                    tail.chars().take(2).map(char::len_utf8).sum()
                } else {
                    tail.chars().next().map_or(0, char::len_utf8)
                };
                if !tail.as_bytes().get(end).is_some_and(|c| *c == b'\'') {
                    i += 1;
                    continue;
                }
            }
            let q = b[i];
            let triple = python && b[i..].starts_with(&[q, q, q]);
            let delimiter = if triple { 3 } else { 1 };
            i += delimiter;
            while i < b.len() {
                if b[i] == b'\\' {
                    i = (i + 2).min(b.len());
                    continue;
                }
                if b[i..].starts_with(&vec![q; delimiter]) {
                    i += delimiter;
                    break;
                }
                i += 1;
            }
        } else if !rust && !python && b[i] == b'/' && {
            let prefix = String::from_utf8_lossy(&b[i.saturating_sub(100)..i]);
            rx!(r"(?:^|[=(:,!&|?;{}]|\breturn|=>)\s*$").is_match(&prefix)
        } {
            i += 1;
            let mut bracket = false;
            while i < b.len() && b[i] != b'\n' {
                if b[i] == b'\\' {
                    i = (i + 2).min(b.len());
                    continue;
                }
                if b[i] == b'[' {
                    bracket = true;
                }
                if b[i] == b']' {
                    bracket = false;
                }
                if b[i] == b'/' && !bracket {
                    i += 1;
                    while i < b.len() && b[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                    break;
                }
                i += 1;
            }
        } else {
            i += 1;
            continue;
        }
        for c in &mut out[start..i] {
            if *c != b'\n' {
                *c = b' ';
            }
        }
    }
    // UTF-8はコード位置以外で一括してマスクした。元のコードは変更しない。
    String::from_utf8(out).expect("マスクはUTF-8の文字列境界を保つ")
}
pub fn symbols(path: &str, text: &str, budget: &super::budget::Budget) -> Vec<Symbol> {
    if !snapshot::source(path) {
        return vec![];
    }
    let clean = mask(text, path.ends_with(".rs"), path.ends_with(".py"));
    let lines = clean.split('\n').collect::<Vec<_>>();
    let mut result = vec![];
    let pattern = rx!(
        r"(?-u)(?:^|\s)(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+([A-Za-z_]\w*)\s*\(|(?:^|\s)(?:export\s+)?(?:async\s+)?function\s+([A-Za-z_$][\w$]*)\s*\(|^\s*(?:async\s+)?def\s+([A-Za-z_]\w*)\s*\(|(?:^|\s)(?:export\s+)?(?:const|let)\s+([A-Za-z_$][\w$]*)\s*=.*=>"
    );
    for (i, l) in lines.iter().enumerate().take_while(|_| budget.check()) {
        if let Some(c) = pattern.captures(l) {
            let name = (1..=4)
                .find_map(|n| c.get(n))
                .expect("宣言パターンは名前を含む")
                .as_str()
                .to_owned();
            result.push(Symbol {
                name,
                line: i + 1,
                end: 0,
            });
        }
    }
    for i in 0..result.len() {
        result[i].end = result.get(i + 1).map_or(lines.len(), |s| s.line - 1);
    }
    result
}
pub fn rust_path(parts: &[&str], paths: &HashSet<&str>) -> Option<String> {
    let base = format!("src/{}", parts.join("/"));
    [format!("{base}.rs"), format!("{base}/mod.rs")]
        .into_iter()
        .find(|p| paths.contains(p.as_str()))
}
fn resolve(path: &str, spec: &str, paths: &HashSet<&str>) -> Option<String> {
    let directory = path.rsplit_once('/').map_or("", |(d, _)| d);
    let combined = format!("{directory}/{spec}");
    let mut out = vec![];
    for p in combined.split('/') {
        if p.is_empty() || p == "." {
            continue;
        }
        if p == ".." {
            out.pop()?;
        } else {
            out.push(p);
        }
    }
    let base = out.join("/");
    let extensions: &[&str] = if path.ends_with(".py") {
        &[".py", "/__init__.py"]
    } else if path.ends_with(".rs") {
        &[".rs", "/mod.rs"]
    } else {
        &[
            ".js",
            ".mjs",
            ".cjs",
            ".ts",
            ".tsx",
            ".jsx",
            "/index.js",
            "/index.ts",
            "/index.tsx",
        ]
    };
    std::iter::once(base.clone())
        .chain(extensions.iter().map(|e| format!("{base}{e}")))
        .find(|p| paths.contains(p.as_str()))
}
fn split_use(s: &str) -> Vec<&str> {
    let (mut depth, mut start) = (0, 0);
    let mut out = vec![];
    for (i, b) in s.bytes().enumerate() {
        if b == b',' && depth == 0 {
            out.push(s[start..i].trim());
            start = i + 1;
        } else if b == b'{' {
            depth += 1;
        } else if b == b'}' {
            depth -= 1;
        }
    }
    out.push(s[start..].trim());
    out.into_iter().filter(|s| !s.is_empty()).collect()
}
struct Imports<'a> {
    path: &'a str,
    text: &'a str,
    clean: String,
    paths: &'a HashSet<&'a str>,
    crate_name: Option<&'a str>,
    bindings: Vec<Binding>,
}
impl Imports<'_> {
    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        start: usize,
        end: usize,
        kind: &str,
        local: &str,
        imported: Option<&str>,
        target: Option<String>,
        specifier: &str,
    ) {
        let prefix = &self.clean[..start];
        let matched = &self.clean[start..end];
        let nested = if self.path.ends_with(".py") {
            matched.starts_with([' ', '\t'])
        } else {
            prefix.bytes().filter(|b| *b == b'{').count()
                > prefix.bytes().filter(|b| *b == b'}').count()
        };
        self.bindings.push(Binding {
            kind: kind.to_owned(),
            local: local.to_owned(),
            imported: imported.map(str::to_owned),
            target,
            specifier: specifier.to_owned(),
            nested,
            line: line_at(&self.clean, start),
            end_line: line_at(&self.clean, end),
            declaration: self.text[start..end].trim().to_owned(),
        });
    }
    fn visit(&mut self, part: &str, prefix: Vec<&str>, start: usize, end: usize) {
        if let Some(brace) = part.find('{') {
            let Some(close) = part.rfind('}') else { return };
            if close < brace || !part[close + 1..].trim().is_empty() {
                return;
            }
            let head = part[..brace].trim().trim_end_matches("::");
            let mut next = prefix;
            next.extend(head.split("::").map(str::trim).filter(|s| !s.is_empty()));
            for inner in split_use(&part[brace + 1..close]) {
                self.visit(inner, next.clone(), start, end);
            }
            return;
        }
        let alias = rx!(r"\s+as\s+").split(part).collect::<Vec<_>>();
        if alias.len() > 2 {
            return;
        }
        let mut names = prefix;
        names.extend(alias[0].split("::").map(str::trim));
        if !names
            .iter()
            .all(|n| rx!(r"(?-u)^[A-Za-z_$][\w$]*$").is_match(n))
            || alias
                .get(1)
                .is_some_and(|n| !rx!(r"(?-u)^[A-Za-z_$][\w$]*$").is_match(n.trim()))
            || names.len() < 2
        {
            return;
        }
        let own = names[0] == "crate" || Some(names[0]) == self.crate_name;
        let rel = &names[1..];
        let is_self = rel.last() == Some(&"self");
        let module = if is_self { &rel[..rel.len() - 1] } else { rel };
        let direct = if own {
            rust_path(module, self.paths)
        } else {
            None
        };
        let kind = if is_self || direct.is_some() {
            "namespace"
        } else {
            "named"
        };
        let target = if own {
            direct.or_else(|| rust_path(&rel[..rel.len() - 1], self.paths))
        } else {
            None
        };
        let local = alias.get(1).map(|n| n.trim()).or_else(|| {
            if is_self {
                rel.get(rel.len().wrapping_sub(2)).copied()
            } else {
                rel.last().copied()
            }
        });
        if let Some(local) = local {
            self.add(
                start,
                end,
                kind,
                local,
                if kind == "namespace" {
                    None
                } else {
                    rel.last().copied()
                },
                target,
                &names.join("::"),
            );
        }
    }
}
fn bindings(
    path: &str,
    text: &str,
    paths: &HashSet<&str>,
    crate_name: Option<&str>,
    budget: &super::budget::Budget,
) -> Vec<Binding> {
    let clean = mask(text, path.ends_with(".rs"), path.ends_with(".py"));
    let mut depth = 0usize;
    for b in clean.bytes() {
        if b == b'{' {
            depth += 1;
            if depth > 128 {
                budget.stop();
                return vec![];
            }
        } else if b == b'}' {
            depth = depth.saturating_sub(1);
        }
    }
    let mut ctx = Imports {
        path,
        text,
        clean: clean.clone(),
        paths,
        crate_name,
        bindings: vec![],
    };
    if rx!(r"\.[cm]?[jt]sx?$").is_match(path) {
        // 引用符の一致はコードで確認する（Rustのregexは後方参照を使わない）。
        for c in rx!(r#"(?-u)\bimport\s+([\w$\s{},*]+?)\s+from\s*(['"])([^'"\n]+)(['"])\s*;?"#)
            .captures_iter(text)
        {
            let m = c.get(0).expect("全体の一致");
            if c[2] != c[4] || !rx!(r"^import\b").is_match(&clean[m.start()..]) {
                continue;
            }
            let clause = c[1].trim();
            let spec = &c[3];
            if rx!(r"^type\s").is_match(clause) {
                continue;
            }
            let target = if spec.starts_with('.') {
                resolve(path, spec, paths)
            } else {
                None
            };
            if let Some(named) = rx!(r"\{([^}]*)\}").captures(clause) {
                for item in named[1].split(',') {
                    let value = item.trim();
                    if value.starts_with("type ") {
                        continue;
                    }
                    if let Some(b) =
                        rx!(r"(?-u)^([A-Za-z_$][\w$]*)(?:\s+as\s+([A-Za-z_$][\w$]*))?$")
                            .captures(value)
                    {
                        ctx.add(
                            m.start(),
                            m.end(),
                            "named",
                            b.get(2).map_or(&b[1], |m| m.as_str()),
                            Some(&b[1]),
                            target.clone(),
                            spec,
                        );
                    }
                }
            }
            if let Some(b) =
                rx!(r"(?-u)(?:^|,)\s*\*\s+as\s+([A-Za-z_$][\w$]*)\s*$").captures(clause)
            {
                ctx.add(
                    m.start(),
                    m.end(),
                    "namespace",
                    &b[1],
                    None,
                    target.clone(),
                    spec,
                );
            }
            if let Some(b) = rx!(r"(?-u)^([A-Za-z_$][\w$]*)(?:\s*,|\s*$)").captures(clause) {
                ctx.add(
                    m.start(),
                    m.end(),
                    "default",
                    &b[1],
                    Some("default"),
                    target.clone(),
                    spec,
                );
            }
        }
    } else if path.ends_with(".rs") {
        for c in rx!(r"\buse\s+([^;]+);").captures_iter(&clean) {
            let m = c.get(0).expect("全体の一致");
            ctx.visit(c[1].trim(), vec![], m.start(), m.end());
        }
    } else if path.ends_with(".py") {
        for c in
            rx!(r"(?m-u)^[ \t]*from\s+([.\w]+)\s+import\s+(\([^)]*\)|[^\n]+)").captures_iter(&clean)
        {
            let m = c.get(0).expect("全体の一致");
            let module = &c[1];
            let dots = module.bytes().take_while(|b| *b == b'.').count();
            let spec = if dots > 0 {
                format!(
                    "./{}{}",
                    "../".repeat(dots - 1),
                    module[dots..].replace('.', "/")
                )
            } else {
                module.to_owned()
            };
            let namespace = dots > 0 && dots == module.len();
            for item in c[2]
                .trim_start_matches('(')
                .trim_end_matches(')')
                .split(',')
            {
                if let Some(b) =
                    rx!(r"(?-u)^\s*([A-Za-z_]\w*)(?:\s+as\s+([A-Za-z_]\w*))?\s*$").captures(item)
                {
                    let target = if dots > 0 {
                        resolve(
                            path,
                            &format!("{spec}{}", if namespace { &b[1] } else { "" }),
                            paths,
                        )
                    } else {
                        None
                    };
                    ctx.add(
                        m.start(),
                        m.end(),
                        if namespace { "namespace" } else { "named" },
                        b.get(2).map_or(&b[1], |m| m.as_str()),
                        if namespace { None } else { Some(&b[1]) },
                        target,
                        &spec,
                    );
                }
            }
        }
        for c in rx!(r"(?m-u)^[ \t]*import\s+([A-Za-z_]\w*(?:\.[A-Za-z_]\w*)*)(?:\s+as\s+([A-Za-z_]\w*))?[ \t]*(?:$|#)").captures_iter(&clean){let m=c.get(0).expect("全体の一致");ctx.add(m.start(),m.end(),"namespace",c.get(2).map_or_else(||c[1].split('.').next().expect("モジュール名"),|m|m.as_str()),None,None,&c[1]);}
    }
    ctx.bindings
}
fn imports(path: &str, text: &str, paths: &HashSet<&str>, crate_name: Option<&str>) -> Vec<Edge> {
    let clean = mask(text, path.ends_with(".rs"), path.ends_with(".py"));
    let mut edges = vec![];
    let mut add = |target: Option<String>, line: usize| {
        if let Some(to) = target.filter(|t| t != path) {
            edges.push(Edge {
                from: path.to_owned(),
                to,
                line,
                kind: "参照宣言",
            });
        }
    };
    for (i, (original, line)) in text.split('\n').zip(clean.split('\n')).enumerate() {
        let code = line.trim();
        let mut target = None;
        if rx!(r"^(?:import|export)\b|\brequire\s*\(").is_match(code)
            && let Some(c) =
                rx!(r#"(?:from\s*|import\s*|require\s*\(\s*)["']([^"']+)["']"#).captures(original)
            && c[1].starts_with('.')
        {
            target = resolve(path, &c[1], paths);
        }
        if path.ends_with(".rs") {
            if let Some(c) = rx!(r"(?-u)^(?:pub\s+)?mod\s+([A-Za-z_]\w*)\s*;").captures(code) {
                let dir = if rx!(r"/(?:lib|main|mod)\.rs$").is_match(path) {
                    path.rsplit_once('/').map_or("", |(d, _)| d)
                } else {
                    path.trim_end_matches(".rs")
                };
                target = [
                    format!("{dir}/{}.rs", &c[1]),
                    format!("{dir}/{}/mod.rs", &c[1]),
                ]
                .into_iter()
                .find(|p| paths.contains(p.as_str()));
            }
            if let Some(c) = rx!(r"(?-u)^use\s+crate::([A-Za-z_]\w*)").captures(code) {
                target = rust_path(&[&c[1]], paths);
            }
        }
        if path.ends_with(".py")
            && let Some(c) = rx!(r"(?-u)^from\s+(\.+)([\w.]*)\s+import\b").captures(code)
            && !c[2].is_empty()
        {
            target = resolve(
                path,
                &format!(
                    "./{}{}",
                    "../".repeat(c[1].len() - 1),
                    c[2].replace('.', "/")
                ),
                paths,
            );
        }
        add(target, i + 1);
    }
    if path.ends_with(".rs") {
        for c in rx!(r"(?-u)\buse\s+([A-Za-z_]\w*)::\{([\s\S]*?)\}\s*;").captures_iter(&clean) {
            if &c[1] != "crate" && Some(&c[1]) != crate_name {
                continue;
            }
            for part in split_use(&c[2]) {
                if let Some(m) = rx!(r"(?-u)^\s*([A-Za-z_]\w*)").captures(part) {
                    add(
                        rust_path(&[&m[1]], paths),
                        line_at(&clean, c.get(0).expect("全体の一致").start()),
                    );
                }
            }
        }
    }
    edges
}
pub fn role(p: &str) -> &'static str {
    if snapshot::readme(p) {
        "プロジェクトの説明"
    } else if snapshot::manifest(p) {
        "起動設定・依存パッケージ"
    } else if p.starts_with(".github/workflows/") {
        "自動テスト・配布の設定"
    } else if rx!(r"(^|/)(tests?|__tests__|tools)(/|\.)|\.(test|spec)\.").is_match(p) {
        "テスト・補助処理"
    } else if p.ends_with(".rs") {
        "Rustのコード"
    } else if rx!(r"\.[cm]?[jt]sx?$").is_match(p) {
        "JavaScript / TypeScriptのコード"
    } else if p.ends_with(".py") {
        "Pythonのコード"
    } else {
        "資料"
    }
}
pub fn analyze<'a>(s: &'a Snapshot, budget: &'a super::budget::Budget) -> Model<'a> {
    let paths = s.inventory.iter().map(|e| e.path.as_str()).collect();
    let crate_name = s
        .files
        .get("Cargo.toml")
        .and_then(Value::as_str)
        .and_then(|t| rx!(r#"(?m)^name\s*=\s*"([^"]+)""#).captures(t))
        .map(|c| c[1].replace('-', "_"));
    let files = s
        .files
        .iter()
        .take_while(|_| budget.check())
        .filter_map(|(path, v)| v.as_str().map(|t| (path, t)))
        .map(|(path, t)| {
            let bindings = bindings(path, t, &paths, crate_name.as_deref(), budget);
            let mut imports = imports(path, t, &paths, crate_name.as_deref());
            for b in &bindings {
                if let Some(target) = &b.target
                    && target != path
                    && !imports.iter().any(|e| e.to == *target && e.line == b.line)
                {
                    imports.push(Edge {
                        from: path.to_owned(),
                        to: target.to_owned(),
                        line: b.line,
                        kind: "参照宣言",
                    });
                }
            }
            File {
                path: path.to_owned(),
                text: t.to_owned(),
                line_offsets: std::iter::once(0)
                    .chain(t.match_indices('\n').map(|(i, _)| i + 1))
                    .collect(),
                symbols: symbols(path, t, budget),
                imports,
                bindings,
                role: role(path),
                budget,
            }
        })
        .collect::<Vec<_>>();
    let mut entries = vec![];
    let mut scripts = vec![];
    for f in &files {
        if rx!(r"(^|/)(main\.(rs|py)|server\.[cm]?[jt]s|app\.[jt]sx?|index\.[cm]?[jt]sx?)$")
            .is_match(&f.path)
        {
            entries.push(json!({"path":f.path,"line":f.symbols.iter().find(|s|s.name=="main").map_or(1,|s|s.line),"reason":if f.path.ends_with("main.rs"){"Rustの実行入口に使われるファイル"}else{"名前から選んだ入口の候補"}}));
        }
    }
    for f in &files {
        if f.path.ends_with(".py")
            && !entries.iter().any(|e| e["path"] == f.path)
            && let Some(i) = f
                .text
                .split('\n')
                .position(|l| rx!(r#"^if\s+__name__\s*==\s*["']__main__["']\s*:"#).is_match(l))
        {
            entries.push(json!({"path":f.path,"line":i+1,"reason":"直接起動したときに動く記述（if __name__ == '__main__'）があるファイル"}));
        }
    }
    for f in &files {
        if rx!(r"(^|/)package\.json$").is_match(&f.path)
            && let Ok(pkg) = serde_json::from_str::<Value>(&f.text)
            && let Some(obj) = pkg["scripts"].as_object()
        {
            for (name, cmd) in obj {
                if cmd.is_string() {
                    let key = serde_json::to_string(name).expect("文字列はJSONへ変換できる");
                    scripts.push(json!({"path":f.path,"name":name,"command":cmd,"line":f.text.split('\n').position(|l|l.contains(&key)).map_or(0,|i|i+1)}));
                }
            }
        }
    }
    let edges = files
        .iter()
        .flat_map(|f| f.imports.iter().map(|e| json!(e)))
        .collect();
    let file_index = files
        .iter()
        .enumerate()
        .map(|(i, f)| (f.path.clone(), i))
        .collect();
    Model {
        files,
        file_index,
        entries,
        scripts,
        inventory: &s.inventory,
        inventory_truncated: s.truncated,
        edges,
        guides: Default::default(),
        budget,
    }
}
