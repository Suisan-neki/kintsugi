// 実Rust APIとブラウザで人の試用記録を検査する。対象OSSの実行・人の試用は実施しない。
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, rm, mkdir, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve, join } from 'node:path';
import { chromium } from 'playwright';
const root = resolve(import.meta.dirname, '..');
const temporary = await mkdtemp(join(tmpdir(), 'mugiurushi-user-trial-'));
const binary = process.env.REVIEW_BINARY || join(root, 'target/debug/mugiurushi');
// 合成の固定版を読ませる。ネットワークや実OSSの評価を代替する試験ではない。
const snapshot = JSON.parse(await readFile(join(root, 'tests/fixtures/reader/system.json'), 'utf8'));
await mkdir(join(temporary, 'app', 'snapshots', snapshot.fullName), { recursive: true });
for (const commit of ['a'.repeat(40), 'b'.repeat(40)]) await writeFile(join(temporary, 'app', 'snapshots', snapshot.fullName, commit + '.json'), JSON.stringify({ ...snapshot, commit }));
let child, browser;
async function start() {
  child = spawn(binary, ['app', join(temporary, 'app'), '--port', '0'], { stdio: ['ignore', 'pipe', 'pipe'] });
  return new Promise((resolve, reject) => {
    let out = '', error = '';
    const timer = setTimeout(() => reject(Error(error || 'startup timeout')), 15000);
    child.stderr.on('data', part => error += part);
    child.stdout.on('data', part => { out += part; const match = /http:\/\/127\.0\.0\.1:\d+/.exec(out); if (match) { clearTimeout(timer); resolve(match[0]); } });
    child.once('exit', () => { clearTimeout(timer); reject(Error(error || 'server stopped')); });
    child.once('error', error => { clearTimeout(timer); reject(error); });
  });
}
async function stop() {
  if (!child || child.exitCode !== null) return;
  const ended = new Promise(resolve => child.once('exit', resolve));
  child.kill('SIGTERM');
  await ended;
}
try {
  let url = await start();
  browser = await chromium.launch({ headless: true, executablePath: process.env.REVIEW_BROWSER_EXECUTABLE || undefined, args: ['--no-sandbox'] });
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  page.setDefaultTimeout(10000);
  const errors = []; page.on('pageerror', error => errors.push(error.message));
  await page.goto(url + '/user-trial');
  await page.getByText('試用計画がまだありません。', { exact: false }).waitFor();
  let token = await page.locator('meta[name=review-token]').getAttribute('content');
  const headers = () => ({ 'X-Review-Token': token, 'Content-Type': 'application/json' });
  const get = async () => (await page.request.get(url + '/api/v1/case', { headers: headers() })).json();
  const post = async body => page.request.post(url + '/api/v1/change', { headers: headers(), data: body });
  await page.getByText('技術者が試用を準備する', { exact: true }).click();
  await page.locator('#preparer').fill('合成の準備者');
  await page.locator('#concern').fill('保存完了が分からないという仮想相談');
  await page.locator('#task').fill('架空の記録を保存してから画面を開き直す');
  await page.locator('#conditions').fill('合成の画面操作試験。対象OSSは起動していない。');
  for (const [label, commit, port] of [['a', 'a'.repeat(40), '18001'], ['b', 'b'.repeat(40), '18002']]) {
    await page.locator('#' + label + '-repository').fill('openemr/openemr');
    await page.locator('#' + label + '-commit').fill(commit);
    await page.locator('#' + label + '-path').fill('interface/forms/soap/new.php');
    await page.locator('#' + label + '-url').fill('http://127.0.0.1:' + port + '/');
  }
  await page.getByRole('button', { name: '試用計画を登録する', exact: true }).click();
  await page.getByText('試用計画を登録しました。', { exact: false }).waitFor();
  let v = await get(); const planId = v.workspace.user_trial_plans[0].id;
  assert.equal(v.verification_passed, false);
  assert.equal(v.workspace.evidence.length, 0);
  await page.locator('#participant-' + planId).fill('合成参加者01');
  await page.locator('#order-' + planId).selectOption('b,a');
  await page.getByRole('button', { name: 'この内容を試す', exact: true }).click();
  await page.getByRole('heading', { name: '次はBを試す', exact: true }).waitFor();
  const session = (await get()).workspace.user_trial_sessions[0];
  assert.deepEqual(session.order, ['b', 'a']);
  const link = page.getByRole('link', { name: 'Bのアプリを開く', exact: true });
  assert.equal(await link.getAttribute('target'), '_blank');
  assert.equal(await link.getAttribute('rel'), 'noopener noreferrer');
  assert.equal(await link.getAttribute('href'), 'http://127.0.0.1:18002/');
  assert.equal((await get()).workspace.user_trial_sessions[0].feedback.length, 0);
  await page.locator('#operation').fill('合成の操作記録');
  await page.locator('#feedback').fill('<img src=x onerror="window.pwned=1">');
  await page.locator('#outcome').selectOption('blocked');
  v = await get();
  const concurrent = await post({ expected_revision: v.workspace.revision, actor: '別の記入者', reason: '競合を検査する', action: { op: 'set_context', key: 'trial_environment', value: '合成の条件変更' } });
  assert.equal(concurrent.status(), 200);
  await page.getByRole('button', { name: 'Bの感想を保存する', exact: true }).click();
  await page.locator('#alert').getByText('ほかの操作で記録が更新されています。', { exact: false }).waitFor();
  assert.equal(await page.locator('#operation').inputValue(), '合成の操作記録');
  await page.getByRole('button', { name: '最新の記録を読み直す', exact: true }).click();
  await page.getByText('最新の記録を読み込みました。', { exact: true }).waitFor();
  await page.locator('#session-body').getByText('この試用を始めた後に、現在の検証条件が変わっています。', { exact: false }).waitFor();
  assert.equal(await page.locator('#feedback').inputValue(), '<img src=x onerror="window.pwned=1">');
  assert.equal(await page.locator('#outcome').inputValue(), 'blocked');
  await page.getByRole('button', { name: 'Bの感想を保存する', exact: true }).click();
  await page.getByRole('heading', { name: '次はAを試す', exact: true }).waitFor();
  assert.equal(await page.locator('#session-body img').count(), 0);
  assert.equal(await page.evaluate(() => window.pwned), undefined);
  await page.reload();
  await page.getByRole('heading', { name: '次はAを試す', exact: true }).waitFor();
  await page.locator('#operation').fill('操作を試そうとしたが環境へ接続できなかった');
  await page.locator('#outcome').selectOption('not_tried');
  await page.locator('#feedback').fill('合成の接続不可。試用を実施した意味にはしない。');
  await page.getByRole('button', { name: 'Aの感想を保存する', exact: true }).click();
  await page.getByText('A・Bの記録を保存しました。', { exact: false }).waitFor();
  v = await get();
  assert.equal(v.workspace.user_trial_sessions[0].feedback.length, 2);
  assert.equal(v.workspace.user_trial_sessions[0].feedback[1].outcome, 'not_tried');
  assert.equal(v.verification_passed, false);
  assert.equal(v.workspace.decisions.length, 0);
  await page.getByRole('button', { name: '試す内容の一覧に戻る', exact: true }).click();
  await page.locator('#plans').getByText('この計画の作成後に、検証する条件が変わっています。', { exact: false }).waitFor();
  assert.equal(await page.getByRole('button', { name: 'この内容を試す', exact: true }).count(), 0, '条件変更済み計画から新しい試用を開始できてしまう');
  await page.getByRole('button', { name: '記録を見る', exact: true }).click();
  await page.getByText('A・Bの記録を保存しました。', { exact: false }).waitFor();
  assert.equal((await page.request.get(url + '/api/v1/case')).status(), 403);
  const id = session.id;
  await stop(); url = await start();
  await page.goto(url + '/user-trial?session=' + id);
  await page.getByText('A・Bの記録を保存しました。', { exact: false }).waitFor();
  token = await page.locator('meta[name=review-token]').getAttribute('content');
  assert.equal((await get()).workspace.user_trial_sessions[0].feedback.length, 2);
  // 別タブがAを保存した後も、手元のAへの下書きをBに転記しない。
  v = await get();
  const original = v.workspace.user_trial_plans[0];
  const secondPlan = { id: 'concurrent-plan', concern: original.concern, task: original.task, conditions: original.conditions, variants: original.variants };
  let response = await post({ expected_revision: v.workspace.revision, actor: '合成準備者', reason: '別タブ操作を検査する計画', action: { op: 'add_user_trial_plan', plan: secondPlan } });
  assert.equal(response.status(), 200);
  v = await response.json();
  response = await post({ expected_revision: v.workspace.revision, actor: '合成参加者02', reason: '並行操作を検査する', action: { op: 'start_user_trial', plan_id: secondPlan.id, participant: '合成参加者02', order: ['a', 'b'] } });
  assert.equal(response.status(), 200);
  v = await response.json();
  const concurrentSession = v.workspace.user_trial_sessions.at(-1);
  await page.goto(url + '/user-trial?session=' + concurrentSession.id);
  await page.getByRole('heading', { name: '次はAを試す', exact: true }).waitFor();
  await page.locator('#operation').fill('Aにだけ対応する未保存の操作');
  await page.locator('#feedback').fill('Aにだけ対応する未保存の感想');
  await page.locator('#outcome').selectOption('blocked');
  response = await post({ expected_revision: v.workspace.revision, actor: '別の記入者', reason: '別タブでAを記録する', action: { op: 'record_user_trial_feedback', session_id: concurrentSession.id, variant: 'a', operation: '別タブで記録したAの操作', feedback: '別タブでのAの感想', outcome: 'completed' } });
  assert.equal(response.status(), 200);
  await page.getByRole('button', { name: 'Aの感想を保存する', exact: true }).click();
  await page.locator('#alert').getByText('ほかの操作で記録が更新されています。', { exact: false }).waitFor();
  await page.getByRole('button', { name: '最新の記録を読み直す', exact: true }).click();
  await page.getByRole('heading', { name: '次はBを試す', exact: true }).waitFor();
  assert.equal(await page.locator('#operation').inputValue(), '');
  assert.equal(await page.locator('#feedback').inputValue(), '');
  assert.equal(await page.locator('#outcome').inputValue(), '');
  await page.locator('#unsaved-draft').getByText('未保存の内容を見る', { exact: true }).click();
  await page.locator('#unsaved-draft').getByText('感想：Aにだけ対応する未保存の感想', { exact: true }).waitFor();
  // 同じ確認事項の二つの版を比較し、登録前の試用下書きへ引き継ぐ。
  const apply = async action => {
    const current = await get();
    const result = await post({ expected_revision: current.workspace.revision, actor: '合成の技術者', reason: '比較から試用準備への接続確認', action });
    assert.equal(result.status(), 200, await result.text());
    return result.json();
  };
  const checkId = 'example-record-system-handoff';
  const source = (commit, repository = snapshot.fullName) => ({ repository, commit: commit.repeat(40), path: 'server/main.py', line: 3, end_line: 5 });
  await apply({ op: 'add_check', check: { id: checkId, version: 1, text: '合成の保存操作を確認する', origin: '合成の改善相談', source: source('a'), observation: null, rule: 'manual_only', dependencies: [], questions: [] } });
  await apply({ op: 'configure_reported_observation', check_id: checkId, criterion: { assertion: 'readback', subject: 'synthetic-record' } });
  const importHandoff = async (run, selectedSource) => {
    const current = await get(), check = current.workspace.checks.find(check => check.id === checkId);
    const correlation = { run_id: run, record_id: 'synthetic-record', attempt_id: 'attempt-' + run };
    return apply({ op: 'import_reported', condition: { id: check.id, version: check.version, text: check.text, origin: check.origin, kind: 'custom' }, trace: { schema_version: 1, correlation, producer: '合成の報告元', origin: '接続試験のみ。OSSは未実行', completed: true, source: selectedSource, context: {}, observations: [{ sequence: 1, correlation, assertion: 'readback', subject: 'synthetic-record', outcome: 'pass', detail: '合成の成功報告', source: selectedSource }] } });
  };
  await importHandoff('handoff-a', source('a'));
  await apply({ op: 'bind_source', check_id: checkId, source: source('b') });
  await importHandoff('handoff-b', source('b'));
  await page.goto(url + '/workbench?' + new URLSearchParams({ url: 'https://github.com/' + snapshot.fullName, ref: 'b'.repeat(40), check: checkId }) + '#result');
  await page.getByRole('button', { name: '比べる', exact: true }).click();
  const handoff = page.getByRole('link', { name: 'この比較から試用を準備する', exact: true });
  await handoff.waitFor();
  const href = await handoff.getAttribute('href');
  assert.deepEqual([...new URL(href, url).searchParams.keys()], ['check', 'before', 'after', 'revision']);
  const beforeHandoff = await get();
  await handoff.click();
  await page.locator('#handoff-notice:not([hidden])').waitFor();
  assert.equal(await page.locator('#concern').inputValue(), '合成の保存操作を確認する');
  assert.equal(await page.locator('#a-commit').inputValue(), 'a'.repeat(40));
  assert.equal(await page.locator('#b-commit').inputValue(), 'b'.repeat(40));
  assert.equal(await page.locator('#a-line').inputValue(), '3');
  assert.equal(await page.locator('#a-end-line').inputValue(), '5');
  for (const id of ['task', 'conditions', 'preparer', 'a-url', 'b-url']) assert.equal(await page.locator('#' + id).inputValue(), '');
  assert.equal((await get()).workspace.revision, beforeHandoff.workspace.revision, 'handoff must not write');
  const invalidQueries = [
    { revision: String(beforeHandoff.workspace.revision - 1) },
    { before: 'missing-evidence' },
    { after: new URL(href, url).searchParams.get('before') },
    { check: 'missing-check' },
    { session: id },
  ].map(values => {
    const query = new URL(href, url); for (const [key, value] of Object.entries(values)) query.searchParams.set(key, value); return query;
  });
  const duplicate = new URL(href, url); duplicate.searchParams.append('check', checkId); invalidQueries.push(duplicate);
  for (const query of invalidQueries) {
    await page.goto(query.href);
    await page.locator('#alert .box').waitFor();
    for (const id of ['concern', 'a-commit', 'b-commit', 'a-evidence', 'b-evidence']) assert.equal(await page.locator('#' + id).inputValue(), '', 'invalid handoff must not partially prefill');
  }
  assert.equal((await get()).workspace.revision, beforeHandoff.workspace.revision);
  // 下書き作成後に条件が変わっても、再読込で新しい条件へ読み替えて登録しない。
  await page.goto(new URL(href, url).href);
  await page.locator('#handoff-notice:not([hidden])').waitFor();
  const plansBeforeReload = (await get()).workspace.user_trial_plans.length;
  await apply({ op: 'set_context', key: 'handoff_environment', value: '比較後に変更した合成条件' });
  await page.getByRole('button', { name: '最新の記録を読み直す', exact: true }).click();
  await page.locator('#handoff-notice').getByText('比較した後に記録が更新されたため、この下書きは登録できません。', { exact: false }).waitFor();
  await page.locator('#task').fill('合成の操作');
  await page.locator('#conditions').fill('合成の条件');
  await page.locator('#preparer').fill('合成の準備者');
  await page.locator('#a-url').fill('http://localhost:18001/');
  await page.locator('#b-url').fill('http://127.0.0.1:18002/');
  await page.getByRole('button', { name: '試用計画を登録する', exact: true }).click();
  await page.locator('#alert').getByText('比較した後に記録が更新されています。', { exact: false }).waitFor();
  assert.equal((await get()).workspace.user_trial_plans.length, plansBeforeReload);
  assert.equal(await page.locator('#a-commit').inputValue(), 'a'.repeat(40));
  await page.getByRole('button', { name: '記録を見る', exact: true }).first().click();
  await page.getByRole('button', { name: '試す内容の一覧に戻る', exact: true }).click();
  await page.getByRole('button', { name: '試用計画を登録する', exact: true }).click();
  await page.locator('#alert').getByText('比較した後に記録が更新されています。', { exact: false }).waitFor();
  assert.equal((await get()).workspace.user_trial_plans.length, plansBeforeReload, 'session navigation must not detach retained draft from its comparison revision');
  // 異なる試行でもコミットが同じなら、変更前後の計画には引き継がない。
  v = await importHandoff('handoff-same-commit', source('b'));
  const sameCommitQuery = new URL(href, url);
  sameCommitQuery.searchParams.set('revision', v.workspace.revision);
  sameCommitQuery.searchParams.set('before', new URL(href, url).searchParams.get('after'));
  sameCommitQuery.searchParams.set('after', v.workspace.evidence.at(-1).id);
  await page.goto(sameCommitQuery.href);
  await page.locator('#alert .box').waitFor();
  assert.equal(await page.locator('#concern').inputValue(), '');
  // 上流→研究用forkでは各版のリポジトリを保持し、人が残りの欄を埋めて登録する。
  const forkSource = source('c', 'example/record-system-fork');
  await apply({ op: 'bind_source', check_id: checkId, source: forkSource });
  v = await importHandoff('handoff-fork', forkSource);
  const forkQuery = new URL(href, url);
  forkQuery.searchParams.set('revision', v.workspace.revision);
  forkQuery.searchParams.set('after', v.workspace.evidence.at(-1).id);
  const plansBeforeFork = v.workspace.user_trial_plans.length;
  await page.goto(forkQuery.href);
  await page.locator('#handoff-notice:not([hidden])').waitFor();
  assert.equal(await page.locator('#a-repository').inputValue(), snapshot.fullName);
  assert.equal(await page.locator('#b-repository').inputValue(), forkSource.repository);
  assert.equal(await page.locator('#b-commit').inputValue(), forkSource.commit);
  assert.equal((await get()).workspace.revision, v.workspace.revision);
  for (const id of ['task', 'conditions', 'preparer', 'a-url', 'b-url']) assert.equal(await page.locator('#' + id).inputValue(), '');
  await page.locator('#task').fill('合成データを保存する操作を試す');
  await page.locator('#conditions').fill('合成の計画登録試験。対象OSSの環境は起動していない。');
  await page.locator('#preparer').fill('合成の準備者');
  await page.locator('#a-url').fill('http://localhost:18001/');
  await page.locator('#b-url').fill('http://127.0.0.1:18002/');
  await page.getByRole('button', { name: '試用計画を登録する', exact: true }).click();
  await page.getByText('試用計画を登録しました。', { exact: false }).waitFor();
  v = await get();
  assert.equal(v.workspace.user_trial_plans.length, plansBeforeFork + 1);
  assert.equal(v.workspace.user_trial_plans.at(-1).variants[1].source.repository, forkSource.repository);
  assert.equal(v.workspace.user_trial_plans.at(-1).variants[0].source.end_line, 5);
  assert.equal(new URL(page.url()).searchParams.has('check'), false);
  await mkdir(join(root, 'test-results'), { recursive: true });
  await page.screenshot({ path: join(root, 'test-results/user-trial.png'), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, 'mobile page must not overflow horizontally');
  await page.getByRole('button', { name: '記録を見る', exact: true }).first().click();
  assert.equal(new URL(page.url()).searchParams.has('check'), false);
  await page.reload();
  await page.getByText('A・Bの記録を保存しました。', { exact: false }).waitFor();
  assert.deepEqual(errors, []);
  console.log(JSON.stringify({ checks: ['plan form without diagram', 'B then A operation and feedback', 'links carry no review token and disown opener', 'no feedback from opening link', 'conflict preserves draft after refresh', 'concurrent advance never moves A draft into B', 'obsolete plan cannot start a new session; historical session feedback remains bound to old conditions', 'escaped feedback', 'session reload and restart persistence', 'not tried stays separate', 'no QA verdict or adoption change', 'API authentication', 'comparison-to-trial draft without writes or credentials', 'source range retained', 'invalid or same-commit handoff rejected without partial prefill', 'upstream-to-fork prefill and explicit plan registration'], oss_execution: 'not_run', actual_user_trial: 'not_conducted' }));
} finally { await stop(); await browser?.close(); await rm(temporary, { recursive: true, force: true }); }
