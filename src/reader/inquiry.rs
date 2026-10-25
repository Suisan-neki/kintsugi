//! 質問・関数に関係する記述を集める。見つからない処理を補わない。
use super::{
    analysis::{File, Model, clip, mask},
    comprehension::{array, callers, explain, number, string},
    overview::{self, effects, enclosing, functions, line_ref, reference},
    snapshot::{self, Snapshot},
};
use serde_json::{Value, json};
use std::collections::HashSet;
pub fn options(model: &Model<'_>) -> Vec<Value> {
    let mut out = vec![];
    for f in model
        .files
        .iter()
        .filter(|f| f.path.ends_with(".py") && !snapshot::test_file(&f.path))
    {
        let clean = mask(&f.text, false, true);
        let lines = f.text.split('\n').collect::<Vec<_>>();
        for (i, m) in clean.split('\n').enumerate() {
            if !rx!(r"\.(add_option|add_argument)\s*\(").is_match(m) {
                continue;
            }
            let mut text = lines[i].to_owned();
            let mut j = i;
            while text.matches('(').count() > text.matches(')').count()
                && j < i + 6
                && j + 1 < lines.len()
            {
                j += 1;
                text.push(' ');
                text.push_str(lines[j].trim());
            }
            let flags = rx!(r#"['"](-{1,2}[\w-]+)['"]"#)
                .captures_iter(&text)
                .map(|c| c[1].to_owned())
                .collect::<Vec<_>>();
            let field = |name: &str| -> Option<String> {
                let p = model.budget.pattern(&format!(
                    r#"{name}\s*=\s*(['"])(.*?)['"]|{name}\s*=\s*([A-Za-z0-9_.]+)"#
                ));
                p.captures(&text)
                    .and_then(|c| c.get(2).or_else(|| c.get(3)))
                    .map(|m| m.as_str().to_owned())
            };
            let dest = field("dest").filter(|s| !s.is_empty()).or_else(|| {
                flags
                    .iter()
                    .find(|f| f.starts_with("--"))
                    .map(|s| s.trim_start_matches("--").replace('-', "_"))
            });
            out.push(json!({"flags":flags,"dest":dest,"default":field("default"),"help":field("help"),"metavar":field("metavar"),"action":field("action"),"path":f.path,"line":i+1,"refs":[reference(&f.path,i+1,j+1)]}));
        }
    }
    for f in model
        .files
        .iter()
        .filter(|f| rx!(r"\.[cm]?[jt]sx?$").is_match(&f.path) && !snapshot::test_file(&f.path))
    {
        for (i, l) in f.text.split('\n').enumerate() {
            if let Some(c) =
                rx!(r#"\.option\s*\(\s*['"]([^'"]+)['"]\s*(?:,\s*['"]([^'"]*)['"])?"#).captures(l)
            {
                out.push(json!({"flags":rx!(r"[ ,]+").split(&c[1]).filter(|s|s.starts_with('-')).collect::<Vec<_>>(),"dest":null,"default":null,"help":c.get(2).map(|m|m.as_str()).filter(|s|!s.is_empty()),"metavar":null,"action":null,"path":f.path,"line":i+1,"refs":[line_ref(&f.path,i+1)]}));
            }
        }
    }
    out
}
fn code(file: &File, line: usize) -> String {
    clip(file.line_text(line).unwrap_or("").trim(), 200)
}
fn parameters(signature: &str, python: bool) -> Vec<String> {
    let Some(c) = rx!(r"\(([\s\S]*)\)").captures(signature) else {
        return vec![];
    };
    let inner = &c[1];
    let mut depth = 0;
    let mut start = 0;
    let mut out = vec![];
    for (i, b) in inner.bytes().enumerate() {
        if b == b',' && depth == 0 {
            out.push(inner[start..i].trim().to_owned());
            start = i + 1;
        } else if b"([{<".contains(&b) {
            depth += 1;
        } else if b")]}>".contains(&b) {
            depth -= 1;
        }
    }
    out.push(inner[start..].trim().to_owned());
    out.retain(|s| !s.is_empty() && !(python && (s == "self" || s == "cls")));
    out
}
fn handlers(model: &Model<'_>, name: &str) -> Vec<Value> {
    let escaped = regex::escape(name);
    let pattern = model.budget.pattern(&format!(
        r"^\s*except\b[^:]*\b{escaped}\b|\bcatch\b.*\b{escaped}\b|instanceof\s+{escaped}\b"
    ));
    let mut found = vec![];
    for f in &model.files {
        let clean = mask(&f.text, f.path.ends_with(".rs"), f.path.ends_with(".py"));
        let masked = clean.split('\n').collect::<Vec<_>>();
        let original = f.text.split('\n').collect::<Vec<_>>();
        let fns = functions(f);
        for (i, m) in masked.iter().enumerate() {
            if pattern.is_match(m) {
                let fun = enclosing(&fns, i + 1);
                let mut body = vec![];
                for k in i + 1..original.len().min(i + 5) {
                    if rx!(r"^\s*(except|finally|else)\b").is_match(masked[k]) {
                        break;
                    }
                    if !original[k].trim().is_empty() {
                        body.push(json!({"line":k+1,"code":original[k].trim()}));
                    }
                }
                found.push(json!({"path":f.path,"line":i+1,"function":fun.map(|f|&f["name"]),"code":code(f,i+1),"refs":[reference(&f.path,i+1,body.last().map_or(i+1,|v|number(&v["line"])))],"body":body}));
            }
        }
    }
    found
}
fn test_mentions(model: &Model<'_>, names: &[String]) -> Vec<Value> {
    let patterns = names
        .iter()
        .filter(|n| n.len() > 1)
        .map(|n| {
            model.budget.pattern(&format!(
                r"(^|[^A-Za-z0-9_-]){}([^A-Za-z0-9_-]|$)",
                regex::escape(n)
            ))
        })
        .collect::<Vec<_>>();
    let mut hits = vec![];
    for f in model.files.iter().filter(|f| snapshot::test_file(&f.path)) {
        for (i, l) in f.text.split('\n').enumerate() {
            if patterns.iter().any(|p| p.is_match(l)) {
                hits.push(json!({"path":f.path,"line":i+1,"code":clip(l.trim(),200),"refs":[line_ref(&f.path,i+1)]}));
                if hits.len() == 12 {
                    return hits;
                }
            }
        }
    }
    hits
}
pub fn detail(model: &Model<'_>, path: &str, line: usize) -> Value {
    let Some(file) = model.file(path) else {
        return json!({"found":false,"reason":format!("{path} はまだ読み取っていない"),"needed_files":[path]});
    };
    let guide = explain(model, path, line);
    if guide.is_null() {
        return json!({"found":false,"reason":format!("{path} の{line}行目を含む関数を抽出できなかった。対応していない構文の可能性がある"),"needed_files":[]});
    }
    let start = number(&guide["range"]["start"]);
    let end = number(&guide["range"]["end"]);
    let fns = functions(file);
    let fun = fns.iter().find(|f| f["line"] == start);
    let name = fun.map_or(&guide["name"], |f| &f["name"]);
    let fx = effects(file)
        .into_iter()
        .filter(|e| number(&e["line"]) >= start && number(&e["line"]) <= end)
        .collect::<Vec<_>>();
    let clean = mask(&file.text, path.ends_with(".rs"), path.ends_with(".py"));
    let returns=array(&guide["steps"]).iter().filter(|s|s["kind"]=="return").map(|s|json!({"line":s["line"],"code":code(file,number(&s["line"])),"refs":[line_ref(path,number(&s["line"]))]})).collect::<Vec<_>>();
    let branches=array(&guide["steps"]).iter().filter(|s|s["kind"]=="branch"||s["kind"]=="loop").map(|s|json!({"kind":s["kind"],"line":s["line"],"code":code(file,number(&s["line"])),"refs":[line_ref(path,number(&s["line"]))]})).collect::<Vec<_>>();
    let mut raised = vec![];
    for (i, l) in clean.split('\n').enumerate().take(end).skip(start - 1) {
        if let Some(c) = rx!(r"\b(?:raise|throw\s+new|throw)\s+([A-Za-z_$][\w$.]*)").captures(l) {
            let exception = c[1].rsplit('.').next().expect("例外名");
            let h = handlers(model, exception);
            let handled = if h.is_empty() {
                json!({"kind":"code","text":format!("読み取った範囲に {exception} を捕捉する記述は見つからなかった。未読のファイルや、より広い例外の捕捉で扱われる可能性がある")})
            } else {
                let locations = h
                    .iter()
                    .map(|h| {
                        format!(
                            "{}（{} {}行目）",
                            h["function"].as_str().unwrap_or(string(&h["path"])),
                            string(&h["path"]),
                            number(&h["line"])
                        )
                    })
                    .collect::<Vec<_>>();
                json!({"kind":"inference","text":format!("{exception} を捕捉する記述が {} にある。この関数から実際にそこへ戻るかは、呼び出し関係の候補から推測した",locations.join("、"))})
            };
            raised.push(json!({"exception":exception,"line":i+1,"code":code(file,i+1),"refs":[line_ref(path,i+1)],"handlers":h,"handled":handled}));
        }
    }
    let caught = fx
        .iter()
        .filter(|e| e["kind"] == "catch")
        .map(|e| json!({"line":e["line"],"code":e["code"],"refs":e["refs"]}))
        .collect::<Vec<_>>();
    let mut callees = vec![];
    for c in array(&guide["calls"]) {
        if !callees.iter().any(|v: &Value| v["name"] == c["name"]) {
            callees.push(json!({"name":c["name"],"line":c["line"],"candidates":array(&c["candidates"]).iter().map(|x|{let mut x=x.clone();x["refs"]=json!([line_ref(string(&x["path"]),number(&x["line"]))]);x}).collect::<Vec<_>>(),"unread":c["unread"],"reason":c["reason"],"refs":[line_ref(path,number(&c["line"]))]}));
        }
    }
    let callers = callers(model, path, start, 400, 80);
    let mut name_callers = vec![];
    let owner = fun.and_then(|f| f["owner"].as_str());
    let short = fun.map_or(string(&guide["name"]), |f| string(&f["short"]));
    if owner.is_some() || array(&callers["callers"]).is_empty() {
        let constructor = owner.is_some() && short == "__init__";
        let pattern = model.budget.pattern(&if constructor {
            format!(
                r"(^|[^A-Za-z0-9_.]){}\s*\(",
                regex::escape(owner.expect("クラスの初期化"))
            )
        } else {
            format!(r"\.{}\s*\(", regex::escape(short))
        });
        for f in model.files.iter().filter(|f| !snapshot::test_file(&f.path)) {
            let m = mask(&f.text, f.path.ends_with(".rs"), f.path.ends_with(".py"));
            let fns = functions(f);
            for (i, l) in m.split('\n').enumerate() {
                if f.path == path && i + 1 >= start && i < end
                    || rx!(r"^\s*class\s|\bsys\.std(out|err)\.|\bconsole\.").is_match(l)
                {
                    continue;
                }
                if pattern.is_match(l) {
                    name_callers.push(json!({"path":f.path,"line":i+1,"function":enclosing(&fns,i+1).map(|f|&f["name"]),"code":code(f,i+1),"refs":[line_ref(&f.path,i+1)],"reason":if constructor{format!("クラス {} を作る記述。__init__ が呼ばれる候補",owner.expect("クラス名"))}else{format!("名前 {short} だけが一致する呼び出し。受け手の型は確かめていない")}}));
                }
            }
        }
    }
    let body = file
        .text
        .split('\n')
        .skip(start - 1)
        .take(end - start + 1)
        .collect::<Vec<_>>()
        .join("\n");
    let caller_text = name_callers
        .iter()
        .chain(array(&callers["callers"]))
        .filter_map(|c| {
            model
                .file(string(&c["path"]))
                .and_then(|f| f.line_text(number(&c["line"])))
        })
        .collect::<Vec<_>>()
        .join("\n");
    let text = format!("{body}\n{caller_text}");
    let related = options(model)
        .into_iter()
        .filter(|o| {
            o["dest"].as_str().is_some_and(|d| {
                !d.is_empty()
                    && model
                        .budget
                        .pattern(&format!(r"\b{}\b", regex::escape(d)))
                        .is_match(&text)
            })
        })
        .collect::<Vec<_>>();
    let mut names = vec![short.to_owned()];
    for o in &related {
        names.extend(array(&o["flags"]).iter().map(|v| string(v).to_owned()));
    }
    name_callers.truncate(20);
    json!({"found":true,"name":name,"path":path,"line":start,"end_line":end,"signature":guide["signature"],"parameters":parameters(string(&guide["signature"]),path.ends_with(".py")),"returns":returns,"branches":branches,"raised":raised,"caught":caught,"effects":fx.iter().map(|e|json!({"kind":e["kind"],"label":e["label"],"certainty":e["certainty"],"line":e["line"],"code":e["code"],"refs":e["refs"]})).collect::<Vec<_>>(),"callees":callees,"callers":array(&callers["callers"]).iter().map(|c|{let mut c=c.clone();c["refs"]=json!([line_ref(string(&c["path"]),number(&c["line"]))]);c}).collect::<Vec<_>>(),"name_callers":name_callers,"config":{"options":related,"env":fx.iter().filter(|e|e["kind"]=="env").collect::<Vec<_>>()},"tests":test_mentions(model,&names),"limits":guide["limits"],"refs":[reference(path,start,end)]})
}
struct Intent {
    id: &'static str,
    label: &'static str,
    kinds: &'static [&'static str],
    matched: bool,
}
fn intents(q: &str) -> Vec<Intent> {
    vec![
    Intent{id:"storage",label:"保存・書き込み",kinds:&["write","delete"],matched:rx!(r"(?i)保存|保管|書き込|書込|記録|格納|永続|残る|残す|データベース|\bDB\b|save|store|persist|write").is_match(q)},
    Intent{id:"failure",label:"失敗したときの扱い",kinds:&["raise","catch","exit","error_output"],matched:rx!(r"(?i)失敗|エラー|できなかった|できない|例外|異常|壊れ|落ち|止ま|error|fail|exception").is_match(q)},
    Intent{id:"send",label:"送信・通知",kinds:&["send"],matched:rx!(r"(?i)通知|送信|送る|メール|通信|連携|\bAPI\b|notify|send|mail").is_match(q)},
    Intent{id:"input",label:"入力",kinds:&["input","env"],matched:rx!(r"(?i)入力|引数|受け取|オプション|パラメータ|argument|input|option").is_match(q)},
    Intent{id:"read",label:"読み出し・表示",kinds:&["read","output"],matched:rx!(r"(?i)読み出|読む|読ん|表示|一覧|出力|見る|見え|read|list|show|print").is_match(q)},
    Intent{id:"start",label:"起動方法",kinds:&[],matched:rx!(r"(?i)起動|始め|使い方|実行方法|インストール|動かし|start|run|install|usage").is_match(q)},
    Intent{id:"test",label:"既存のテスト",kinds:&[],matched:rx!(r"(?i)テスト|試験|検証|\btest").is_match(q)},
    Intent{id:"config",label:"設定",kinds:&["env"],matched:rx!(r"(?i)設定|環境変数|config|setting").is_match(q)},
].into_iter().filter(|i|i.matched).collect()
}
fn terms(q: &str) -> Vec<String> {
    let mut out = vec![];
    for m in rx!(r"[A-Za-z_][\w.-]{2,}").find_iter(q) {
        let s = m.as_str();
        if !rx!(r"(?i)^(the|and|for|with|what|where|how|API|DB)$").is_match(s)
            && !out.iter().any(|x| x == s)
        {
            out.push(s.to_owned());
        }
    }
    for c in rx!(r#"[「『"]([^」』"]{1,40})[」』"]"#).captures_iter(q) {
        if !out.iter().any(|x| x == &c[1]) {
            out.push(c[1].to_owned());
        }
    }
    out.truncate(8);
    out
}
fn origin(file: &File, effect: &Value, fun: Option<&Value>) -> Option<Value> {
    let c=rx!(r"\bopen\s*\(\s*([A-Za-z_]\w*)|\b(?:writeFile|appendFile|createWriteStream)(?:Sync)?\s*\(\s*([A-Za-z_$][\w$]*)").captures(string(&effect["code"]))?;
    let variable = c.get(1).or_else(|| c.get(2))?.as_str();
    let fun = fun?;
    let pattern = file.budget.pattern(&format!(
        r"(^|[^A-Za-z0-9_.]){}\s*=[^=]|for\s+.*\b{}\b.*\bin\b",
        regex::escape(variable),
        regex::escape(variable)
    ));
    let lines = file.text.split('\n').collect::<Vec<_>>();
    for i in
        (number(&fun["line"]).saturating_sub(1)..number(&effect["line"]).saturating_sub(1)).rev()
    {
        if pattern.is_match(lines[i]) {
            return Some(
                json!({"variable":variable,"line":i+1,"code":clip(lines[i].trim(),200),"refs":[line_ref(&file.path,i+1)]}),
            );
        }
    }
    Some(json!({"variable":variable,"line":null,"code":null,"refs":[]}))
}
pub fn inquire(s: &Snapshot, m: &Model<'_>, question: &str, focus: Option<(&str, usize)>) -> Value {
    let q = question.trim();
    let intents = intents(q);
    let words = terms(q);
    let mut findings = vec![];
    let mut missing = vec![];
    let mut functions_rank: Vec<(String, usize, usize)> = vec![];
    let add_rank = |rank: &mut Vec<(String, usize, usize)>,
                    p: &str,
                    line: usize,
                    score: usize,
                    replace: bool| {
        if let Some((_, _, v)) = rank.iter_mut().find(|(a, b, _)| a == p && *b == line) {
            if replace {
                *v = score;
            } else {
                *v += score;
            }
        } else {
            rank.push((p.to_owned(), line, score));
        }
    };
    let sources = m
        .files
        .iter()
        .filter(|f| !snapshot::test_file(&f.path))
        .collect::<Vec<_>>();
    let all = sources.iter().flat_map(|f| effects(f)).collect::<Vec<_>>();
    for intent in &intents {
        if intent.kinds.is_empty() {
            continue;
        }
        let list = all
            .iter()
            .filter(|e| intent.kinds.contains(&string(&e["kind"])))
            .collect::<Vec<_>>();
        if list.is_empty() {
            missing.push(json!({"text":format!("読み取った {} 件のファイルに、「{}」に当たる記述は見つからなかった。読み取っていないファイルにある可能性がある",sources.len(),intent.label),"kind":"code","refs":[]}));
            continue;
        }
        let mut groups: Vec<(&str, usize, Vec<&Value>)> = vec![];
        for e in list {
            let p = string(&e["path"]);
            let line = number(&e["function_line"]);
            if let Some((_, _, v)) = groups.iter_mut().find(|(a, b, _)| *a == p && *b == line) {
                v.push(e);
            } else {
                groups.push((p, line, vec![e]));
            }
        }
        for (path, line, items) in groups {
            let first = items[0];
            let file = m
                .files
                .iter()
                .find(|f| f.path == path)
                .expect("effectの元ファイル");
            let fns = functions(file);
            let fun = fns.iter().find(|f| f["line"] == line);
            let strong = items.iter().filter(|e| e["certainty"] != "name").count();
            let mut labels = vec![];
            for e in &items {
                let l = string(&e["label"]);
                if !labels.contains(&l) {
                    labels.push(l);
                }
            }
            let parts = labels
                .iter()
                .map(|l| {
                    format!(
                        "{l}（{}行目）",
                        items
                            .iter()
                            .filter(|e| e["label"] == *l)
                            .map(|e| e["line"].to_string())
                            .collect::<Vec<_>>()
                            .join("・")
                    )
                })
                .collect::<Vec<_>>();
            let prefix = if let Some(f) = first["function"].as_str() {
                format!("{f}（{path}）")
            } else {
                path.to_owned()
            };
            let mut finding = json!({"intent":intent.id,"text":format!("{prefix} に、{}がある",parts.join("、")),"kind":if strong>0{"code"}else{"inference"},"refs":items.iter().take(8).map(|e|e["refs"][0].clone()).collect::<Vec<_>>(),"lines":items.iter().map(|e|json!({"line":e["line"],"code":e["code"],"label":e["label"],"refs":e["refs"]})).collect::<Vec<_>>()});
            if intent.id == "storage"
                && let Some(o) = items
                    .iter()
                    .find_map(|e| origin(file, e, fun).filter(|o| !o["line"].is_null()))
            {
                finding["storage_origin"] = json!({"text":format!("書き込み先の {} は、{}行目「{}」で作られている",string(&o["variable"]),number(&o["line"]),string(&o["code"])),"kind":"code","refs":o["refs"]});
            }
            findings.push(finding);
            if line > 0 {
                add_rank(
                    &mut functions_rank,
                    path,
                    line,
                    items.len() + if strong > 0 { 2 } else { 0 },
                    false,
                );
            }
        }
    }
    for word in &words {
        let mut hit = false;
        let lower = word.to_lowercase();
        for f in &m.files {
            if rx!(r"(?i)\.(php|twig|sql)$").is_match(&f.path)
                && f.path.to_lowercase().contains(&lower)
            {
                // 構文解析に未対応の本文も、実際に読んだ行をAIの根拠へ渡す。
                let count = f.text.lines().count();
                let refs: Vec<_> = (1..=count.min(240))
                    .step_by(80)
                    .map(|start| reference(&f.path, start, (start + 79).min(count)))
                    .collect();
                findings.push(json!({"intent":"path","text":format!("質問の「{word}」をパスに含む本文: {}。処理の意味と実行時の動作は未確認",f.path),"kind":"code","refs":refs}));
                hit = true;
            }
            for fun in functions(f)
                .iter()
                .filter(|fun| fun["short"] == *word || fun["name"] == *word)
            {
                hit = true;
                add_rank(
                    &mut functions_rank,
                    &f.path,
                    number(&fun["line"]),
                    100,
                    true,
                );
                findings.push(json!({"intent":"name","text":format!("質問の「{word}」と同じ名前の関数 {} が {} の{}行目にある",string(&fun["name"]),f.path,number(&fun["line"])),"kind":"code","refs":[reference(&f.path,number(&fun["line"]),number(&fun["end"]))]}));
            }
            if !hit {
                let lines = f
                    .text
                    .split('\n')
                    .enumerate()
                    .filter(|(_, l)| l.to_lowercase().contains(&lower))
                    .take(3)
                    .map(|(i, _)| i + 1)
                    .collect::<Vec<_>>();
                if !lines.is_empty() {
                    hit = true;
                    findings.push(json!({"intent":"word","text":format!("「{word}」を含む記述が {} にある（{}行目）",f.path,lines.iter().map(ToString::to_string).collect::<Vec<_>>().join("・")),"kind":"code","refs":lines.iter().map(|n|line_ref(&f.path,*n)).collect::<Vec<_>>()}));
                }
            }
        }
        if !hit {
            missing.push(json!({"text":format!("「{word}」を含む記述は、読み取った範囲に見つからなかった"),"kind":"code","refs":[]}));
        }
    }
    if let Some((path, line)) = focus
        && let Some(f) = m.files.iter().find(|f| f.path == path)
        && let Some(fun) = enclosing(&functions(f), line)
    {
        add_rank(&mut functions_rank, path, number(&fun["line"]), 1000, true);
    }
    functions_rank.sort_by_key(|a| std::cmp::Reverse(a.2));
    let details = functions_rank
        .iter()
        .take(3)
        .map(|(p, l, _)| detail(m, p, *l))
        .collect::<Vec<_>>();
    if intents.iter().any(|i| i.id == "failure") {
        for d in details.iter().filter(|d| d["found"] == true) {
            for r in array(&d["raised"]) {
                let mut refs = array(&r["refs"]).to_vec();
                refs.extend(
                    array(&r["handlers"])
                        .iter()
                        .flat_map(|h| array(&h["refs"]).to_vec()),
                );
                findings.push(json!({"intent":"failure","text":format!("{} は {}行目で {} を発生させる。{}",string(&d["name"]),number(&r["line"]),string(&r["exception"]),string(&r["handled"]["text"])),"kind":r["handled"]["kind"],"refs":refs}));
            }
        }
    }
    let mut paths = HashSet::new();
    for d in details.iter().filter(|d| d["found"] == true) {
        paths.insert(string(&d["path"]).to_owned());
    }
    for f in &findings {
        for r in array(&f["refs"]) {
            paths.insert(string(&r["path"]).to_owned());
        }
    }
    if paths.is_empty() {
        paths.extend(m.files.iter().map(|f| f.path.clone()));
    }
    let unread = s
        .inventory
        .iter()
        .filter(|e| {
            !s.files.contains_key(&e.path)
                && snapshot::readable(&e.path)
                && matches!(e.mode.as_str(), "100644" | "100755")
                && e.size.is_none_or(|n| n <= snapshot::MAX_FILE_BYTES)
        })
        .map(|o| o.path.as_str())
        .collect::<HashSet<_>>();
    let mut needed = vec![];
    let mut add = |p: &str| {
        if !needed.iter().any(|s| s == p) {
            needed.push(p.to_owned());
        }
    };
    // 固有名を含むパスを、一般的な「保存」等の候補より先に調べる。
    // inventoryも見るので、PHP等を除外していた旧読取記録からも補える。
    let mut named: Vec<_> = unread
        .iter()
        .filter(|p| {
            words
                .iter()
                .any(|w| p.to_lowercase().contains(&w.to_lowercase()))
        })
        .copied()
        .collect();
    named.sort_by_key(|p| {
        (
            snapshot::test_file(p),
            !words
                .iter()
                .any(|w| p.split('/').any(|part| part.eq_ignore_ascii_case(w))),
            !rx!(r"(?i)\.(php|twig|sql|rs|py|[cm]?[jt]sx?)$").is_match(p),
            !rx!(r"(?i)(^|/)(save|view|report|new)\.").is_match(p),
            p.to_string(),
        )
    });
    for p in named {
        add(p);
    }
    for e in &m.edges {
        if paths.contains(string(&e["from"])) && unread.contains(string(&e["to"])) {
            add(string(&e["to"]));
        }
    }
    for f in m.files.iter().filter(|f| paths.contains(&f.path)) {
        for b in &f.bindings {
            if let Some(t) = &b.target
                && unread.contains(t.as_str())
            {
                add(t);
            }
        }
    }
    for o in &s.omitted {
        if !unread.contains(o.path.as_str()) {
            continue;
        }
        if intents.iter().any(|i| match i.id {
            "storage" => rx!(r"(?i)(store|storage|save|db|database|repo|persist|model|file)")
                .is_match(&o.path),
            "send" => rx!(r"(?i)(notif|mail|send|client|api|http|webhook)").is_match(&o.path),
            "failure" => rx!(r"(?i)(error|except|fail)").is_match(&o.path),
            "config" => rx!(r"(?i)(config|setting|env)").is_match(&o.path),
            "test" => rx!(r"(?i)(test|spec)").is_match(&o.path),
            _ => false,
        }) {
            add(&o.path);
        }
    }
    for d in &details {
        if d["found"] != true {
            for p in array(&d["needed_files"]) {
                add(string(p));
            }
        }
    }
    needed.truncate(8);
    if intents.is_empty() && words.is_empty() && focus.is_none() {
        missing.push(json!({"text":"質問から、調べる観点（保存・失敗・送信・入力・表示・起動・テスト・設定）や関数名を読み取れなかった。関数名や「保存」「失敗」などの語を含めてください","kind":"code","refs":[]}));
    }
    let mut limits = vec![
        json!({"text":"コードに記述があることは、実行時にその処理が行われることを示さない。実際の動作は試験で確かめる","kind":"inference","refs":[]}),
    ];
    if !s.omitted.is_empty() {
        limits.push(json!({"text":format!("読み取っていないファイルが {} 件あり、そこに該当する処理がある可能性がある",s.omitted.len()),"kind":"code","refs":[]}));
    }
    findings.truncate(40);
    let mut result = json!({"schema_version":1,"question":q,"intents":intents.iter().map(|i|json!({"id":i.id,"label":i.label})).collect::<Vec<_>>(),"terms":words,"findings":findings,"functions":details,"missing":missing,"needed_files":needed,"unread_files":s.omitted.len(),"limits":limits,"effect_kinds":(["input","env","read","write","delete","send","output","error_output","raise","catch","exit"].iter().map(|k|(k.to_string(),json!(overview::effect_label(k)))).collect::<serde_json::Map<_,_>>())});
    if intents.iter().any(|i| i.id == "test") {
        result["tests"] = json!(overview::tests(s, m));
    }
    if intents.iter().any(|i| i.id == "config" || i.id == "input") {
        result["options"] = json!(options(m));
    }
    result
}
