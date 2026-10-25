// 実Rust APIと画面で、固定レシピの選択・版の変更・実行要求を確認する。
// --liveではOpenEMRを実際に起動する。通常は実行直前で止める合成試験。
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, mkdir, writeFile, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve, join } from 'node:path';
import { chromium } from 'playwright';
const root = resolve(import.meta.dirname, '..'), live = process.argv.includes('--live');
const temp = await mkdtemp(join(tmpdir(), 'eel-browser-handoff-')), data = join(temp, 'app');
const original = JSON.parse(await readFile(join(root, 'examples/openemr-browser/soap-recipe.json'), 'utf8'));
const repository = original.source.repository, commit = original.source.commit, before = 'a'.repeat(40), path = original.source.path;
const output = join(root, 'test-results');
await mkdir(join(data, 'snapshots', repository), { recursive: true });
let text = '# 合成の読取記録。実OpenEMRのコードではありません。\n';
if (live) {
  const response = await fetch(`https://raw.githubusercontent.com/${repository}/${commit}/${path}`);
  assert.equal(response.status, 200); text = await response.text();
}
for (const sha of [before, commit]) await writeFile(join(data, 'snapshots', repository, sha + '.json'), JSON.stringify({
  schema: 1, fullName: repository, reference: sha, commit: sha,
  inventory: [{ path, mode: '100644' }], files: { [path]: sha === before ? '# 合成の旧版\n' : text }, omitted: [],
}));
const secrets = join(temp, 'secrets.json');
await writeFile(secrets, JSON.stringify({ username: 'eel-operator', password: 'eel-pass-trial' }), { mode: 0o600 });
const child = spawn(process.env.REVIEW_BINARY || join(root, 'target/debug/mugiurushi'), ['app', data, '--port', '0'], {
  env: { ...process.env, EEL_BROWSER_SECRETS_FILE: secrets }, stdio: ['ignore', 'pipe', 'pipe'],
});
let browser;
try {
  const url = await new Promise((ok, no) => {
    let out = '', err = ''; const timer = setTimeout(() => no(Error(err || '起動待ちの上限')), 15000);
    child.stderr.on('data', d => err += d); child.stdout.on('data', d => {
      out += d; const m = /http:\/\/127\.0\.0\.1:\d+/.exec(out); if (m) { clearTimeout(timer); ok(m[0]); }
    }); child.once('exit', () => { clearTimeout(timer); no(Error(err)); });
  });
  browser = await chromium.launch({ headless: true, executablePath: process.env.REVIEW_BROWSER_EXECUTABLE || undefined, args: ['--no-sandbox'] });
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } }); page.setDefaultTimeout(15000);
  const errors = []; page.on('pageerror', e => errors.push(e.message));
  await page.addInitScript(() => sessionStorage.setItem('eel-person', JSON.stringify({ name: '合成の試験担当', role: 'developer' })));
  await page.goto(url + '/workbench');
  const token = await page.locator('meta[name=review-token]').getAttribute('content'), headers = { 'X-Review-Token': token, 'Content-Type': 'application/json' };
  const view = async () => (await page.request.get(url + '/api/v1/case', { headers })).json();
  const post = async (name, body) => {
    const r = await page.request.post(url + '/api/v1/' + name, { headers, data: body });
    const result = await r.json(); assert.equal(r.status(), 200, JSON.stringify(result)); return result;
  };
  const origin = '合成の相談から登録した確認項目。実利用者の相談ではありません。';
  await post('change', { expected_revision: (await view()).workspace.revision, actor: '合成の試験担当', reason: '画面の接続試験', action: { op: 'add_check', check: {
    id: 'soap-dialogue-check', version: 1, text: 'SOAPを保存して開き直し、4項目の内容をDBと照合できる', origin,
    source: { ...original.source, commit: before }, rule: 'manual_only', dependencies: [], questions: [],
  } } });
  const requestUrl = new URL(url + '/workbench'); requestUrl.search = new URLSearchParams({ url: 'https://github.com/' + repository, ref: before, check: 'soap-dialogue-check', from: 'system', focus: 'synthetic-node', diagram: 'd'.repeat(64) }); requestUrl.hash = 'trial';
  await page.goto(requestUrl.href);
  const preset = page.locator('[data-browser-preset="openemr-soap"]'); await preset.waitFor();
  assert.match(await preset.innerText(), /現在の確認項目の版/);
  assert.equal((await view()).workspace.checks[0].source.commit, before);
  await preset.getByRole('button', { name: /^レシピの版に切り替えて設定する/ }).click();
  await page.getByRole('region', { name: 'ブラウザ試験の設定' }).waitFor();
  let v = await view(); assert.equal(v.workspace.checks[0].source.commit, commit); assert.equal(v.workspace.checks[0].origin, origin); assert.equal(v.workspace.evidence.length, 0);
  assert.equal(new URL(await page.locator('#back-to-system').getAttribute('href'), url).searchParams.get('ref'), before);
  await page.getByRole('button', { name: '次へ：試す状況を選ぶ', exact: true }).click();
  await page.getByRole('button', { name: '次へ：判定の条件を確認する', exact: true }).click();
  await page.getByRole('button', { name: '実行前に確認する', exact: true }).click();
  assert.match(await page.locator('#trial-body').innerText(), /soap-subjective/);
  assert.equal((await view()).workspace.evidence.length, 0);
  await mkdir(output, { recursive: true }); await page.screenshot({ path: join(output, 'browser-handoff-confirm.png'), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), '390px幅で横にはみ出さない');
  await page.screenshot({ path: join(output, 'browser-handoff-confirm-mobile.png'), fullPage: true });
  await page.setViewportSize({ width: 1440, height: 1000 });
  let sent;
  if (!live) {
    await page.route('**/api/v1/prepare', route => route.fulfill({ json: { executed: false } }));
    await page.route('**/api/v1/execute', route => {
      sent = route.request().postDataJSON(); return route.fulfill({ status: 400, json: { error: '合成試験：実行直前で停止' } });
    });
  }
  await page.getByRole('button', { name: '試験を始める', exact: true }).click();
  if (live) {
    await Promise.race([
      page.locator('#result:not([hidden])').waitFor({ timeout: 600000 }),
      page.locator('#alert .dads-notification-banner').waitFor({ timeout: 600000 }).then(async () => { throw Error(await page.locator('#alert').innerText()); }),
    ]);
    v = await view(); const evidence = v.workspace.evidence.at(-1);
    assert.equal(evidence.check_id, 'soap-dialogue-check'); assert.equal(evidence.execution.recipe.source.commit, commit);
    assert.equal(evidence.execution.containers.browser.completed, true); assert.deepEqual(evidence.execution.containers.leftovers, []);
    assert(evidence.execution.trial.results.every(r => r.observed === 'pass'));
    const actual = evidence.execution.recipe;
    const normalized = value => {
      if (Array.isArray(value)) return value.map(normalized);
      if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value)
        .filter(([key, v]) => !(v === null || (['frames', 'http_reads'].includes(key) && Array.isArray(v) && !v.length)))
        .map(([key, v]) => [key, normalized(v)]));
      return value;
    };
    assert.deepEqual(normalized(actual.browser), normalized(original.browser));
    await writeFile(join(output, 'browser-handoff-live.json'), JSON.stringify({ source: actual.source, run_id: evidence.manifest.correlation.run_id, verdict: v.checks.find(c => c.check_id === 'soap-dialogue-check').verdict, human_evaluation: 'not_conducted' }, null, 2));
    // 再読込後の再試験は、保存したブラウザ手順を復元する。
    await page.reload(); await page.locator('#result:not([hidden]) [data-evidence]').first().waitFor();
    assert.equal(new URL(await page.locator('#back-to-system').getAttribute('href'), url).searchParams.get('ref'), before);
    await page.locator('a.step-link[href="#trial"]').click();
    await page.getByRole('region', { name: 'ブラウザ試験の設定' }).waitFor();
  } else {
    await page.getByText('合成試験：実行直前で停止', { exact: true }).waitFor();
    const expected = structuredClone(original); expected.check_id = 'soap-dialogue-check'; expected.criterion.subject = expected.check_id;
    assert.deepEqual(sent.recipe, expected, '準備・操作・HTTP・DBの対象・入力・条件IDを落とさない');
    assert.equal((await view()).workspace.evidence.length, 0);
    await page.reload(); await page.locator('[data-browser-preset="openemr-soap"]').waitFor();
    assert.equal(new URL(await page.locator('#back-to-system').getAttribute('href'), url).searchParams.get('ref'), before);
  }
  assert.deepEqual(errors, []);
  console.log(JSON.stringify({ checks: ['版の相違を表示', '明示操作による参照先変更', '相談の出所を保持', '確認まで実行しない', 'ブラウザ手順・入力・観測先を保持', ...(live ? ['画面から実OpenEMRを起動・保存・再表示・DB照合', '再読込後にブラウザ設定へ戻る'] : [])], actual_target_execution: live ? 'OpenEMR' : 'not_conducted' }));
} catch (error) {
  await mkdir(output, { recursive: true });
  const page = browser?.contexts()[0]?.pages()[0];
  await page?.screenshot({ path: join(output, 'browser-handoff-failure.png'), fullPage: true }).catch(() => {});
  throw error;
} finally { child.kill('SIGTERM'); await browser?.close(); await rm(temp, { recursive: true, force: true }); }
