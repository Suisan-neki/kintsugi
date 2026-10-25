// 参照アプリの実コードを固定し、表示の誤り→修正→再試験→故障→未観測→復旧を通す。
import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { mkdir, readFile, writeFile, rm } from 'node:fs/promises';
import { resolve, join, dirname } from 'node:path';
const root = resolve(import.meta.dirname, '..');
const binary = process.env.REVIEW_BINARY || join(root, 'target/debug/mugiurushi');
const output = resolve(process.argv[2] || join(root, 'evaluations', 'queue-' + Date.now()));
await mkdir(dirname(output), { recursive: true }); await mkdir(output);
const repository = join(output, 'repo'), casePath = join(output, 'case');
await mkdir(join(repository, 'examples/queue-display'), { recursive: true });
await mkdir(join(repository, 'adapters/observation'), { recursive: true });
for (const path of ['examples/queue-display/app.py', 'examples/queue-display/trial.py', 'examples/queue-display/readback.py', 'adapters/observation/observe.py']) await writeFile(join(repository, path), await readFile(join(root, path)));
const rawSentinel = 'SYNTHETIC_RAW_LOG_MUST_STAY_LOCAL';
await writeFile(join(repository, 'examples/queue-display/trial.py'), (await readFile(join(repository, 'examples/queue-display/trial.py'), 'utf8')) + `\nprint('${rawSentinel}')\n`);
const git = (...args) => execFileSync('git', ['-C', repository, ...args], { encoding: 'utf8' }).trim();
const run = (...args) => JSON.parse(execFileSync(binary, args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }));
git('init', '-q'); git('config', 'user.name', 'Synthetic test'); git('config', 'user.email', 'test@example.invalid');
git('remote', 'add', 'origin', 'https://github.com/example/queue-display.git');
const appPath = 'examples/queue-display/app.py', fixed = await readFile(join(repository, appPath), 'utf8');
const broken = fixed.replace('DISPLAY_OFFSET = 0', 'DISPLAY_OFFSET = 1'); assert.notEqual(broken, fixed);
await writeFile(join(repository, appPath), broken); git('add', '.'); git('commit', '-qm', 'synthetic wrong display');
const beforeCommit = git('rev-parse', 'HEAD');
let source = { repository: 'example/queue-display', commit: beforeCommit, path: appPath, line: 1, end_line: 8 };
let state = run('case', 'init', casePath, '--template', 'general', '--id', 'queue-display', '--title', '架空の呼び出し番号を確認する', '--actor', '合成の作成者', '--reason', '参照アプリの試行');
async function change(action, reason = '参照アプリの確認') {
  await writeFile(join(output, 'change.json'), JSON.stringify({ expected_revision: state.workspace.revision, actor: '合成の作成者', reason, action }));
  state = run('case', 'apply', casePath, join(output, 'change.json')); return state;
}
await change({ op: 'select_profile', domain: 'medical' });
await change({ op: 'add_profile_check', template_id: 'medical-access', selection_version: 1 });
await change({ op: 'add_check', check: { id: 'queue', version: 1, text: '入力した架空の番号42がHTTPの表示と再起動後の表示に一致し、保存物を別プロセスで読み出せる', origin: '参照アプリの合成条件。実際の担当者の要件ではない', source, observation: null, rule: 'manual_only', dependencies: [], questions: [] } });
const criterion = { assertion: 'queue_matches_input', subject: 'display-1' };
await change({ op: 'configure_observation', check_id: 'queue', criterion });
const queueModel = JSON.parse(await readFile(join(root, 'examples/queue-display/architecture.json'), 'utf8'));
function model(version, source) {
  const result = structuredClone(queueModel);
  result.version = version; result.source = source;
  for (const item of [...result.elements, ...result.relations]) {
    if (item.basis.source) item.basis.source = { ...item.basis.source, repository: source.repository, commit: source.commit };
  }
  return result;
}
await change({ op: 'add_architecture_model', model: model(1, source) });

await change({ op: 'add_statement', statement: { id: 'queue-question', kind: 'unanswered', role: 'operator', text: '保存した番号と表示した番号が違う場合に気づけるか', origin: '参照アプリの合成の疑問。実参加者の発言ではない', check_ids: ['queue'], source: null, evidence_id: null } });
await change({ op: 'add_architecture_link', link: { id: 'queue-question-link', model_id: 'queue-reference', model_version: 1, object_id: 'request', target: { kind: 'statement', statement_id: 'queue-question' } } });
// 解析には実際に読んだ固定版のファイルだけを渡す。対象を解析経路で実行しない。
const tokenPath = join(output, 'api-token');
const apiServer = spawn(binary, ['api', casePath, '--port', '0', '--token-file', tokenPath], { stdio: ['ignore', 'pipe', 'pipe'] });
try {
  const url = await new Promise((accept, reject) => {
    let text = ''; const timer = setTimeout(() => reject(Error('reader API timeout')), 10000);
    apiServer.stdout.on('data', data => { text += data; const match = /http:\/\/127\.0\.0\.1:\d+/.exec(text); if (match) { clearTimeout(timer); accept(match[0]); } });
    apiServer.once('exit', () => { clearTimeout(timer); reject(Error('reader API exited')); });
  });
  const content = await readFile(join(repository, appPath), 'utf8');
  const line = content.split('\n').findIndex(line => line.trim().startsWith('def call(')) + 1;
  assert.ok(line > 0);
  const snapshot = { schema: 1, fullName: source.repository, commit: beforeCommit, reference: beforeCommit, truncated: false, files: { [appPath]: content }, inventory: [{ path: appPath, size: Buffer.byteLength(content), mode: '100644', sha: git('rev-parse', beforeCommit + ':' + appPath) }], omitted: [] };
  const snapshotDirectory=join(casePath,'snapshots',snapshot.fullName);await mkdir(snapshotDirectory,{recursive:true});await writeFile(join(snapshotDirectory,snapshot.commit+'.json'),JSON.stringify(snapshot));
  const response = await fetch(url + '/api/v1/understand', { method: 'POST', headers: { 'X-Review-Token': await readFile(tokenPath, 'utf8'), 'Content-Type': 'application/json' }, body: JSON.stringify({ operation: 'analyze', repository: snapshot.fullName, commit: snapshot.commit, path: appPath, line }) });
  const analysis = await response.json(); assert.equal(response.status, 200, JSON.stringify(analysis));
  assert.ok(JSON.stringify(analysis).includes('call'));
  await writeFile(join(output, 'understanding.json'), JSON.stringify(analysis, null, 2));
  await change({ op: 'add_statement', statement: { id: 'queue-explanation', kind: 'implementation', role: 'developer', text: '固定版の保存処理とHTTP表示を分けて試験する。コードの解析だけでは実際の表示は確認できない', origin: '固定版コードの静的解析と、参照アプリの試験設定による合成の説明', check_ids: ['queue'], source: { ...source, line, end_line: line }, evidence_id: null } });
} finally { apiServer.kill('SIGTERM'); await rm(tokenPath, { force: true }); }


const trialCheck = (id, scope, method, text) => ({ id, scope, method, expect: 'contains', text });
const recipe = { schema_version: 1, check_id: 'queue', source, runtime: 'python3', entrypoint: 'examples/queue-display/trial.py', args: [], input: { number: 42, test_id: '{{test_value}}' }, criterion, timeout_ms: 8000,
  trial: { schema_version: 1, fault: 'normal', storage: 'stored.json', readback: { entrypoint: 'examples/queue-display/readback.py', args: ['stored.json'] }, checks: [
    trialCheck('display', 'HTTPで返す表示番号', 'stdout', 'DISPLAYED=42'),
    trialCheck('stored', '試行用の保存物', 'stored_file', '{{test_value}}'),
    trialCheck('readback', '別プロセスの読み出し', 'readback_output', 'STORED=42'),
    trialCheck('restart', '同じ保存物から再起動後の表示', 'stdout', 'RESTARTED=42'),
  ] } };
const view = () => state.checks.find(c => c.check_id === 'queue');
async function execute(fault) {
  recipe.trial.fault = fault;
  await writeFile(join(output, 'recipe.json'), JSON.stringify(recipe));
  state = run('case', 'execute', casePath, join(output, 'recipe.json'), '--repository-root', repository, '--mode', 'local', '--actor', '合成の作成者', '--reason', fault + 'の試行');
  const evidence = state.workspace.evidence.at(-1);
  const modelVersion = source.commit === beforeCommit ? 1 : 2;
  for (const [observation_id, object_id] of [['display', 'request'], ['stored', 'storage'], ['readback', 'persist'], ['restart', 'app']]) {
    await change({ op: 'add_architecture_link', link: { id: evidence.id + '-' + observation_id, model_id: 'queue-reference', model_version: modelVersion, object_id,
      target: { kind: 'evidence', evidence_id: evidence.id, observation_id } } });
  }
  return view();
}
const wrong = await execute('normal'); assert.equal(wrong.verdict, 'not_satisfied');
const parts = state.workspace.evidence.at(-1).execution.trial.results;
assert.equal(parts.find(x => x.id === 'stored').observed, 'pass');
assert.equal(parts.find(x => x.id === 'display').observed, 'fail');
await change({ op: 'decide', role: 'developer', action: 'modify', check_ids: ['queue'], change_reference: '表示時に1を足す誤りを修正する', confidence: null, elapsed_ms: null });
await writeFile(join(repository, appPath), fixed); git('add', '.'); git('commit', '-qm', 'synthetic correct display');
const afterCommit = git('rev-parse', 'HEAD');
source = { ...source, commit: afterCommit }; recipe.source = source;
await change({ op: 'bind_source', check_id: 'queue', source }); assert.equal(view().verdict, 'unknown');
assert.equal(state.workspace.decisions[0].basis[0].verdict, 'not_satisfied');
await change({ op: 'add_architecture_model', model: model(2, source) });
const results = {};
for (const [fault, expected] of [['normal', 'satisfied'], ['storage_is_directory', 'not_satisfied'], ['skip_state_read', 'unknown'], ['normal', 'satisfied']]) {
  const result = await execute(fault); assert.equal(result.verdict, expected, JSON.stringify(result));
  results[fault] = result.verdict;
}
assert.equal(state.checks.find(c => c.check_id === 'profile-medical-access').verdict, 'unknown');
assert.ok(state.workspace.evidence.every(e => e.review_profile?.domain === 'medical'));
const evidence = JSON.stringify(state.workspace.evidence);
await change({ op: 'select_profile', domain: 'general' });
assert.equal(JSON.stringify(state.workspace.evidence), evidence);
assert.equal(view().verdict, 'satisfied');
const jsonPath = join(output, 'review.json'), htmlPath = join(output, 'review.html');
execFileSync(binary, ['case', 'report', casePath, '--output', jsonPath]);
execFileSync(binary, ['case', 'report', casePath, '--output', htmlPath, '--format', 'html']);
const report = JSON.parse(await readFile(jsonPath, 'utf8'));
assert.equal(report.domain, 'general');
assert.ok(!JSON.stringify(report).includes(rawSentinel));
assert.ok(!(await readFile(htmlPath, 'utf8')).includes(rawSentinel));
assert.equal(report.architecture_models.length, 2);
assert.equal(report.architecture_links.length, 21);
assert.equal(report.statements.length, 2);
assert.equal(report.checks.find(c => c.definition.id === 'profile-medical-access').current.verdict, 'unknown');
const reference = report.checks.find(c => c.definition.id === 'queue');
assert.equal(reference.current.evidence[0].observed.verdict, 'not_satisfied');
assert.equal(reference.current.evidence[0].applicable, false);
assert.equal(reference.references[0].source.commit, beforeCommit);
assert.equal(reference.references.at(-1).source.commit, afterCommit);
assert.equal(reference.references[0].observations.find(o => o.id === 'stored').observed, 'pass');
assert.equal(reference.references[0].observations.find(o => o.id === 'display').observed, 'fail');
assert.ok(reference.current.evidence.every(e => e.summary === null));
assert.ok(report.decisions.every(d => d.basis.every(c => c.evidence.every(e => e.summary === null))));
const result = { schema_version: 1, before_commit: beforeCommit, after_commit: afterCommit, results, case_path: casePath,
  checks: ['fixed-code static understanding', 'question linked to architecture', 'wrong display with successful storage', 'code revision invalidates prior result', 'decision basis retained', 'actual storage failure', 'missing read stays unknown', 'same-storage application restart', 'retest succeeds', 'domain change preserves results', 'manual medical condition remains unknown', 'report includes exact code and observation source'],
  limits: ['trusted synthetic fixture with local execution; no OS isolation', 'HTTP HTML content is checked by the target driver; browser rendering is a separate test', 'no network egress or medical workflow verification', 'human evaluation not conducted'] };
await writeFile(join(output, 'result.json'), JSON.stringify(result, null, 2));
console.log(JSON.stringify({ output, checks: result.checks, results }));
