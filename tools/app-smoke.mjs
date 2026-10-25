// 理解と試験の画面を、公開リポジトリ sjl/t の固定コミットで実際に操作する。
// GitHubへの接続が必要。対象の実行は試行用の作業ディレクトリで行い、既存のデータは使わない。
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, mkdir, writeFile, readdir, rm, symlink } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve, join } from 'node:path';
import { chromium } from 'playwright';

const root = resolve(import.meta.dirname, '..');
const binary = process.env.REVIEW_BINARY || join(root, 'target/debug/mugiurushi');
const output = join(root, 'test-results');
const REPO = 'https://github.com/sjl/t';
const BEFORE = '70817fd';
const AFTER = '815ccaf';
const temp = await mkdtemp(join(tmpdir(), 'eel-app-'));
await mkdir(output, { recursive: true });
const log = (...a) => console.log('・', ...a);
async function openDetails(summary) {
  if (!await summary.evaluate(node => node.parentElement.open)) await summary.click();
}

const servers = [];

function start(data, env = {}) {
  const server = spawn(binary, ['app', data, '--port', '0', '--execution-mode', 'local'], { stdio: ['ignore', 'pipe', 'pipe'], env: { ...process.env, ...env } });
  servers.push(server);
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
  const alert = page.locator('#alert');
  const shot = name => page.screenshot({ path: join(output, `app-${name}.png`), fullPage: true });

  // 初期画面: URLと短い目的だけ。手順は読み込むまで出さない。
  assert.equal(await page.locator('#steps').isHidden(), true);
  assert.equal(await page.locator('#current').isHidden(), true);
  await page.keyboard.press('Tab');
  assert.equal(await page.evaluate(() => document.activeElement.textContent), '本文へ移動');
  await shot('initial');

  // URLの形が違うときは、送信せずに入力欄の下へ理由を出す。
  await page.locator('#repo-url').fill('github.com/sjl');
  await page.getByRole('button', { name: '読み込む', exact: true }).click();
  assert.match(await page.locator('#repo-url-error').innerText(), /https:\/\/github\.com\/所有者\/リポジトリ/);
  assert.equal(await page.locator('#repo-url').getAttribute('aria-invalid'), 'true');
  log('URLの形の誤りを入力欄の下に表示した');

  // 存在しないリポジトリ: 取得に失敗したことを表示し、何も読み込まない。
  await page.locator('#repo-url').fill('https://github.com/sjl/eel-this-repository-does-not-exist');
  await page.getByRole('button', { name: '読み込む', exact: true }).click();
  await alert.locator('.dads-notification-banner').waitFor();
  const fetchError = await alert.innerText();
  assert.match(fetchError, /操作を完了できませんでした/);
  assert.match(fetchError, /見つかりません/, '存在しないリポジトリとしての失敗ではない');
  assert.equal(await page.locator('#steps').isHidden(), true);
  log('取得の失敗を表示した:', fetchError.replace(/\s+/g, ' '));
  await shot('fetch-failure');

  // 1. 修正前のコミットを開く。
  await page.locator('#repo-url').fill(REPO);
  await page.locator('#repo-ref').fill(BEFORE);
  await page.getByRole('button', { name: '読み込む', exact: true }).click();
  await page.locator('#understand:not([hidden])').waitFor({ timeout: 60000 });
  assert.match(await page.locator('#current-commit').innerText(), new RegExp(BEFORE));
  await page.getByText('読んだ範囲とコードの版', { exact: true }).click();
  await page.getByText('起動のしかたと入口', { exact: true }).click();
  await page.getByText('入力から保存・送信まで', { exact: true }).click();
  const understand = await page.locator('#understand-body').innerText();
  await writeFile(join(output, 'app-understand.txt'), understand);
  for (const text of ['読んだ範囲', '目的', '起動のしかたと入口', '主な構成と役割', '入力から保存・送信まで', 'TaskDict.write', 'コードで確認', '作者の説明', '解析の推測'])
    assert.ok(understand.includes(text), `理解の画面に「${text}」がない`);
  assert.match(understand, /件のうち \d+ 件は、固定コミットのファイルと行に存在することを麦漆が確かめました/);
  log('修正前のコミットを読み、目的・起動・構成・処理の流れを根拠付きで表示した');
  await shot('understand');

  // キーボードで手順を切り替え、移った先の見出しへフォーカスする。フォーカスの枠を表示する。
  await page.locator('#steps a[href="#expect"]').focus();
  await page.keyboard.press('Enter');
  assert.equal(await page.evaluate(() => document.activeElement.id), 'expect');
  assert.equal(await page.locator('#steps li[data-step="expect"]').getAttribute('aria-current'), 'step');
  await page.locator('#steps a[href="#understand"]').focus();
  await page.keyboard.press('Enter');
  assert.equal(await page.evaluate(() => document.activeElement.id), 'understand');
  await page.locator('#question').focus();
  await page.keyboard.press('Tab');
  const focused = await page.evaluate(() => ({ text: document.activeElement.textContent, outline: getComputedStyle(document.activeElement).outlineStyle }));
  assert.equal(focused.text, '調べる');
  assert.notEqual(focused.outline, 'none');
  log('キーボードで手順を切り替え、フォーカスの枠が表示された');

  // 質問からコードへ。
  await page.locator('#question').fill('保存できなかったらどうなる？');
  await page.getByRole('button', { name: '調べる' }).click();
  await page.locator('#answer .box').first().waitFor();
  const answer = await page.locator('#answer').innerText();
  assert.match(answer, /BadFile/);
  assert.match(answer, /_main/);
  assert.match(answer, /捕捉する記述/);
  log('「保存できなかったらどうなる？」から BadFile と _main での捕捉を示した');
  await page.getByRole('button', { name: 't.py の 244行目のコードを開く' }).first().click();
  await page.locator('#code:not([hidden]) .code-line[data-line="244"][data-mark]').waitFor();
  assert.match(await page.locator('#code-meta').innerText(), /t\.py/);
  await page.getByRole('button', { name: '244行目を選ぶ' }).click();
  await page.locator('#code-function').click();
  await page.locator('#current-function', { hasText: 'TaskDict.write' }).waitFor();
  assert.match(await page.locator('#answer').innerText(), /受け取る値（引数）[\s\S]*delete_if_empty/);
  await shot('question-and-code');

  // AIは未設定。静的解析は続ける。
  assert.equal(await page.locator('#ask-ai').isHidden(), true);
  assert.match(await page.locator('#ai-status').innerText(), /運営者によるサーバー設定が必要/);

  // 2. 確認したい動作を書く。名前がないと記録しない。
  await page.getByRole('link', { name: /検証する動作を決める/ }).click();
  await page.locator('#expect-text').fill('利用者がタスクを追加したとき、後から一覧で同じタスクを読み出せる');
  await page.getByRole('button', { name: 'この動作を確かめる' }).click();
  assert.match(await alert.innerText(), /名前を入力してください/);
  await page.locator('#person').getByLabel('名前').fill('架空の運用担当');
  await page.locator('#person').getByLabel('役割').selectOption('operator');
  await page.getByRole('button', { name: 'この動作を確かめる' }).click();
  await page.locator('[data-check="sjl-t-1"]').waitFor();
  await page.locator('#steps a[href="#expect"]').click();
  await page.locator('#expect-text').fill('保存できなかったとき、利用者に失敗が分かる');
  await page.getByRole('button', { name: 'この動作を確かめる' }).click();
  await page.locator('[data-check="sjl-t-2"]').waitFor();
  let view = await caseView();
  assert.equal(view.workspace.checks.length, 2);
  assert.ok(view.workspace.statements.every(s => s.recorded_by.includes('架空の運用担当') && s.confirmations.length === 0));
  assert.ok(view.statements.every(s => s.status === 'unconfirmed'));
  log('期待を2件記録した。確認者はまだいない（未確認）');

  await page.locator('#steps a[href="#expect"]').click();
  // 技術者が実装と確認方法を対応づける。
  const as = async (name, role) => { await page.locator('#person').getByLabel('名前').fill(name); await page.locator('#person').getByLabel('役割').selectOption(role); };
  const linkAs = async (checkId, line, explain, method) => {
    await as('架空の開発者', 'developer');
    await page.getByRole('button', { name: `${line}行目を選ぶ` }).click();
    await page.locator('#code-link').click();
    const card = page.locator(`[data-check="${checkId}"]`);
    await card.getByLabel('この動作を実装しているコードと処理内容').fill(explain);
    await card.getByLabel('どう確かめるか（確認の方法）').fill(method);
    await card.getByRole('button', { name: '対応づけを記録' }).click();
    await card.locator('.statement[data-kind="implementation"]').waitFor({ state: 'attached' });
    await openDetails(card.getByText('確認項目と説明の履歴', { exact: true }));
    await card.locator('.statement[data-kind="implementation"]').waitFor();
  };
  await linkAs('sjl-t-1', 244, 'TaskDict.write が tasks ファイルを書き込み用に開き、タスクを1行ずつ書き込む。一覧の表示は同じファイルを読み直す。', '試験用の値でタスクを追加し、麦漆が保存先 tasks を直接読み出して値があること、別のプロセスで一覧を表示して値が出ることを確かめる。');
  await linkAs('sjl-t-2', 244, '書き込みに失敗すると BadFile が発生し、_main がエラー表示に変える。終了コードは読んだ範囲では指定されていない。', '保存先を書き込み不可にして追加し、終了コードが0以外か、エラー表示があるか、保存先に値が書かれていないかを確かめる。');
  view = await caseView();
  assert.equal(view.workspace.checks.find(c => c.id === 'sjl-t-1').source.commit.slice(0, 7), BEFORE);

  // 運用担当が対応づけを読んで確認する。開発者の確認がないので「両方確認」にならない。
  await as('架空の運用担当', 'operator');
  await page.locator('[data-check="sjl-t-1"]').getByText('確認項目と説明の履歴', { exact: true }).click();
  const impl = page.locator('[data-check="sjl-t-1"] .statement[data-kind="implementation"]');
  await impl.getByRole('button', { name: '内容を確認したことを記録する' }).click();
  await page.locator('[data-check="sjl-t-1"] .statement[data-kind="implementation"]', { hasText: '架空の運用担当（運用担当）が事例の版' }).waitFor({ state: 'attached' });
  await openDetails(page.locator('[data-check="sjl-t-1"]').getByText('確認項目と説明の履歴', { exact: true }));
  view = await caseView();
  const implView = view.statements.find(s => s.id === view.workspace.statements.find(x => x.kind === 'implementation' && x.check_ids.includes('sjl-t-1')).id);
  assert.equal(implView.status, 'partly_confirmed');

  // 食い違いと未回答の質問。
  const card2 = page.locator('[data-check="sjl-t-2"]');
  await card2.getByText('食い違いを指摘する').click();
  await card2.getByLabel('確認項目・説明・コードが一致しない箇所').fill('期待は「利用者に失敗が分かる」だが、呼び出し側のプログラムが終了コードで失敗を知れるかは説明にない。');
  await card2.getByRole('button', { name: '指摘を記録' }).click();
  await card2.locator('.statement[data-kind="discrepancy"]').waitFor({ state: 'attached' });
  await openDetails(card2.getByText('確認項目と説明の履歴', { exact: true }));
  await card2.getByText('答えが必要な質問を残す').click();
  await card2.getByLabel('質問').fill('「失敗が分かる」には、エラー表示だけでなく終了コードが0以外であることも含めるか？');
  await card2.getByRole('button', { name: '質問を記録' }).click();
  await card2.getByText('未回答の質問').waitFor();
  view = await caseView();
  assert.equal(view.checks.find(c => c.check_id === 'sjl-t-2').verdict, 'unknown');
  await card2.getByText('回答する').click();
  await card2.getByLabel('回答').fill('含める。ほかのプログラムから使うときは終了コードで判断するため。');
  await card2.getByRole('button', { name: '回答を記録' }).click();
  await card2.getByText('未回答の質問').waitFor({ state: 'detached' });
  await card2.getByText('確認項目と説明の履歴', { exact: true }).click();
  await card2.locator('.statement[data-kind="discrepancy"]').getByText('対応を記録する').click();
  await card2.locator('.statement[data-kind="discrepancy"]').getByLabel('対応の内容').fill('質問への回答で、終了コードも判定の条件に含めることにした。');
  await card2.locator('.statement[data-kind="discrepancy"]').getByRole('button', { name: '対応を記録' }).click();
  await card2.locator('.statement[data-kind="discrepancy"]', { hasText: '対応: 質問への回答で' }).waitFor({ state: 'attached' });
  await openDetails(card2.getByText('確認項目と説明の履歴', { exact: true }));
  log('対応づけ・確認・食い違い・質問と回答・対応を記録した');
  await shot('expect');

  // 古い版の記録で操作すると409で止まり、読み直せる。
  const revision = (await caseView()).workspace.revision;
  const external = await context.request.post(url + '/api/v1/change', { headers: { 'X-Review-Token': token, Origin: url, 'Content-Type': 'application/json' }, data: { expected_revision: revision, actor: '架空の別の利用者', reason: '同時の更新を試す', action: { op: 'set_context', key: 'concurrent', value: 'yes' } } });
  assert.equal(external.status(), 200);
  await page.locator('[data-check="sjl-t-1"]').getByText('確認項目と説明の履歴', { exact: true }).click();
  await impl.getByRole('button', { name: '内容を確認したことを記録する' }).click();
  await alert.getByText('ほかの操作で記録が更新されています').waitFor();
  await alert.getByRole('button', { name: '最新の記録を読み直す' }).click();
  await alert.locator('.dads-notification-banner').waitFor({ state: 'detached' });
  log('古い版の記録での更新を409で止め、読み直した');

  // 3. 通常時の試行。
  await as('架空の開発者', 'developer');
  await page.locator('[data-check="sjl-t-1"]').getByRole('button', { name: 'この動作を検証する' }).click();
  await page.locator('#trial:not([hidden])').waitFor();
  await page.getByRole('button', { name: '通常時の設定例を使う' }).click();
  await page.locator('#trial-body form').waitFor();
  await page.getByText('実行する内容', { exact: true }).waitFor({ state: 'attached' });
  const preview = await page.locator('#trial-body').textContent();
  assert.match(preview, /python3 t\.py --task-dir \. --list tasks \{\{test_value\}\}/);
  assert.match(preview, /依存パッケージのインストールは行いません/);
  assert.match(await page.locator('#trial-body').textContent(), /cram tests\/\*\.t（cram形式のテスト）[\s\S]*インストールしないため、既存テストは実行しない/);
  await shot('trial-normal');
  const runTrial = async () => {
    if (await page.getByRole('button', { name: '次へ：試す状況を選ぶ', exact: true }).isVisible()) await page.getByRole('button', { name: '次へ：試す状況を選ぶ', exact: true }).click();
    await page.getByRole('button', { name: '次へ：判定の条件を確認する', exact: true }).click();
    await page.getByRole('button', { name: '実行前に確認する' }).click();
    await page.getByRole('button', { name: '試験を始める', exact: true }).click();
    await page.locator('#result:not([hidden])').waitFor({ timeout: 120000 });
  };
  await runTrial();
  let result = await page.locator('#result-body').innerText();
  assert.match(result, /現在の判定[\s\S]*条件を満たす/);
  // 結果の取得元は保存済みのサーバー記録を表示する。画面のラベルとは区別する。
  assert.match(result, /基盤が保存先を直接読み出した結果/);
  assert.match(result, /別のプロセスで対象の読み出し機能を動かした結果/);
  log('通常時: 保存先の直接読み出しと別プロセスの読み出しで「条件を満たす」');
  await shot('result-normal');

  // 観測の欠落: 基盤の読み出しを止めると、前の成功で補わず「判定できない」。
  await page.locator('#steps').getByRole('link', { name: /実際に試す/ }).click();
  await page.getByRole('button', { name: '次へ：試す状況を選ぶ', exact: true }).click();
  await page.getByLabel(/麦漆による保存先の読み出しを止める/).check();
  await runTrial();
  result = await page.locator('#result-body').innerText();
  assert.match(result, /現在の判定[\s\S]*検証未通過/);
  await page.getByText('過去の試行を見る', { exact: true }).click();
  assert.match(await page.locator('#result-body').innerText(), /この実行結果は現在の確認項目に適用できません/);
  log('観測の欠落: 前の成功を使わず「判定できない」');

  // 故障: 修正前のコミットで書き込み不可にする。
  await page.locator('#steps').getByRole('link', { name: /検証する動作を決める/ }).click();
  await page.locator('[data-check="sjl-t-2"]').getByRole('button', { name: 'この動作を検証する' }).click();
  await page.getByRole('button', { name: '保存失敗時の設定例を使う' }).click();
  await runTrial();
  result = await page.locator('#result-body').innerText();
  assert.match(result, /現在の判定[\s\S]*条件を満たさない/);
  assert.match(result, /Permission denied/);
  view = await caseView();
  const failed = view.workspace.evidence.filter(e => e.check_id === 'sjl-t-2').at(-1).execution;
  assert.equal(failed.exit_code, 0);
  assert.equal(failed.trial.fault_effective, true);
  log('故障（書き込み不可）: 修正前はエラー表示があるが終了コード0のため「条件を満たさない」');
  await shot('result-fault-before');

  // 結果への異論を書く。
  const evCard = page.locator('[data-evidence]').first();
  await evCard.locator('summary', { hasText: /^この結果への質問・異論/ }).click();
  await evCard.getByLabel('質問・異論').fill('エラー表示が出ているので、人が使う場合は失敗に気づける。終了コードだけで「満たさない」としてよいか。');
  await as('架空の運用担当', 'operator');
  await evCard.getByRole('button', { name: '質問・異論を記録' }).click();
  await evCard.locator('.statement[data-kind="comment"]').waitFor({ state: 'attached' });
  await openDetails(evCard.locator('summary', { hasText: /^この結果への質問・異論/ }));
  await evCard.locator('.statement[data-kind="comment"]').waitFor();

  // 結果からコードへ戻る。
  await evCard.getByRole('button', { name: /対応づけたコード/ }).click();
  await page.locator('#code:not([hidden]) .code-line[data-line="244"][data-mark]').waitFor();

  // 修正後のコミットで再試験する。
  await page.getByText('修正した版で再試験する', { exact: true }).click();
  await page.getByLabel('修正した版（ブランチ、タグ、またはコミット）').fill(AFTER);
  await page.getByRole('button', { name: '修正した版を読み込む' }).click();
  await page.getByText('新しいコミットでの対応づけを確かめてください').waitFor({ timeout: 60000 });
  assert.match(await page.locator('#current-commit').innerText(), new RegExp(AFTER));
  // 修正後は書き込みの行が250行目に移っているため、行を選び直す。
  await page.getByRole('button', { name: '250行目を選ぶ' }).click();
  await page.locator('#code-link').click();
  const relink = page.locator('[data-check="sjl-t-2"]');
  await relink.getByLabel('この動作を実装しているコードと処理内容').fill('修正後も TaskDict.write が書き込み、失敗すると BadFile を _main が _die に渡して終了コード1で終わる。');
  await relink.getByLabel('どう確かめるか（確認の方法）').fill('前と同じ故障・判定の条件で再試験する。');
  await relink.getByRole('button', { name: '対応づけを記録' }).click();
  await relink.locator('.statement[data-kind="implementation"]').nth(1).waitFor({ state: 'attached' });
  await openDetails(relink.getByText('確認項目と説明の履歴', { exact: true }));
  await relink.getByRole('button', { name: 'この動作を検証する' }).click();
  await page.locator('#trial-body form').waitFor();
  await page.getByText('実行する内容', { exact: true }).waitFor({ state: 'attached' });
  const retestPreview = await page.locator('#trial-body').textContent();
  assert.match(retestPreview, new RegExp(`コミット ${AFTER}`));
  assert.match(retestPreview, /保存先を書き込み不可にする/);
  await runTrial();
  result = await page.locator('#result-body').innerText();
  assert.match(result, /現在の判定[\s\S]*条件を満たす/);
  view = await caseView();
  const trials = view.workspace.evidence.filter(e => e.check_id === 'sjl-t-2');
  assert.equal(trials.length, 2);
  assert.equal(trials[1].execution.exit_code, 1);
  log('修正後のコミットで再試験し「条件を満たす」。前の試行も残っている');

  // 比べる。
  await page.getByRole('button', { name: '比べる' }).click();
  await page.getByText('異なる項目・確認できない項目').waitFor();
  const diff = await page.locator('#result-body').innerText();
  assert.match(diff, /コード参照（コミット・ファイル・行）/);
  assert.match(diff, /終了コード/);
  assert.match(diff, /原因|因果/);
  log('二つの試行を比べ、コード参照と終了コードの違いを示した');
  const beforeTrial = page.getByLabel('前の試行', { exact: true });
  const afterTrial = page.getByLabel('後の試行', { exact: true });
  const beforeId = await beforeTrial.inputValue(), afterId = await afterTrial.inputValue();
  const comparison = page.locator('[data-trial-comparison]');
  await afterTrial.selectOption(beforeId);
  assert.equal(await comparison.innerText(), '', '選択を変えたら前の比較を消す');
  await afterTrial.selectOption(afterId);
  let releaseResponse, responseHeld;
  const held = new Promise(resolve => { responseHeld = resolve; });
  const release = new Promise(resolve => { releaseResponse = resolve; });
  await context.route('**/api/v1/compare', async route => {
    const response = await route.fetch(); responseHeld(); await release; await route.fulfill({ response });
  });
  await page.getByRole('button', { name: '比べる', exact: true }).click();
  await held;
  await afterTrial.selectOption(beforeId);
  const responseReturned = page.waitForResponse(r => r.url().endsWith('/api/v1/compare'));
  releaseResponse(); await responseReturned;
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  assert.equal(await comparison.innerText(), '', '選択前の遅い応答を表示しない');
  await context.unroute('**/api/v1/compare');
  await afterTrial.selectOption(afterId);
  await page.getByRole('button', { name: '比べる', exact: true }).click();
  await page.getByText('異なる項目・確認できない項目').waitFor();
  assert.deepEqual((await caseView()).workspace, view.workspace, '比較と選択で記録を変更しない');
  log('比較の選択変更で前の表示を消し、選択前の遅い応答を採用しない');


  // 判断。合意済みにはならない。
  await as('架空の開発者', 'developer');
  await page.getByText('判断を記録・確認する', { exact: true }).click();
  await page.getByLabel('判断').selectOption('verify');
  await page.getByLabel(/^理由\s*※必須$/).fill('修正後は条件を満たした。人が使う場合の扱いへの異論が未対応のため、運用担当の確認を待つ。');
  await page.getByRole('button', { name: '判断を記録' }).click();
  await page.getByText('記録した判断（1 件）').waitFor({ state: 'attached' });
  await openDetails(page.getByText('判断を記録・確認する', { exact: true }));
  view = await caseView();
  assert.ok(!JSON.stringify(view).includes('合意済み'));
  assert.ok(view.statements.every(s => s.status !== 'both_confirmed'));
  await shot('result-after-fix');

  // 狭い画面と文字の拡大。
  // 幅320pxは、1280pxの画面を400%に拡大したときの幅に当たる。文字だけの200%は幅1280pxで確かめる。
  for (const [width, fontSize] of [[390, ''], [320, ''], [1280, '200%']]) {
    await page.setViewportSize({ width, height: 900 });
    await page.evaluate(size => { document.documentElement.style.fontSize = size; }, fontSize);
    await page.waitForTimeout(100);
    for (const step of ['understand', 'expect', 'trial', 'result']) {
      await page.locator(`#steps a[href="#${step}"]`).click();
      const layout = await page.evaluate(() => ({
        overflow: document.documentElement.scrollWidth - document.documentElement.clientWidth,
        wide: (() => { if (document.documentElement.scrollWidth - document.documentElement.clientWidth <= 1) return []; const main = document.querySelector('main'); const base = main.scrollWidth; let node = main; const path = []; for (let d = 0; d < 30; d++) { let found = null; for (const c of node.children) { const prev = c.style.display; c.style.display = 'none'; const w = main.scrollWidth; c.style.display = prev; if (w < base) { found = c; break; } } if (!found) break; node = found; path.push(`${node.tagName.toLowerCase()}.${node.className}#${node.id}`); } return [...path, node.textContent.slice(0, 60)]; })(),
        stepsClipped: (n => n.scrollWidth - n.clientWidth)(document.getElementById('steps')),
        narrowHeadings: [...document.querySelectorAll('.dads-notification-banner__heading-text')].filter(e => e.offsetParent && e.getBoundingClientRect().width < 120).map(e => e.textContent),
      }));
      assert.ok(layout.overflow <= 1, `幅${width}px・文字${fontSize || '100%'}の「${step}」で横に ${layout.overflow}px はみ出している: ${layout.wide.join(' / ')}`);
      assert.ok(layout.stepsClipped <= 1, `幅${width}px・文字${fontSize || '100%'}で手順の名前が隠れている`);
      assert.deepEqual(layout.narrowHeadings, [], `幅${width}px・文字${fontSize || '100%'}の「${step}」で通知の見出しが細くなっている`);
    }
    await shot(`result-${width}${fontSize ? '-text200' : ''}`);
  }
  await page.evaluate(() => { document.documentElement.style.fontSize = ''; });
  await page.setViewportSize({ width: 1440, height: 1000 });
  log('幅390px・320px、文字200%（幅1280px）の4つの手順で、横のはみ出しと見出しの崩れがない');

  // A slow case response must not send the user back after they choose the trial step.
  let releaseCase, caseArrived;
  const heldCase = new Promise(resolve => { releaseCase = resolve; });
  const caseReceived = new Promise(resolve => { caseArrived = resolve; });
  const holdCase = async route => {
    const response = await route.fetch();
    caseArrived();
    await heldCase;
    await route.fulfill({ response });
  };
  await page.route('**/api/v1/case', holdCase);
  // 実行が未対応の言語。
  await page.getByRole('link', { name: '対象を選び直す', exact: true }).click();
  await page.locator('#repo-url').fill('https://github.com/spf13/cobra');
  await page.locator('#repo-ref').fill('');
  await page.getByRole('button', { name: '読み込む', exact: true }).click();
  await page.locator('#current-repository', { hasText: 'spf13/cobra' }).waitFor({ timeout: 60000 });
  assert.equal(await page.locator('#code').isHidden(), true, '前のリポジトリのコードを表示したままにしない');
  assert.match(await page.locator('#understand-body').innerText(), /Go（\d+ ファイル）: Go の実行は未対応/);
  await caseReceived;
  assert.match(await page.locator('#status').innerText(), /GitHubから読み込んでいます/);
  await page.locator('#steps').getByRole('link', { name: /実際に試す/ }).click();
  releaseCase();
  await page.locator('#status', { hasText: /コミット .*固定して読み込みました/ }).waitFor();
  await page.getByText('この対象の実行は未対応です').waitFor();
  assert.equal(await page.locator('#steps li[data-step=trial]').getAttribute('aria-current'), 'step');
  await page.unroute('**/api/v1/case', holdCase);
  const unread = page.locator('#understand-body details', { has: page.locator('summary', { hasText: /^読み取っていないファイル（/ }) }).last();
  log('Goのリポジトリで「実行は未対応」を表示した');
  await page.locator('#steps').getByRole('link', { name: /ソフトウェアを理解する/ }).click();
  await openDetails(page.getByText('読んだ範囲とコードの版', { exact: true }));
  const unreadCount = Number(/（(\d+)\s*件）/.exec(await unread.locator(':scope > summary').textContent())[1]);
  assert.ok(unreadCount > 0, '本文の読取上限を超えるリポジトリで、未読のファイルを表示していない');
  await openDetails(unread.locator(':scope > summary'));
  await unread.locator('li', { hasText: /\.go（解析に対応していない種類/ }).first().getByRole('button', { name: '読む' }).click();
  await page.locator('#code-meta', { hasText: /\.go（コミット/ }).waitFor();
  assert.match(await page.locator('#code-view').innerText(), /package cobra/);
  log(`未読のファイル ${unreadCount} 件を表示し、そのうち1件を画面から読んだ`);

  // 取得の失敗（400）と古い版での更新（409）は、意図して起こした応答。
  assert.deepEqual(errors.filter(e => !/status of (400|409) /.test(e)), []);
  assert.equal(errors.length, 2);

  // AIの接続先が応答しないとき。静的解析の結果は表示し続ける。
  const keyFile = join(temp, 'ai-key');
  await writeFile(keyFile, 'synthetic-key-for-test\n', { mode: 0o600 });
  const aiUrl = await start(join(temp, 'ai-data'), { EEL_AI_BASE_URL: 'http://127.0.0.1:9/v1', EEL_AI_MODEL: 'synthetic-model', EEL_AI_API_KEY_FILE: keyFile });
  const aiPage = await context.newPage();
  await aiPage.goto(aiUrl + '/workbench');
  await aiPage.locator('#repo-url').fill(REPO);
  await aiPage.locator('#repo-ref').fill(AFTER);
  await aiPage.getByRole('button', { name: '読み込む', exact: true }).click();
  await aiPage.locator('#understand:not([hidden])').waitFor({ timeout: 60000 });
  await aiPage.locator('#question').fill('タスクはどこに保存される？');
  await aiPage.getByRole('button', { name: 'AIの説明も求める' }).click();
  await aiPage.getByText('AIによる説明を表示できません').waitFor({ timeout: 60000 });
  await aiPage.getByRole('button', { name: '調べる' }).click();
  await aiPage.locator('#answer', { hasText: 'TaskDict.write' }).waitFor();
  assert.ok(!(await aiPage.locator('#answer').innerText()).includes('AIによる説明（'));
  log('AIの接続に失敗したことを表示し、静的解析の結果は表示した');
  await aiPage.screenshot({ path: join(output, 'app-ai-failure.png'), fullPage: true });

  // 認証のない要求とOriginの異なる要求は拒否する。
  assert.equal((await context.request.post(url + '/api/v1/understand', { headers: { Origin: url, 'Content-Type': 'application/json' }, data: { operation: 'open', url: REPO, reference: '' } })).status(), 403);
  assert.equal((await context.request.post(url + '/api/v1/understand', { headers: { Origin: 'http://example.invalid', 'X-Review-Token': token, 'Content-Type': 'application/json' }, data: { operation: 'open', url: REPO, reference: '' } })).status(), 403);
  log('トークンのない要求とOriginの異なる要求を拒否した');

  // 実行環境がないとき。PATHにNode.jsだけを置き、python3を見つけられない状態で同じ記録を開き直す。
  // 40文字のコミットは保存した読取記録から開くため、GitHubへは問い合わせない。
  const finalCase = await caseView();
  const fullCommit = finalCase.workspace.checks.find(c => c.id === 'sjl-t-2').source.commit;
  const main = servers[0];
  await new Promise(done => { main.once('exit', done); main.kill(); });
  const nodeOnly = join(temp, 'node-only');
  await mkdir(nodeOnly);
  await symlink(process.execPath, join(nodeOnly, 'node'));
  const bareUrl = await start(join(temp, 'data'), { PATH: nodeOnly });
  const barePage = await context.newPage();
  await barePage.goto(bareUrl + '/workbench');
  await barePage.locator('#repo-url').fill(REPO);
  await barePage.locator('#repo-ref').fill(fullCommit);
  await barePage.getByRole('button', { name: '読み込む', exact: true }).click();
  await barePage.locator('#understand:not([hidden])').waitFor({ timeout: 30000 });
  await barePage.locator('#steps a[href="#expect"]').click();
  await barePage.getByRole('button', { name: 'この動作を検証する' }).first().click();
  await barePage.getByText('実行環境がないため試せません').waitFor();
  await barePage.getByRole('button', { name: '次へ：試す状況を選ぶ', exact: true }).click();
  await barePage.getByRole('button', { name: '次へ：判定の条件を確認する', exact: true }).click();
  await barePage.getByText('実行環境がないため試せません').waitFor();
  assert.equal(await barePage.getByRole('button', { name: '実行前に確認する' }).isDisabled(), true);
  await barePage.screenshot({ path: join(output, 'app-no-runtime.png'), fullPage: true });
  log('python3が見つからないことを表示し、実行のボタンを止めた');

  // 試用計画から過去の検証IDを開いた場合、最新の試行へ読み替えず対象の記録を開く。
  const historical = finalCase.workspace.evidence.find(e => e.check_id === 'sjl-t-2');
  const evidenceUrl = new URL('/workbench', bareUrl);
  for (const [key, value] of Object.entries({ url: REPO, ref: historical.source.commit, check: historical.check_id, evidence: historical.id })) evidenceUrl.searchParams.set(key, value);
  evidenceUrl.hash = 'result';
  await barePage.goto(evidenceUrl.href);
  const historicalCard = barePage.locator('[data-evidence]').filter({ has: barePage.getByRole('heading', { name: `試行 ${historical.manifest.correlation.run_id}`, exact: true }) });
  await historicalCard.waitFor({ state: 'visible' });
  assert.equal(await historicalCard.evaluate(node => document.activeElement === node), true);
  log('試用計画から指定した過去の検証IDを開き、対応する試行へフォーカスした');

  // 試行は試験用のディレクトリに置き、データの保存先の外へ書いていない。
  const runs = await readdir(join(temp, 'data', 'case', 'runs'));
  assert.equal(runs.length, 4);
  console.log('app-smoke: ok');
} catch (error) {
  await page?.screenshot({ path: join(output, 'app-failure.png'), fullPage: true }).catch(() => {});
  throw error;
} finally {
  await browser?.close();
  for (const s of servers) s.kill();
  if (!process.env.KEEP_APP_SMOKE) await rm(temp, { recursive: true, force: true });
  else console.log('kept', temp);
}
