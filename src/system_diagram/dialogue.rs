//! 図の固定版に対応する対話と整理案。試験結果・人の確認とは別に保存する。
use super::*;
use std::io::Write;

mod dify;
pub use dify::Config as DifyConfig;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub repository: String,
    pub commit: String,
    pub revision: String,
    pub object_id: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub binding: Binding,
    pub expected_turn: usize,
    pub audience: String,
    pub question: String,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Reference {
    path: String,
    line: u32,
    end_line: u32,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Claim {
    text: String,
    refs: Vec<Reference>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Step {
    operation: String,
    expected: String,
    observe: String,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Record {
    concern: String,
    actor: String,
    operation: String,
    expectation: String,
    related_object_ids: Vec<String>,
    claims: Vec<Claim>,
    unknowns: Vec<String>,
    test_plan: Vec<Step>,
    reported_observations: Vec<String>,
    improvement_ideas: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    request_id: String,
    record: Record,
}
#[derive(Serialize, Deserialize)]
struct Turn {
    request_id: String,
    question: String,
    audience: String,
    answer: String,
    record: Record,
    claims: Value,
    results: Value,
    #[serde(default)]
    writing_policy: Option<Value>,
    #[serde(default)]
    source_read: Option<Value>,
    #[serde(default)]
    context_digest: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct Thread {
    schema_version: u32,
    id: String,
    binding: Binding,
    backend: String,
    remote_id: Option<String>,
    turns: Vec<Turn>,
}
fn thread_id(binding: &Binding) -> io::Result<String> {
    Ok(digest(
        &serde_json::to_vec(binding).map_err(io::Error::other)?,
    ))
}
fn location(
    store: &crate::understanding::Store,
    binding: &Binding,
) -> io::Result<std::path::PathBuf> {
    Ok(store
        .diagram_directory(&binding.repository, &binding.commit)?
        .join("dialogues")
        .join(format!("{}.json", thread_id(binding)?)))
}
fn graph(store: &crate::understanding::Store, binding: &Binding) -> io::Result<Value> {
    let cached = super::generate(
        None,
        store,
        super::Request {
            repository: binding.repository.clone(),
            commit: binding.commit.clone(),
            revision: Some(binding.revision.clone()),
            refresh: false,
        },
    )?;
    if let Some(id) = &binding.object_id
        && !cached["result"]["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .chain(cached["result"]["edges"].as_array().into_iter().flatten())
            .any(|v| v["id"] == *id)
    {
        return Err(invalid("指定した部分はこの図にありません"));
    }
    Ok(cached["result"].clone())
}
fn load(path: &Path, binding: &Binding) -> io::Result<Thread> {
    if !path.exists() {
        return Ok(Thread {
            schema_version: 1,
            id: thread_id(binding)?,
            binding: binding.clone(),
            backend: String::new(),
            remote_id: None,
            turns: vec![],
        });
    }
    let saved = read_cache(path)?;
    let thread: Thread =
        serde_json::from_value(saved).map_err(|_| invalid("保存した対話を読み取れません"))?;
    if thread.schema_version != 1 || thread.binding != *binding || thread.id != thread_id(binding)?
    {
        return Err(invalid("対話と構成図の対象・版が一致しません"));
    }
    Ok(thread)
}
fn public(thread: &Thread) -> Value {
    json!({"status":"ok", "thread_id":thread.id, "binding":thread.binding, "turn":thread.turns.len(), "history":thread.turns,
        "record":thread.turns.last().map(|t| &t.record), "claims":thread.turns.last().map(|t| &t.claims),
        "answer":thread.turns.last().map(|t| &t.answer), "backend":if thread.backend.starts_with("dify:") {"dify"} else {"native"}, "record_status":"ai_draft"})
}
pub fn read(store: &crate::understanding::Store, binding: Binding) -> io::Result<Value> {
    graph(store, &binding)?;
    let _guard = store.diagram_guard()?;
    Ok(public(&load(&location(store, &binding)?, &binding)?))
}
fn save(path: &Path, thread: &Thread) -> io::Result<()> {
    let bytes = serde_json::to_vec(thread).map_err(io::Error::other)?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err(invalid("対話の保存上限に達しました"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| invalid("対話の保存先がありません"))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!("{}-{}.pending", thread.id, std::process::id()));
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::rename(temporary, path)
}
fn text(value: &str, limit: usize, empty: bool) -> bool {
    (empty || !value.trim().is_empty()) && value.chars().count() <= limit
}
fn strings(values: &[String]) -> bool {
    values.len() <= 12 && values.iter().all(|s| text(s, 2000, false))
}
fn assess(
    envelope: &str,
    request_id: &str,
    graph: &Value,
    snapshot: &Value,
    provided: &crate::ai_response::ProvidedLines,
) -> io::Result<(Record, Value)> {
    if envelope.len() > 96 * 1024 {
        return Err(invalid("AIの整理案が上限を超えています"));
    }
    let output: Envelope = serde_json::from_str(envelope)
        .map_err(|_| invalid("AIの整理案を所定のJSONとして読めません"))?;
    let record = output.record;
    if output.request_id != request_id
        || [
            &record.concern,
            &record.actor,
            &record.operation,
            &record.expectation,
        ]
        .iter()
        .any(|s| !text(s, 2000, true))
        || record.related_object_ids.len() > 24
        || record.claims.len() > 16
        || !strings(&record.unknowns)
        || !strings(&record.reported_observations)
        || !strings(&record.improvement_ideas)
        || record.test_plan.len() > 12
        || record.test_plan.iter().any(|s| {
            [&s.operation, &s.expected, &s.observe]
                .iter()
                .any(|s| !text(s, 2000, false))
        })
    {
        return Err(invalid("AIの整理案が今回の対話または上限と一致しません"));
    }
    for id in &record.related_object_ids {
        if !graph["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .chain(graph["edges"].as_array().into_iter().flatten())
            .any(|v| v["id"] == *id)
        {
            return Err(invalid("AIがこの図にない要素を関連箇所として返しました"));
        }
    }
    let evidence = crate::ai_response::assess(
        &json!({"answer":"整理案の根拠", "claims":record.claims, "unknowns":record.unknowns})
            .to_string(),
        snapshot,
        provided,
    )?;
    if evidence["unverified_references"] != 0
        || evidence["references_outside_context"] != 0
        || evidence["claims"]
            .as_array()
            .is_some_and(|v| v.iter().any(|v| v["references_in_context"] != true))
    {
        return Err(invalid(
            "整理案の根拠が今回渡したコードの行と一致しません。対話は更新していません",
        ));
    }
    Ok((record, evidence["claims"].clone()))
}
fn results(directory: &Path, binding: &Binding, id: &str) -> io::Result<Value> {
    let workspace = crate::workspace::load(directory)?;
    let mut ids = vec![];
    for statement in &workspace.statements {
        let Ok(origin) = serde_json::from_str::<Value>(&statement.origin) else {
            continue;
        };
        if origin["dialogue_id"] == id
            && origin["repository"] == binding.repository
            && origin["commit"] == binding.commit
            && origin["model_revision"] == binding.revision
        {
            ids.push(format!("repository-concern:{}", statement.id));
        }
    }
    let selected: Vec<_> = workspace
        .checks
        .iter()
        .filter(|c| ids.contains(&c.origin))
        .map(|c| (c.id.clone(), c.text.clone(), c.source.clone()))
        .collect();
    let view = crate::workspace::view(workspace);
    let records: Vec<_> = selected
        .iter()
        .take(8)
        .filter_map(|(id, condition, source)| {
            view.checks
                .iter()
                .find(|c| c.check_id == *id)
                .map(|c| json!({"condition":condition,"source":source,"result":c}))
        })
        .collect();
    let value =
        json!({"checks":records,"additional_checks_omitted":selected.len().saturating_sub(8)});
    if value.to_string().len() > 48 * 1024 {
        return Ok(
            json!({"checks":[],"unavailable":"結果が大きいため会話へは渡していません。検証画面で確認してください"}),
        );
    }
    Ok(value)
}
pub fn send(
    ai: Option<&crate::ai::Config>,
    dify: Option<&DifyConfig>,
    store: &crate::understanding::Store,
    directory: &Path,
    message: Message,
) -> io::Result<Value> {
    if !matches!(message.audience.as_str(), "operator" | "developer")
        || !text(&message.question, 2000, false)
        || message.question.len() > 6000
    {
        return Err(invalid(
            "立場と、空でない2000文字・6000バイト以内の疑問を指定してください",
        ));
    }
    let graph = graph(store, &message.binding)?;
    let inquiry_result = store.handle(crate::understanding::UnderstandRequest::Inquire {
        repository: message.binding.repository.clone(),
        commit: message.binding.commit.clone(),
        question: message.question.clone(),
        path: None,
        line: None,
    })?;
    let snapshot = store.load(&message.binding.repository, &message.binding.commit)?;
    let source_read = json!({"loaded_files":inquiry_result["result"]["loaded_files"],
        "failed_files":inquiry_result["result"]["failed_files"],
        "needed_files":inquiry_result["result"]["needed_files"]});
    // 質問の根拠を先に入れ、図だけで48KiBを使い切らないようにする。
    let mut inquiry = json!({"a_question":inquiry_result["result"],"z_graph":graph});
    crate::understanding::verify_refs(&mut inquiry, &snapshot);
    let (code, provided) = crate::ai::excerpts(&snapshot, &inquiry, 48 * 1024);
    let context = json!({"repository":message.binding.repository,"commit":message.binding.commit,"model_revision":message.binding.revision,"selected_element":message.binding.object_id,"architecture":graph,"code_excerpts":code}).to_string();
    let context_digest = digest(context.as_bytes());
    if context.len() > 128 * 1024 {
        return Err(invalid("図とコードの文脈が128KiBを超えています"));
    }
    let _guard = store.diagram_guard()?;
    let path = location(store, &message.binding)?;
    let mut thread = load(&path, &message.binding)?;
    if thread.turns.len() != message.expected_turn {
        return Err(invalid(
            "別の操作で対話が更新されました。会話を読み直してから送信してください",
        ));
    }
    if thread.turns.len() >= 40 {
        return Err(invalid(
            "この対話は40往復に達しました。検証内容を記録してから次の疑問を整理してください",
        ));
    }
    let request_id =
        digest(format!("{}:{}:{}", thread.id, thread.turns.len(), message.question).as_bytes());
    let results = results(directory, &message.binding, &thread.id)?;
    let history: Vec<_> = thread
        .turns
        .iter()
        .rev()
        .take(6)
        .rev()
        .map(|t| json!({"question":t.question,"answer":t.answer}))
        .collect();
    let query = json!({"request_id":request_id,"audience":if message.audience=="developer" {"技術者"}else{"非技術者"},"message":message.question,"previous_record":thread.turns.last().map(|t| &t.record),"history":history,"results":results,"source_read":source_read,"writing_policy":crate::writing::policy(),"writing_policy_source":crate::writing::metadata()});
    let (answer, envelope, remote_id, backend) = if let Some(config) = dify {
        // Difyの初回system_contextを、追加読取後も使い続けない。
        // ローカルの履歴と整理案はqueryで引き継ぐ。
        let remote = if thread.backend == config.identity()
            && thread
                .turns
                .last()
                .and_then(|t| t.context_digest.as_deref())
                == Some(&context_digest)
        {
            thread.remote_id.as_deref()
        } else {
            None
        };
        let (answer, envelope, remote_id) = config.chat(&thread.id, remote, &context, &query)?;
        (answer, envelope, Some(remote_id), config.identity())
    } else {
        let config = ai.ok_or_else(|| invalid("対話には運営者によるAI接続の設定が必要です"))?;
        let envelope = config.complete(json!([
            {"role":"system","content":include_str!("dialogue_record.txt")},
            {"role":"user","content":format!("調査資料:\n{context}\n今回の入力:\n{query}")}
        ]))?["content"]
            .as_str()
            .ok_or_else(|| invalid("整理案の本文がありません"))?
            .to_owned();
        // 自然文を生成する前に構造と根拠を検査する。
        assess(&envelope, &request_id, &graph, &snapshot, &provided)?;
        let reply = config.complete(json!([
            {"role":"system","content":include_str!("dialogue_reply.txt")},
            {"role":"user","content":json!({"context":context,"query":query,"record":envelope,"history":history}).to_string()}
        ]))?;
        let reply: Value = serde_json::from_str(reply["content"].as_str().unwrap_or_default())
            .map_err(|_| invalid("返答を所定のJSONとして読めません"))?;
        let answer = reply["answer"]
            .as_str()
            .ok_or_else(|| invalid("返答の本文がありません"))?
            .to_owned();
        (answer, envelope, None, "native".into())
    };
    let (record, claims) = assess(&envelope, &request_id, &graph, &snapshot, &provided)?;
    if !text(&answer, 8000, false) {
        return Err(invalid("AIの返答が上限を満たしていません"));
    }
    thread.backend = backend;
    thread.remote_id = remote_id;
    thread.turns.push(Turn {
        request_id,
        question: message.question,
        audience: message.audience,
        answer,
        record,
        claims,
        results,
        writing_policy: Some(crate::writing::metadata()),
        source_read: Some(source_read),
        context_digest: Some(context_digest),
    });
    save(&path, &thread)?;
    Ok(public(&thread))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn envelope() -> Value {
        json!({"request_id":"turn-1","record":{"concern":"保存後に残るか不安","actor":"私","operation":"保存して開き直す","expectation":"同じ内容を読める","related_object_ids":["screen","store"],"claims":[{"text":"保存を呼び出す記述がある","refs":[{"path":"app.py","line":1,"end_line":1}]}],"unknowns":["再読込は未検証"],"test_plan":[{"operation":"保存して開き直す","expected":"同じ内容を読める","observe":"表示と本文"}],"reported_observations":[],"improvement_ideas":[]}})
    }
    #[test]
    fn record_requires_current_turn_existing_objects_and_only_provided_lines() {
        let snapshot = json!({"fullName":"example/notes","commit":"a".repeat(40),"files":{"app.py":"save(note)\nread(note)\n"}});
        let graph = json!({"nodes":[{"id":"screen"},{"id":"store"}],"edges":[]});
        let provided = [("app.py".into(), 1)].into();
        let valid = envelope();
        let (record, claims) =
            assess(&valid.to_string(), "turn-1", &graph, &snapshot, &provided).unwrap();
        assert_eq!(record.actor, "私");
        assert_eq!(record.unknowns, vec!["再読込は未検証"]);
        assert_eq!(claims[0]["references_in_context"], true);
        for mutation in 0..5 {
            let mut invalid = valid.clone();
            match mutation {
                0 => invalid["request_id"] = json!("previous-turn"),
                1 => invalid["record"]["related_object_ids"] = json!(["invented_database"]),
                2 => invalid["record"]["claims"][0]["refs"][0]["line"] = json!(2),
                3 => invalid["record"]["claims"][0]["refs"][0]["verified"] = json!(true),
                _ => invalid["record"]["verdict"] = json!("satisfied"),
            }
            assert!(
                assess(&invalid.to_string(), "turn-1", &graph, &snapshot, &provided).is_err(),
                "mutation {mutation}"
            );
        }
    }
    #[test]
    fn conversation_identity_changes_with_repository_code_diagram_and_selection() {
        let base = Binding {
            repository: "example/notes".into(),
            commit: "a".repeat(40),
            revision: "b".repeat(64),
            object_id: Some("store".into()),
        };
        let id = thread_id(&base).unwrap();
        for n in 0..4 {
            let mut other = base.clone();
            match n {
                0 => other.repository = "example/other".into(),
                1 => other.commit = "c".repeat(40),
                2 => other.revision = "d".repeat(64),
                _ => other.object_id = None,
            }
            assert_ne!(thread_id(&other).unwrap(), id);
        }
    }
}
