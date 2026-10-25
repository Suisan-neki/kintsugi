// 医療系OSSの pynetdicom（DICOMの通信）を、理解と試験の画面から固定コミットのまま試す。
// 受信サーバー（storescp）を先に起動して待たせ、送信側（storescu）で試験用の画像を送る。依存先 pydicom も固定コミットで読み込む。
// GitHubへの接続が必要。画像はリポジトリに含まれる試験用のファイルだけを使い、実在の患者の画像は使わない。
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, mkdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve, join } from 'node:path';
import { chromium } from 'playwright';

const root = resolve(import.meta.dirname, '..');
const binary = process.env.REVIEW_BINARY || join(root, 'target/debug/mugiurushi');
const output = join(root, 'test-results');
const PYNETDICOM = '2692cb93222c82c872ec8ecc8a426f1c9d86bc46'; // v3.0.4
const PYDICOM = '20aa4b73aef76bffb6fbc46a8827588dfa9bb89d'; // v3.0.2
const SAMPLE = '{{source}}/pynetdicom/tests/dicom_files/CTImageStorage.dcm';
// storescp は受け取った画像を「種類.SOPインスタンスUID」の名前で保存する（apps/common.py の handle_store）。
const STORED = 'received/CT.1.3.6.1.4.1.5962.1.1.1.1.1.20040119072730.12322';
const PATIENT_ID = '1CT1'; // 試験用ファイルの患者ID（架空の値）
const temp = await mkdtemp(join(tmpdir(), 'eel-dicom-'));
await mkdir(output, { recursive: true });
const log = (...a) => console.log('・', ...a);
let server;

function start(data) {
  server = spawn(binary, ['app', data, '--port', '0', '--execution-mode', 'local'], { stdio: ['ignore', 'pipe', 'pipe'], env: process.env });
  return new Promise((accept, reject) => {
    let stdout = '';
    const timer = setTimeout(() => reject(Error('server did not start: ' + stdout)), 15000);
    server.stdout.on('data', d => { stdout += d; const m = /理解と試験の画面: (http:\/\/127\.0\.0\.1:\d+)/.exec(stdout); if (m) { clearTimeout(timer); accept(m[1]); } });
    server.stderr.on('data', d => process.stderr.write(d));
    server.once('exit', code => { clearTimeout(timer); reject(Error('server exit ' + code)); });
  });
}

let browser, page;
try {
  const url = await start(join(temp, 'data'));
  browser = await chromium.launch({ headless: true, executablePath: process.env.REVIEW_BROWSER_EXECUTABLE || undefined });
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 } });
  page = await context.newPage();
  const errors = [];
  page.on('pageerror', e => errors.push(e.message));
  page.on('console', m => { if (m.type() === 'error') errors.push(m.text()); });
  await page.goto(url + '/workbench');
  const token = await page.locator('meta[name="review-token"]').getAttribute('content');
  const caseView = async () => (await context.request.get(url + '/api/v1/case', { headers: { 'X-Review-Token': token } })).json();
  const shot = name => page.screenshot({ path: join(output, `dicom-${name}.png`), fullPage: true });
  const as = async (name, role) => { await page.locator('#person').getByLabel('名前').fill(name); await page.locator('#person').getByLabel('役割').selectOption(role); };

  // 1. 固定コミットを読む。
  await page.locator('#repo-url').fill('https://github.com/pydicom/pynetdicom');
  await page.locator('#repo-ref').fill(PYNETDICOM);
  await page.getByRole('button', { name: '読み込む', exact: true }).click();
  await page.locator('#steps').waitFor({ timeout: 120000 });
  await page.getByText('読んだ範囲とコードの版', { exact: true }).click();
  await page.getByText(`pydicom/pynetdicom のコミット ${PYNETDICOM} を読みました`).waitFor({ timeout: 120000 });
  log('pynetdicom の固定コミットを読んだ');

  // 2. 期待を書き、受信した画像を保存するコードに対応づける。
  await page.getByRole('link', { name: /検証する動作を決める/ }).click();
  await as('架空の運用担当', 'operator');
  await page.locator('#expect-text').fill('装置から受け取った画像は、保存先に書き込まれ、後から読み出せる');
  await page.getByRole('button', { name: 'この動作を確かめる' }).click();
  await page.locator('[data-check]').nth(0).waitFor();
  await page.locator('#steps a[href="#expect"]').click();
  await page.locator('#expect-text').fill('画像を保存できなかったとき、送った側に失敗が分かる');
  await page.getByRole('button', { name: 'この動作を確かめる' }).click();
  await page.locator('[data-check="pydicom-pynetdicom-2"]').waitFor();
  const view = await caseView();
  const [saved, failure] = view.workspace.checks.map(c => c.id);
  assert.ok(saved && failure);

  await as('架空の開発者', 'developer');
  const link = async (checkId, explain, method) => {
    await page.getByRole('link', { name: /ソフトウェアを理解する/ }).click();
    const scope = page.locator('#understand-body > details').first();
    if (!await scope.evaluate(d => d.open)) await scope.locator(':scope > summary').click();
    const files = page.locator('#understand details', { has: page.locator('summary', { hasText: /^ファイル一覧（/ }) }).last();
    if (!(await files.evaluate(d => d.open))) await files.locator('summary').click();
    await files.getByRole('button', { name: 'pynetdicom/apps/common.py', exact: true }).click();
    await page.getByRole('button', { name: '633行目を選ぶ' }).click();
    await page.locator('#code-link').click();
    const card = page.locator(`[data-check="${checkId}"]`);
    await card.getByLabel('この動作を実装しているコードと処理内容').fill(explain);
    await card.getByLabel('どう確かめるか（確認の方法）').fill(method);
    await card.getByRole('button', { name: '対応づけを記録' }).click();
    await card.locator('.statement[data-kind="implementation"]').waitFor({ state: 'attached' });
    const records = card.getByText('確認項目と説明の履歴', { exact: true });
    if (!await records.evaluate(node => node.parentElement.open)) await records.click();
    await card.locator('.statement[data-kind="implementation"]').waitFor();
  };
  await link(saved, 'storescp の handle_store が、受け取った画像を出力先へ ds.save_as で書き込み、成功したら状態 0x0000 を返す。', '受信サーバーを起動して試験用の画像を送り、送信側の表示、麦漆による保存先の読み出し、pydicom での読み出しを確かめる。');
  await link(failure, '書き込みで OSError が起きると、handle_store は状態 0xA700（Out of Resources）を返す。送信側の storescu がそれをどう扱うかは storescu.py の処理による。', '保存先のディレクトリを書き込み不可にして送り、送信側の終了コードと表示、保存先、受信側のエラー表示を確かめる。');
  log('期待2件を apps/common.py の 633行目（ds.save_as）に対応づけた');

  // 3. 試験の内容を入れる。
  const fill = async ({ fault, checks, server = true, readback = true }) => {
    await page.locator('#steps').getByRole('link', { name: /実際に試す/ }).click();
    const form = page.locator('#trial-body form');
    for (const title of ['Pythonの読み込み先と依存先（任意）', '先に起動するプロセス（任意）', '後から読み出す（任意）']) await form.locator('summary', { hasText: title }).click();
    await form.getByLabel(/^起動対象/).fill('pynetdicom');
    await form.getByLabel('起動のしかた').first().selectOption('module');
    await form.getByLabel('引数', { exact: true }).fill(['storescu', '-v', '127.0.0.1', '{{port}}', SAMPLE].join('\n'));
    await form.getByLabel('保存先', { exact: true }).fill(STORED);
    await form.getByLabel('依存先', { exact: true }).fill(`pydicom/pydicom ${PYDICOM} src`);
    const wait = form.getByRole('group', { name: '先に起動するプロセス（任意）', exact: true });
    await wait.getByLabel('先に起動するプロセス', { exact: true }).fill(server ? 'pynetdicom' : '');
    await wait.getByLabel('起動のしかた').selectOption('module');
    await wait.getByLabel('先に起動するプロセスの引数').fill(['storescp', '{{port}}', '-ba', '127.0.0.1', '-od', 'received'].join('\n'));
    const back = form.getByRole('group', { name: '後から読み出す（任意）', exact: true });
    await back.getByLabel('読み出し用の起動対象').fill(readback ? 'pydicom' : '');
    await back.getByLabel('起動のしかた').selectOption('script');
    await back.getByLabel('読み出しの引数').fill(['show', STORED].join('\n'));
    await page.getByRole('button', { name: '次へ：試す状況を選ぶ', exact: true }).click();
    await page.getByLabel(fault).check();
    await page.getByRole('button', { name: '次へ：判定の条件を確認する', exact: true }).click();
    while (await form.getByRole('button', { name: 'この条件を外す' }).count()) await form.getByRole('button', { name: 'この条件を外す' }).first().click();
    for (const [i, c] of checks.entries()) {
      await form.getByRole('button', { name: '条件を追加' }).click();
      const row = form.locator('.check-row').nth(i);
      await row.getByLabel('確認する範囲').fill(c.scope);
      await row.getByLabel('観測の方法').selectOption(c.method);
      await row.getByLabel('期待する結果', { exact: true }).selectOption(c.expect);
      if (c.text) await row.getByLabel('文字列').fill(c.text);
    }
  };
  const run = async () => {
    if (await page.getByRole('button', { name: '実行前に確認する' }).isVisible()) await page.getByRole('button', { name: '実行前に確認する' }).click();
    await page.getByRole('button', { name: '試験を始める', exact: true }).click();
    await page.locator('#result:not([hidden])').waitFor({ timeout: 180000 });
    return (await caseView()).workspace.evidence.at(-1).execution;
  };

  // 通常時。
  await page.locator(`[data-check="${saved}"]`).getByRole('button', { name: 'この動作を検証する' }).click();
  await fill({ fault: /故障なし（通常時）/, checks: [
    { scope: '受付', method: 'stderr', expect: 'contains', text: 'Received Store Response (Status: 0x0000 - Success)' },
    { scope: '書き込み', method: 'stored_file', expect: 'contains', text: PATIENT_ID },
    { scope: '後からの読み出し', method: 'readback_output', expect: 'contains', text: PATIENT_ID },
    { scope: '受信側のエラー', method: 'server_output', expect: 'not_contains', text: 'E: ' },
  ] });
  await page.getByRole('button', { name: '実行前に確認する', exact: true }).click();
  await page.getByText('実行する内容', { exact: true }).waitFor();
  const preview = await page.locator('#trial-body').innerText();
  assert.match(preview, /pydicom\/pydicom のコミット 20aa4b73aef76bffb6fbc46a8827588dfa9bb89d の src/);
  assert.match(preview, /python3 -S -P -m pynetdicom storescp \{\{port\}\}/);
  await shot('trial-normal');
  let trace = await run();
  let result = await page.locator('#result-body').innerText();
  assert.match(result, /現在の判定[\s\S]*条件を満たす/, result);
  assert.equal(trace.python.packages[1].commit, PYDICOM);
  assert.equal(trace.python.packages[1].metadata, 'pydicom-3.0.2.dist-info');
  assert.equal(trace.python.packages[0].metadata, 'pynetdicom-3.0.4.dist-info');
  assert.equal(trace.trial.server.ready, true);
  assert.equal(trace.trial.server.stopped_by_platform, true);
  assert.equal(trace.trial.state.status, 'file');
  assert.match(trace.trial.readback.stdout_excerpt, /Patient ID\s+LO: '1CT1'/);
  assert.equal(trace.python.launches.find(l => l.role === 'readback').path, 'src/pydicom/cli/main.py');
  log(`通常時: 受信 ${trace.trial.server.ready_after_ms}ms で準備、保存先の読み出しと pydicom show で患者IDを確認し「条件を満たす」`);
  await shot('result-normal');

  // 故障: 保存先のディレクトリを書き込み不可にする。
  await page.locator('#steps').getByRole('link', { name: /検証する動作を決める/ }).click();
  await page.locator(`[data-check="${failure}"]`).getByRole('button', { name: 'この動作を検証する' }).click();
  await fill({ fault: /保存先のディレクトリを書き込み不可にする/, readback: false, checks: [
    { scope: '終了コードで失敗が分かる', method: 'exit_code', expect: 'nonzero' },
    { scope: '送信側の表示で失敗が分かる', method: 'stderr', expect: 'contains', text: 'Failure' },
    { scope: '書き込み', method: 'stored_file', expect: 'not_contains', text: PATIENT_ID },
    { scope: '受信側のエラー', method: 'server_output', expect: 'contains', text: 'Permission denied' },
  ] });
  trace = await run();
  result = await page.locator('#result-body').innerText();
  assert.match(result, /現在の判定[\s\S]*条件を満たさない/, result);
  assert.equal(trace.exit_code, 0, '受信側が失敗を返しても storescu は終了コード0で終わる');
  assert.equal(trace.trial.fault_effective, true);
  assert.match(trace.stderr_excerpt, /Received Store Response \(Status: 0xA700 - Failure\)/);
  assert.match(trace.trial.server.stderr_excerpt, /Permission denied/);
  const observed = Object.fromEntries(trace.trial.results.map(r => [r.id, r.observed]));
  assert.deepEqual(observed, { c1: 'fail', c2: 'pass', c3: 'pass', c4: 'pass' });
  log('保存先が書き込み不可: 受信側は 0xA700 を返し、送信側は表示に Failure を出すが終了コード0。「条件を満たさない」');
  await shot('result-read-only');

  // 故障: 受信サーバーを起動しない。
  await page.locator('#steps').getByRole('link', { name: /実際に試す/ }).click();
  await fill({ fault: /受信サーバーなどの事前起動を行わない/, readback: false, checks: [
    { scope: '終了コードで失敗が分かる', method: 'exit_code', expect: 'nonzero' },
    { scope: '書き込み', method: 'stored_file', expect: 'not_contains', text: PATIENT_ID },
  ] });
  trace = await run();
  result = await page.locator('#result-body').innerText();
  assert.equal(trace.trial.server.started, false);
  assert.equal(trace.trial.fault_effective, true);
  log(`受信サーバーなし: 終了コード ${trace.exit_code}。判定は ${/現在の判定[\s\S]*?(条件を満たす|条件を満たさない|判定できない)/.exec(result)?.[1]}`);
  await shot('result-no-server');

  assert.deepEqual(errors, [], errors.join('\n'));
  log('画面のエラーなし');
} catch (error) {
  await page?.screenshot({ path: join(output, 'dicom-failure.png'), fullPage: true });
  throw error;
} finally {
  await browser?.close();
  server?.kill();
}
