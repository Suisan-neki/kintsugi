// 保存APIの応答だけに合成の実行記録を差し込み、実画面の表示を確認する。
// コンテナやDBを実行した結果ではない。実Rust APIに合成の証拠は保存しない。
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, mkdir, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { chromium } from 'playwright';

export async function verifyResultObservations(page, { url, baseView, recipe, checkId, output }) {
  const caseView = structuredClone(baseView);
  const check = caseView.checks.find(item => item.check_id === checkId);
  const raw = { id: 'synthetic-result-observations', execution: {
    recipe: structuredClone(recipe), mode: 'local', runtime_version: '合成の実行環境', runtime_path: 'synthetic', exit_code: 1,
    stdout_excerpt: '', stderr_excerpt: '', trial: {},
    containers: { egress: 'blocked', egress_confirmed: true, traffic_note: '合成の記録。mysqlの中継ログは一部未観測。', traffic: [
      { from: 'openemr', to: 'mysql', connections: 4, failed: 1, unfinished: 2, bytes_from_client: 1234, bytes_to_client: 0 },
      { from: '<img src=x onerror=alert(1)>', to: 'mysql', connections: 0, failed: 0, bytes_from_client: 0, bytes_to_client: 0 },
    ] },
  } };
  const evidence = { id: raw.id, run_id: 'synthetic-db-recovery', applicable: true,
    observed: { verdict: 'not_satisfied', reason: '合成の故障中の判定', next_action: '保存操作を再検証する', unconfirmed: [] },
    summary: { tried: '合成の故障と復旧の記録を表示する', fault: 'DB停止', test_value: 'synthetic', items: [], command: [] },
  };
  check.evidence = [evidence]; check.verdict = 'not_satisfied'; check.reason = evidence.observed.reason;
  caseView.verification_passed = false;
  caseView.workspace.evidence = [raw];
  raw.execution.trial.recovery = {
    attempted: true, effective: true, applied: '同じDBの起動を確認した（合成）', database_query: 'SELECT 1の応答は 1（合成）',
    services: [{ name: 'mysql', ready: true, ready_after_ms: 0, detail: '準備できた（合成）' }],
    changes: [
      { id: 'soap', source: 'database_rows', during_fault: '故障中は読めなかった', after_recovery: '正常終了し0行を読み出した', rows_during_fault: null, rows_after_recovery: 0, change: 'readable_after_recovery' },
      { id: 'server-read', source: 'http_read', during_fault: 'HTTP 503', after_recovery: 'HTTP 200', rows_during_fault: null, rows_after_recovery: null, change: 'changed' },
    ], limits: '読み出せた内容だけを比べた（合成）。保存の再実行はしていない。',
  };
  const target = new URL('/workbench', url);
  target.search = new URLSearchParams({ url: 'https://github.com/' + recipe.source.repository, ref: recipe.source.commit, check: checkId });
  target.hash = 'result';
  await page.route('**/api/v1/case', route => route.fulfill({ json: caseView }));
  const card = page.locator(`[data-evidence="${raw.id}"]`);
  const traffic = card.locator('[data-container-observations]'), recovery = card.locator('[data-recovery-observations]');
  const reload = async () => {
    if (page.url() === target.href) await page.reload(); else await page.goto(target.href);
    await card.waitFor();
    for (const label of ['コンテナの通信と隔離', 'DB故障の解除と復旧後の比較']) {
      const summary = card.locator('summary', { hasText: label });
      if (await summary.count()) await summary.click();
    }
  };
  try {
    await reload();
    assert.match(await traffic.innerText(), /外部通信を遮断する設定を確認した/);
    assert.match(await traffic.innerText(), /アプリの処理の失敗件数ではありません/);
    assert.match(await traffic.innerText(), /mysqlの中継ログは一部未観測/);
    const cells = await traffic.locator('tbody tr').first().locator('td').allTextContents();
    assert.deepEqual(cells, ['openemr', 'mysql', '4件', '1件', '2件', '1,234', '0']);
    assert.match(await traffic.locator('tbody tr').nth(1).innerText(), /未記録/);
    assert.equal(await traffic.locator('img').count(), 0, '取得した文字をHTMLとして実行しない');
    assert.match(await traffic.innerText(), /<img src=x onerror=alert\(1\)>/);
    const soap = recovery.locator('tbody tr', { hasText: '故障中は読めなかった' });
    assert.match(await soap.locator('td').nth(2).innerText(), /行数は不明/);
    assert(!/0行/.test(await soap.locator('td').nth(2).innerText()), '読めなかった行数を0行にしない');
    assert.match(await soap.locator('td').nth(3).innerText(), /^0行/);
    assert.match(await soap.innerText(), /復旧後に読み出せた/);
    assert.equal(await recovery.locator('tbody tr', { hasText: 'server-read' }).getByText('行数は不明', { exact: true }).count(), 0, 'HTTP応答にDB行数を付けない');
    assert.match(await recovery.innerText(), /0ms/);
    assert.match(await recovery.innerText(), /故障中の判定は変更しません/);
    assert.equal(await card.locator('.verdict').getAttribute('data-verdict'), 'not_satisfied', '復旧できても故障中の失敗判定を成功にしない');
    await page.screenshot({ path: join(output, 'result-observations.png'), fullPage: true });
    await page.setViewportSize({ width: 390, height: 844 });
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), '表があっても390px幅の画面全体を横にはみ出さない');
    await page.screenshot({ path: join(output, 'result-observations-mobile.png'), fullPage: true });
    await page.setViewportSize({ width: 1440, height: 1000 });

    raw.execution.containers.egress_confirmed = false;
    const recovered = raw.execution.trial.recovery;
    recovered.effective = false; recovered.services[0].ready = false; recovered.services[0].ready_after_ms = null; recovered.changes = [];
    await reload();
    assert.match(await traffic.innerText(), /遮断を指定したが、設定を確認できていない/);
    assert.match(await recovery.innerText(), /故障の解除後の状態を確認できていない/);
    assert.match(await recovery.innerText(), /準備できていない/);
    assert.match(await recovery.innerText(), /比較は記録されていません/);
    recovered.attempted = false; await reload();
    assert.match(await recovery.innerText(), /故障の解除は試みていません/);

    raw.execution.containers.egress = 'allowed'; raw.execution.containers.traffic = [];
    await reload();
    assert.match(await traffic.innerText(), /外部通信を許可した/);
    assert.match(await traffic.innerText(), /通信がなかったことを示すものではありません/);

    delete raw.execution.containers.egress; delete raw.execution.containers.egress_confirmed;
    delete raw.execution.containers.traffic; delete raw.execution.containers.traffic_note; delete raw.execution.trial.recovery;
    await reload();
    assert.match(await traffic.innerText(), /外部通信の方針は未記録/);
    assert.match(await traffic.innerText(), /接続の集計は記録されていません/);
    assert.equal(await recovery.count(), 0, '旧記録に復旧を実施したと表示しない');
    raw.execution.recipe.trial.recovery = { ready_timeout_ms: 1000 }; await reload();
    assert.match(await recovery.innerText(), /復旧結果は記録されていません/);
    delete raw.execution;
    await reload();
    assert.equal(await traffic.count(), 0); assert.equal(await recovery.count(), 0);
    assert.equal(await card.locator('.verdict').getAttribute('data-verdict'), 'not_satisfied', '実行記録のない旧形式でも判定を保持する');
  } finally {
    await page.unroute('**/api/v1/case');
  }
}

// この表示の確認にはDockerを要しない。読取記録と確認項目は実Rust APIで準備する。
if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const root = resolve(import.meta.dirname, '..'), temp = await mkdtemp(join(tmpdir(), 'mugiurushi-result-ui-'));
  const output = join(root, 'test-results'), data = join(temp, 'app');
  const recipe = JSON.parse(await readFile(join(root, 'examples/openemr-browser/soap-recipe.json'), 'utf8'));
  const source = recipe.source, checkId = 'synthetic-result-check';
  await mkdir(join(data, 'snapshots', source.repository), { recursive: true }); await mkdir(output, { recursive: true });
  await writeFile(join(data, 'snapshots', source.repository, source.commit + '.json'), JSON.stringify({
    schema: 1, fullName: source.repository, reference: source.commit, commit: source.commit,
    inventory: [{ path: source.path, mode: '100644' }], files: { [source.path]: '# 合成の読取記録。実OpenEMRのコードではありません。\n' }, omitted: [],
  }));
  const child = spawn(process.env.REVIEW_BINARY || join(root, 'target/debug/mugiurushi'), ['app', data, '--port', '0'], { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] });
  let browser;
  try {
    const url = await new Promise((accept, reject) => {
      let stdout = '', stderr = ''; const timer = setTimeout(() => reject(Error(stderr || '起動待ちの上限')), 15000);
      child.stderr.on('data', chunk => stderr += chunk);
      child.stdout.on('data', chunk => { stdout += chunk; const match = /http:\/\/127\.0\.0\.1:\d+/.exec(stdout); if (match) { clearTimeout(timer); accept(match[0]); } });
      child.once('error', error => { clearTimeout(timer); reject(error); });
      child.once('exit', () => { clearTimeout(timer); reject(Error(stderr || 'サーバーが終了した')); });
    });
    browser = await chromium.launch({ headless: true, executablePath: process.env.REVIEW_BROWSER_EXECUTABLE || undefined, args: ['--no-sandbox'] });
    const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } }); page.setDefaultTimeout(10000);
    const errors = []; page.on('pageerror', error => errors.push(error.message));
    await page.goto(url + '/workbench');
    const headers = { 'X-Review-Token': await page.locator('meta[name=review-token]').getAttribute('content') };
    const view = async () => { const response = await page.request.get(url + '/api/v1/case', { headers }); assert.equal(response.status(), 200); return response.json(); };
    const initial = await view();
    const added = await page.request.post(url + '/api/v1/change', { headers, data: {
      expected_revision: initial.workspace.revision, actor: '合成の試験担当', reason: '結果表示を確認する', action: { op: 'add_check', check: {
        id: checkId, version: 1, text: '合成の故障中と復旧後の記録を比較する', origin: '表示の合成試験', source, rule: 'manual_only', dependencies: [], questions: [],
      } },
    } });
    assert.equal(added.status(), 200, await added.text());
    const baseView = await view();
    await verifyResultObservations(page, { url, baseView, recipe, checkId, output });
    assert.deepEqual(await view(), baseView, '表示確認で保存済みの判定・証拠を書き換えない');
    assert.deepEqual(errors, []);
    console.log('通信・故障解除・未観測と0行・旧記録・判定の保持・390px幅・HTMLとして解釈しない表示を確認。対象アプリの実行は未実施。');
  } catch (error) {
    await browser?.contexts()[0]?.pages()[0]?.screenshot({ path: join(output, 'result-observations-failure.png'), fullPage: true }).catch(() => {});
    throw error;
  } finally {
    child.kill('SIGTERM'); await browser?.close(); await rm(temp, { recursive: true, force: true });
  }
}
