//! 関数の範囲、読解案内、呼び出し候補。実行順序や到達を確定しない。
use super::analysis::{self, File, Model, Symbol, line_at, line_offset, mask};
use serde_json::{Value, json};
use std::collections::HashSet;
#[derive(Debug, Clone)]
pub struct Range {
    pub start: usize,
    pub end: usize,
    pub header: Option<usize>,
}
impl Range {
    pub fn json(&self, text: &str) -> Value {
        let mut v = json!({"start":self.start,"end":self.end});
        if let Some(h) = self.header {
            v["header_end"] = json!(text[..h].encode_utf16().count());
        }
        v
    }
}
pub fn range(file: &File, s: &Symbol, clean: &str) -> Range {
    let lines = clean.split('\n').collect::<Vec<_>>();
    let mut r = Range {
        start: s.line,
        end: s.line,
        header: None,
    };
    if file.path.ends_with(".py") {
        let indent = lines[s.line - 1].len() - lines[s.line - 1].trim_start().len();
        for (i, line) in lines.iter().enumerate().skip(s.line) {
            if line.trim().is_empty() {
                continue;
            }
            if line.len() - line.trim_start().len() <= indent {
                break;
            }
            r.end = i + 1;
        }
        return r;
    }
    let offset = line_offset(clean, s.line);
    let mut parens = 0;
    let mut brace = None;
    for (i, b) in clean.as_bytes().iter().enumerate().skip(offset) {
        if *b == b'(' {
            parens += 1;
        }
        if *b == b')' {
            parens -= 1;
        }
        if *b == b'{' && parens == 0 {
            brace = Some(i);
            break;
        }
    }
    let boundary = file
        .symbols
        .iter()
        .find(|x| x.line > s.line)
        .map_or(lines.len() + 1, |x| x.line);
    let Some(brace) = brace.filter(|i| line_at(clean, *i) < boundary) else {
        return r;
    };
    let mut depth = 0;
    for (i, b) in clean.as_bytes().iter().enumerate().skip(brace) {
        if *b == b'{' {
            depth += 1;
        }
        if *b == b'}' {
            depth -= 1;
            if depth == 0 {
                r.end = line_at(clean, i);
                r.header = Some(brace);
                return r;
            }
        }
    }
    r
}
fn collision(file: &File, name: &str, r: &Range) -> bool {
    if !rx!(r"^[A-Za-z_$][\w$]*$").is_match(name) {
        return false;
    }
    let clean = mask(
        &file.text,
        file.path.ends_with(".rs"),
        file.path.ends_with(".py"),
    );
    let offset = line_offset(&clean, r.start);
    let head_end = r.header.unwrap_or_else(|| {
        clean[offset..]
            .find('\n')
            .map_or(clean.len(), |n| offset + n)
    });
    let header = &clean[offset..head_end];
    let escaped = regex::escape(name);
    let parameters = rx!(r"\(([\s\S]*)\)").captures(header);
    if let Some(p) = parameters
        && file
            .budget
            .pattern(&format!(
                r"(?:^|[^A-Za-z0-9_$]){escaped}(?:$|[^A-Za-z0-9_$])"
            ))
            .is_match(&p[1])
    {
        return true;
    }
    let end = line_offset(&clean, r.end + 1)
        .saturating_sub(usize::from(r.end < clean.split('\n').count()));
    let start = if r.header.is_some() {
        (head_end + 1).min(end)
    } else {
        head_end.min(end)
    };
    file.budget.pattern(&format!(r"(?-u:\b)(?:let|const|var|fn|function|def|class)\s+(?:mut\s+)?{escaped}(?:$|[^A-Za-z0-9_$])|(?:^|\n)\s*{escaped}\s*=")).is_match(&clean[start..end])
}
fn targets(model: &Model<'_>, file: &File, name: &str, r: &Range) -> Value {
    let parts = rx!(r"::|\.").split(name).collect::<Vec<_>>();
    let local = parts[0];
    let imported = parts[parts.len() - 1];
    let bindings = file
        .bindings
        .iter()
        .filter(|b| b.local == local && (!b.nested || b.line > r.start && b.end_line <= r.end))
        .collect::<Vec<_>>();
    if collision(file, local, r) {
        return json!({"candidates":[],"bindings":bindings,"unread":[],"reason":"引数や局所宣言に同名の記述があり、対応先を特定できません。"});
    }
    let mut t: Vec<(String, String, String)> = vec![];
    if !bindings.is_empty() {
        for b in &bindings {
            if ((parts.len() == 1 && b.kind != "namespace")
                || (parts.len() == 2 && b.kind == "namespace"))
                && let Some(p) = &b.target
            {
                t.push((
                    p.to_owned(),
                    if b.kind == "namespace" {
                        imported.to_owned()
                    } else {
                        b.imported.clone().unwrap_or_default()
                    },
                    b.kind.to_owned(),
                ));
            }
        }
    } else if parts.len() == 1 {
        t.push((
            file.path.to_owned(),
            imported.to_owned(),
            "local".to_owned(),
        ));
    } else if parts[0] == "crate" {
        let paths = model.inventory.iter().map(|e| e.path.as_str()).collect();
        if let Some(p) = analysis::rust_path(&parts[1..parts.len() - 1], &paths) {
            t.push((p, imported.to_owned(), "qualified".to_owned()));
        }
    }
    let mut candidates = vec![];
    let mut unread = vec![];
    for (path, mut symbol, kind) in t {
        let Some(destination) = model.files.iter().find(|f| f.path == path) else {
            if !unread.contains(&path) {
                unread.push(path);
            }
            continue;
        };
        if kind == "default" {
            let clean = mask(&destination.text, false, false);
            symbol=rx!(r"\bexport\s+default\s+(?:async\s+)?function\s+([A-Za-z_$][\w$]*)|\bexport\s+default\s+([A-Za-z_$][\w$]*)\s*;").captures(&clean).and_then(|c|c.get(1).or_else(||c.get(2))).map_or(String::new(),|m|m.as_str().to_owned());
        }
        for s in destination.symbols.iter().filter(|s| s.name == symbol) {
            if !candidates
                .iter()
                .any(|v: &Value| v["path"] == path && v["line"] == s.line)
            {
                candidates.push(json!({"path":path,"line":s.line,"name":s.name}));
            }
        }
    }
    let reason = if !candidates.is_empty() {
        if !bindings.is_empty() {
            "読み込み宣言の名前と参照先の宣言から選んだ候補です。"
        } else {
            "同じファイルの名前、またはcrateの記述から選んだ候補です。"
        }
    } else if !unread.is_empty() {
        "参照先をまだ読み取っていません。"
    } else {
        "対応する宣言は抽出できませんでした。動的な呼び出しや未対応構文は未解決です。"
    };
    json!({"candidates":candidates,"bindings":bindings,"unread":unread,"reason":reason})
}
pub fn explain(model: &Model<'_>, path: &str, selected: usize) -> Value {
    if !model.budget.check() {
        return Value::Null;
    }
    let key = (path.to_owned(), selected);
    if let Some(v) = model.guides.borrow().get(&key) {
        return v.clone();
    }
    let Some(file) = model.files.iter().find(|f| f.path == path) else {
        return Value::Null;
    };
    let clean = mask(&file.text, path.ends_with(".rs"), path.ends_with(".py"));
    let ranges = file
        .symbols
        .iter()
        .map(|s| (s, range(file, s, &clean)))
        .collect::<Vec<_>>();
    let Some((symbol, r)) = ranges
        .iter()
        .rev()
        .find(|(_, r)| r.start <= selected && selected <= r.end)
    else {
        return Value::Null;
    };
    let mut steps = vec![];
    let mut calls = vec![];
    let lines = clean.split('\n').collect::<Vec<_>>();
    for (number, line) in lines.iter().enumerate().take(r.end).skip(r.start - 1) {
        let number = number + 1;
        if ranges
            .iter()
            .any(|(_, other)| other.start > r.start && other.start <= number && number <= other.end)
        {
            continue;
        }
        let rules = [
            (
                "branch",
                rx!(r"\b(if|else|match|switch)\b").is_match(line),
                "条件に応じて処理を分ける記述があります。条件式と各分岐を確認してください。",
            ),
            (
                "loop",
                rx!(r"\b(for|while|loop)\b").is_match(line),
                "処理を繰り返す記述があります。繰り返す対象と終了条件を確認してください。",
            ),
            (
                "error",
                rx!(r"\b(throw|raise|panic)\b|\b(?:panic|bail)!").is_match(line),
                "例外やエラーを発生させる記述があります。呼び出し元の扱いを確認してください。",
            ),
            (
                "return",
                rx!(r"\breturn\b").is_match(line),
                "この位置で呼び出し元へ戻る記述があります。返す値をコードで確認してください。",
            ),
            (
                "wait",
                rx!(r"\bawait\b|\.await\b").is_match(line),
                "非同期処理の完了を待つ記述があります。待機後の成功・失敗の扱いを確認してください。",
            ),
            (
                "propagate",
                path.ends_with(".rs") && line.contains('?'),
                "?演算子があります。ResultやOptionの値に応じて呼び出し元へ戻る可能性があります。",
            ),
        ];
        for (kind, yes, text) in rules {
            if yes {
                steps.push(json!({"kind":kind,"line":number,"text":text}));
            }
        }
        for c in rx!(r"\b([A-Za-z_$][\w$]*(?:(?:::|\.)[A-Za-z_$][\w$]*)*)\s*(!)?\s*\(")
            .captures_iter(line)
        {
            let name = &c[1];
            let preceding = &line[..c.get(0).expect("全体の一致").start()];
            if [
                "if", "for", "while", "switch", "catch", "match", "fn", "function", "def",
                "return", "sizeof", "Some", "Ok", "Err",
            ]
            .contains(&name)
                || rx!(r"(?:fn|function|def)\s*$").is_match(preceding)
            {
                continue;
            }
            let mut call = if rx!(r"[)\]]\s*\.\s*$").is_match(preceding) {
                json!({"candidates":[],"bindings":[],"unread":[],"reason":"式の結果に対するメソッド呼び出しで、対応先を特定できません。"})
            } else {
                targets(model, file, name, r)
            };
            call["name"] = json!(name);
            call["line"] = json!(number);
            call["macro"] = json!(c.get(2).is_some());
            calls.push(call);
        }
    }
    let start = line_offset(&file.text, r.start);
    let signature = if let Some(h) = r.header {
        file.text[start..h].trim()
    } else {
        file.line_text(r.start).unwrap_or("").trim()
    };
    let truncated = steps.len() > 80 || calls.len() > 80;
    steps.truncate(80);
    calls.truncate(80);
    let value = json!({"name":symbol.name,"range":r.json(&file.text),"signature":signature,"steps":steps,"calls":calls,"truncated":truncated,"limits":"コードの記述から作った読解案内です。呼び出し先は名前と参照宣言からの候補で、実際の到達・実行順序・通信や保存の成否は未確認です。動的呼び出しや未対応構文は取りこぼす場合があります。"});
    model.guides.borrow_mut().insert(key, value.clone());
    value
}
pub fn callers(
    model: &Model<'_>,
    path: &str,
    line: usize,
    max_functions: usize,
    max_callers: usize,
) -> Value {
    let mut found = vec![];
    let mut seen = HashSet::new();
    let mut scanned = 0;
    let mut limited = false;
    'outer: for f in &model.files {
        for s in &f.symbols {
            if scanned >= max_functions || !model.budget.check() {
                limited = true;
                break 'outer;
            }
            scanned += 1;
            let guide = explain(model, &f.path, s.line);
            if guide.is_null() {
                continue;
            }
            if guide["truncated"] == true {
                limited = true;
            }
            for call in array(&guide["calls"]) {
                if array(&call["candidates"])
                    .iter()
                    .any(|c| c["path"] == path && c["line"] == line)
                {
                    let key = format!("{}:{}:{}", f.path, call["line"], call["name"]);
                    if !seen.insert(key) {
                        continue;
                    }
                    if found.len() >= max_callers {
                        limited = true;
                        continue;
                    }
                    found.push(json!({"path":f.path,"line":call["line"],"function":guide["name"],"function_line":guide["range"]["start"],"call":call["name"],"bindings":call["bindings"],"reason":call["reason"]}));
                }
            }
        }
    }
    json!({"callers":found,"scanned_functions":scanned,"limited":limited,"inventory_truncated":model.inventory_truncated,"max_functions":max_functions,"max_callers":max_callers,"unread_files":model.inventory.iter().filter(|e|super::snapshot::source(&e.path)&&!model.files.iter().any(|f|f.path==e.path)).count()})
}
pub fn array(v: &Value) -> &[Value] {
    v.as_array().map_or(&[], Vec::as_slice)
}
pub fn string(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}
pub fn number(v: &Value) -> usize {
    v.as_u64().unwrap_or(0) as usize
}
