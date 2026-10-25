//! GitDiagramの構成把握の方針を踏襲し、根拠付きの共通モデルを生成・保存する。
use crate::error::invalid;
mod context;
pub mod dialogue;
mod model;
#[cfg(test)]
mod tests;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, io, path::Path};
const PROMPT_VERSION: &str = "gitdiagram-adapted-ja-v2-yomiyasu";
const PROMPT: &str = include_str!("system_diagram/prompt.txt");

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub repository: String,
    pub commit: String,
    #[serde(default)]
    pub refresh: bool,
    #[serde(default)]
    pub revision: Option<String>,
}
fn read_cache(path: &Path) -> io::Result<Value> {
    if fs::metadata(path)?.len() > 2 * 1024 * 1024 {
        return Err(invalid("保存した図のサイズが上限を超えています"));
    }
    crate::recording::read_json(path)
}
pub fn generate(
    config: Option<&crate::ai::Config>,
    store: &crate::understanding::Store,
    request: Request,
) -> io::Result<Value> {
    let _guard = store.diagram_guard()?;
    let directory = store.diagram_directory(&request.repository, &request.commit)?;
    let cached = if let Some(revision) = &request.revision {
        if revision.len() != 64
            || !revision
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid("図の版が不正です"));
        }
        Some(directory.join(format!("{revision}.json")))
    } else {
        let current = directory.join("current.json");
        current.exists().then_some(current)
    };
    if let Some(path) = cached.filter(|_| !request.refresh) {
        let mut cached = read_cache(&path)?;
        if cached["source"]["repository"] != request.repository
            || cached["source"]["commit"] != request.commit
        {
            return Err(invalid("図と読取記録の版が一致しません"));
        }
        cached["cached"] = json!(true);
        return Ok(cached);
    }
    let config=config.ok_or_else(|| invalid("構成図の生成には運営者によるAI接続の設定が必要です。しばらくしてから再度お試しください"))?;
    let snapshot = store.architecture_source(&request.repository, &request.commit)?;
    let context = context::build(&snapshot)?;
    let mut messages =
        json!([{"role":"system","content":PROMPT},{"role":"user","content":context.text}]);
    let mut last_error = String::new();
    for attempt in 1..=2 {
        let response = config.complete(messages.clone())?;
        let content = response["content"]
            .as_str()
            .ok_or_else(|| invalid("AI応答の本文がありません"))?;
        match model::compile(content, &snapshot, &context) {
            Ok(mut graph) => {
                graph["model"] = response["model"].clone();
                graph["generation_attempts"] = json!(attempt);
                let references = crate::understanding::verify_refs(&mut graph, &snapshot);
                if references["unverified"]
                    .as_array()
                    .is_none_or(|a| !a.is_empty())
                {
                    return Err(invalid("生成した図の参照行を照合できませんでした"));
                }
                let output = json!({"schema_version":1,"source":crate::understanding::source_summary(&snapshot),"result":graph,"references":references,"cached":false});
                fs::create_dir_all(&directory)?;
                let revision = output["result"]["model_revision"]
                    .as_str()
                    .ok_or_else(|| invalid("図の版がありません"))?;
                let saved = directory.join(format!("{revision}.json"));
                // 成功した版は上書きしない。失敗した再生成も以前の図を残す。
                let output = if saved.exists() {
                    read_cache(&saved)?
                } else {
                    crate::recording::write_json_new(&saved, &output)?;
                    output
                };
                let temporary = directory.join(format!("current-{}.pending", std::process::id()));
                fs::write(
                    &temporary,
                    serde_json::to_vec(&output).map_err(io::Error::other)?,
                )?;
                fs::rename(temporary, directory.join("current.json"))?;
                return Ok(output);
            }
            Err(error) => {
                last_error = error.to_string();
                if let Some(items) = messages.as_array_mut() {
                    items.push(json!({"role":"assistant","content":content}));
                    items.push(json!({"role":"user","content":format!("構造・参照の検査に失敗しました。次を修正し、同じJSON形式で返してください。新しい根拠を作らず、元の抜粋だけを使ってください。\n{last_error}")}));
                }
            }
        }
    }
    Err(invalid(format!(
        "AIの構成データを検査できませんでした。図は更新していません。{last_error}"
    )))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Question {
    repository: String,
    commit: String,
    revision: String,
    object_id: Option<String>,
    question: String,
}
pub fn answer(
    config: Option<&crate::ai::Config>,
    store: &crate::understanding::Store,
    request: Question,
) -> io::Result<Value> {
    let config = config
        .ok_or_else(|| invalid("AIはまだ利用できません。運営者によるサーバー設定が必要です。"))?;
    if request.question.trim().is_empty() || request.question.len() > 6000 {
        return Err(invalid("疑問は空でない6000バイト以内で入力してください"));
    }
    let cached = generate(
        None,
        store,
        Request {
            repository: request.repository.clone(),
            commit: request.commit.clone(),
            refresh: false,
            revision: Some(request.revision.clone()),
        },
    )?;
    let graph = &cached["result"];
    let nodes = graph["nodes"]
        .as_array()
        .ok_or_else(|| invalid("保存した図に構成要素がありません"))?;
    let edges = graph["edges"]
        .as_array()
        .ok_or_else(|| invalid("保存した図に接続がありません"))?;
    let (label, mut inquiry) = if let Some(object) = &request.object_id {
        let selected = nodes
            .iter()
            .chain(edges)
            .find(|n| n["id"] == *object)
            .ok_or_else(|| invalid("指定した部分はこの図にありません"))?;
        let related: Vec<_> = edges
            .iter()
            .filter(|e| e["from"] == *object || e["to"] == *object || e["id"] == *object)
            .collect();
        let peers: Vec<_> = nodes
            .iter()
            .filter(|n| {
                related
                    .iter()
                    .any(|e| e["from"] == n["id"] || e["to"] == n["id"])
            })
            .collect();
        (
            selected["title"]
                .as_str()
                .unwrap_or("選んだ部分")
                .to_owned(),
            json!({"selected":selected,"connections":related,"peers":peers,"overview":graph["overview"]}),
        )
    } else {
        ("システム全体".into(), graph.clone())
    };
    let snapshot = store.load(&request.repository, &request.commit)?;
    crate::understanding::verify_refs(&mut inquiry, &snapshot);
    let mut answer = crate::ai::explain_references(
        config,
        &snapshot,
        &inquiry,
        crate::ai::AiRequest {
            repository: request.repository,
            commit: request.commit,
            question: format!(
                "構成図では対象を「{label}」と表現しています。この名称もAIの解釈です。利用者の疑問：{}",
                request.question
            ),
            path: None,
            line: None,
        },
    )?;
    answer["diagram_revision"] = json!(request.revision);
    answer["object_id"] = json!(request.object_id);
    Ok(answer)
}
