// 理解と試験の画面。解析・照合・判定はサーバーのAPIが行い、この画面は結果を表示して操作を送る。
// 画面の文字はtextContentで入れ、取得したコードや記録をHTMLとして解釈しない。
const token = document.querySelector('meta[name="review-token"]').content;
const $ = id => document.getElementById(id);
const state = {
  source: null, overview: null, overviewRefs: null, plan: null, view: null, execution: null, ai: null,
  files: new Map(), code: null, selectedLine: null, fn: null, checkId: null, pendingRef: null, pendingAI: null, aiSequence: 0, lastRun: null, step: 'understand',
};

const ROLE = { operator: '運用担当', developer: '開発者', other: 'その他' };
const KIND_LABEL = { code: 'コードで確認', author: '作者の説明', inference: '解析の推測', ai: 'AIの説明' };
const KIND_TITLE = {
  code: 'コードの記述から確認した事実。実行して確かめたものではない',
  author: 'READMEやコメントなど作者の説明。コードで確かめていない',
  inference: '静的な解析からの推測。実行して確かめていない',
  ai: 'AIによる説明。示した行の存在は麦漆が確かめたが、説明の正しさは確かめていない',
};
const STATEMENT_KIND = { statement: '確認したい動作（本人の言葉）', code_fact: 'コードの事実', proposal: '提案（決定ではない）', unanswered: '未回答の質問', implementation: '実装と確認方法の説明', discrepancy: '食い違いの指摘', comment: '質問・異論' };
const VERDICT = {
  satisfied: { mark: '○', label: '条件を満たす' },
  not_satisfied: { mark: '×', label: '条件を満たさない' },
  unknown: { mark: '？', label: '検証未通過（証拠不足・判定不能）' },
};
const OBSERVED = { pass: '○ 期待どおり', fail: '× 期待と異なる', not_observed: '？ 観測できていない' };
const METHODS = [
  ['exit_code', '終了コード', '対象プログラムの報告（終了コード・出力）'],
  ['stdout', '標準出力', '対象プログラムの報告（終了コード・出力）'],
  ['stderr', '標準エラー（エラー表示）', '対象プログラムの報告（終了コード・出力）'],
  ['stored_file', '保存先の内容', '麦漆が保存先を直接読み出した結果'],
  ['readback_output', '後からの読み出し', '別のプロセスで対象の読み出し機能を動かした結果'],
  ['server_output', '先に起動するプロセスの出力', '先に起動するプロセス（受信側など）の出力'],
  ['container_log', 'コンテナのログ', 'コンテナのログ（対象の応答とは別）'],
  ['database_query', 'データベースの直接読み出し', '麦漆がデータベースを直接読み出した結果'],
];
const SCOPES = ['受付', '書き込み', '後からの読み出し', '終了コードで失敗が分かる', 'エラー表示で失敗が分かる', '受信側のエラー'];
const UNANALYZED = '解析に対応していない種類（未対応の言語、画像、設定など）';
const FAULT_LABEL = { normal: '故障なし（通常時）', storage_read_only: '保存先を書き込み不可にする', storage_is_directory: '保存先に同名のディレクトリを置く', skip_state_read: '麦漆による保存先の読み出しを止める（観測の欠落）', network_unavailable: '外部への通信を遮断する', storage_directory_read_only: '保存先のディレクトリを書き込み不可にする', server_not_started: '受信サーバーなどの事前起動を行わない', database_stopped: 'データベースのコンテナを止める', database_unreachable: '対象とデータベースの間の通信を切る', volume_read_only: '保存先のボリュームを読み取り専用にする' };
const LAUNCHES = [['file', 'ファイルを起動する'], ['module', 'モジュールとして起動する（python -m）'], ['script', '宣言されたコマンドを起動する（[project.scripts]）']];
const LAUNCH_LABEL = Object.fromEntries(LAUNCHES);
const DECISION = { modify: '修正する', hold: '保留する', verify: '追加で確かめる', ask: '質問する', release: '採用する' };

// ---------- 共通 ----------
function h(tag, attrs = {}, ...children) {
  const el = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs || {})) {
    if (value === undefined || value === null || value === false) continue;
    if (key === 'class') el.className = value;
    else if (key === 'text') el.textContent = value;
    else if (key === 'value') el.value = value;
    else if (key.startsWith('on')) el.addEventListener(key.slice(2), value);
    else if (value === true) el.setAttribute(key, '');
    else el.setAttribute(key, String(value));
  }
  for (const child of children.flat(Infinity)) if (child !== null && child !== undefined && child !== false) el.append(child instanceof Node ? child : String(child));
  return el;
}
function button(text, onclick, type = 'outline', size = 'sm', attrs = {}) {
  return h('button', { class: 'dads-button', 'data-type': type, 'data-size': size, type: 'button', onclick, ...attrs }, text);
}
let uid = 0;
const nextId = prefix => `${prefix}-${++uid}`;
function field({ label, id = nextId('f'), support, required, control }) {
  const supportId = support ? `${id}-support` : null;
  control.id = id;
  if (supportId) control.setAttribute('aria-describedby', supportId);
  if (required) control.required = true;
  return h('div', { class: 'dads-form-control-label', 'data-size': 'md' },
    h('label', { class: 'dads-form-control-label__label', for: id }, label, required ? h('span', { class: 'dads-form-control-label__requirement', 'data-required': 'true' }, '※必須') : null),
    support ? h('p', { id: supportId, class: 'dads-form-control-label__support-text' }, support) : null,
    h('div', {}, control));
}
function input(attrs = {}) { const i = h('input', { class: 'dads-input-text__input', type: 'text', 'data-size': 'md', ...attrs }); const wrap = h('span', { class: 'dads-input-text' }, i); wrap.input = i; return wrapField(wrap, i); }
function textarea(attrs = {}) { const t = h('textarea', { class: 'dads-textarea__textarea', rows: 3, ...attrs }); return wrapField(h('span', { class: 'dads-textarea' }, t), t); }
function select(options, attrs = {}) {
  const s = h('select', { class: 'dads-select__select', 'data-size': 'md', ...attrs }, options.map(([value, text]) => h('option', { value }, text)));
  return wrapField(h('span', { class: 'dads-select' }, h('span', { class: 'dads-select__control' }, s, chevron())), s);
}
function chevron() {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('class', 'dads-select__chevron'); svg.setAttribute('width', '16'); svg.setAttribute('height', '16'); svg.setAttribute('viewBox', '0 0 24 24'); svg.setAttribute('aria-hidden', 'true');
  const p = document.createElementNS('http://www.w3.org/2000/svg', 'path'); p.setAttribute('d', 'M12 17L3 8L4 7L12 15L20 7L21 8L12 17Z'); p.setAttribute('fill', 'currentcolor'); svg.append(p);
  return svg;
}
// field() はラベルを control.id で結ぶ。外側の要素を渡しても内側の入力欄に id が付くようにする。
function wrapField(wrap, control) {
  Object.defineProperty(wrap, 'id', { set(v) { control.id = v; }, get() { return control.id; } });
  wrap.setAttribute = (name, value) => (name === 'aria-describedby' ? control.setAttribute(name, value) : Element.prototype.setAttribute.call(wrap, name, value));
  Object.defineProperty(wrap, 'required', { set(v) { control.required = v; } });
  wrap.control = control;
  return wrap;
}
function fill(el, ...children) { el.replaceChildren(...children.flat(Infinity).filter(c => c !== null && c !== undefined && c !== false)); }
function disclosure(summary, ...content) {
  return h('details', { class: 'dads-disclosure' }, h('summary', { class: 'dads-disclosure__summary' }, summary), h('div', { class: 'dads-disclosure__content' }, content));
}
const ICON = {
  error: 'M12 2a10 10 0 1 0 0 20a10 10 0 0 0 0-20Zm-1 5h2v7h-2Zm0 9h2v2h-2Z',
  warning: 'M12 2L1 21h22L12 2Zm-1 7h2v6h-2Zm0 8h2v2h-2Z',
  success: 'M12 2a10 10 0 1 0 0 20a10 10 0 0 0 0-20Zm-1.5 14.5L6 12l1.4-1.4l3.1 3.1l6.1-6.1L18 9Z',
  'info-1': 'M12 2a10 10 0 1 0 0 20a10 10 0 0 0 0-20Zm-1 5h2v2h-2Zm0 4h2v6h-2Z',
};
function bannerIcon(type) {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('class', 'dads-notification-banner__icon'); svg.setAttribute('viewBox', '0 0 24 24'); svg.setAttribute('aria-hidden', 'true');
  const p = document.createElementNS('http://www.w3.org/2000/svg', 'path'); p.setAttribute('d', ICON[type]); p.setAttribute('fill', 'currentcolor'); svg.append(p);
  return svg;
}
function banner(type, heading, ...body) {
  type = type === 'info' ? 'info-1' : type;
  const label = { error: 'エラー', warning: '注意', success: '完了', 'info-1': 'お知らせ' }[type];
  return h('div', { class: 'dads-notification-banner', 'data-style': 'standard', 'data-type': type },
    h('h2', { class: 'dads-notification-banner__heading' }, bannerIcon(type), h('span', { class: 'dads-notification-banner__heading-text' }, `${label}: ${heading}`)),
    h('div', { class: 'dads-notification-banner__body' }, body));
}
function setStatus(text) { $('status').textContent = text || ''; }
function showError(error, actions = []) {
  setStatus('');
  const node = banner('error', '操作を完了できませんでした', h('p', {}, error.message || String(error)), error.status === 409 ? h('p', {}, 'ほかの操作で記録が更新されています。最新の記録を読み直してから、もう一度操作してください。') : null,
    h('div', { class: 'button-row' }, error.status === 409 ? button('最新の記録を読み直す', async () => { await loadCase(); clearError(); }) : null, actions, button('閉じる', clearError, 'text')));
  $('alert').replaceChildren(node);
  node.setAttribute('tabindex', '-1');
  node.focus();
}
function clearError() { $('alert').replaceChildren(); }
async function api(method, path, body) {
  const response = await fetch(path, { method, headers: { 'X-Review-Token': token, ...(body ? { 'Content-Type': 'application/json' } : {}) }, body: body ? JSON.stringify(body) : undefined });
  let data;
  try { data = await response.json(); } catch { data = { error: `サーバーの応答を読み取れませんでした（HTTP ${response.status}）` }; }
  if (!response.ok) { const e = new Error(data.error || `HTTP ${response.status}`); e.status = response.status; throw e; }
  return data;
}
async function guarded(task) { clearError(); try { return await task(); } catch (e) { showError(e); return null; } }
function shortCommit(c) { return c ? c.slice(0, 12) : ''; }
function repoSlug(repository) { return repository.replace(/[^A-Za-z0-9]+/g, '-').replace(/^-|-$/g, '').slice(0, 80); }
function randomId(prefix) { const b = new Uint8Array(6); crypto.getRandomValues(b); return `${prefix}-${Date.now().toString(36)}-${[...b].map(x => x.toString(16).padStart(2, '0')).join('')}`; }

function basis(kind) { return h('span', { class: 'basis', 'data-kind': kind, title: KIND_TITLE[kind] || '' }, KIND_LABEL[kind] || kind); }
function refLinks(refs = []) {
  return h('span', { class: 'refs' }, refs.map(r => r.verified === false
    ? h('span', { class: 'ref-unverified' }, `（${r.path} ${r.line}行目: 固定コミットで確認できない参照。${r.reason || ''}）`)
    : h('span', { class: 'ref' }, h('button', { class: 'linklike', type: 'button', onclick: () => openCode(r.path, r.line, r.end_line || r.line), 'aria-label': `${r.path} の ${r.line}行目のコードを開く` }, `${r.path} ${r.line}${r.end_line && r.end_line !== r.line ? `〜${r.end_line}` : ''}行目`))));
}
function itemLi(it) { return h('li', {}, it.kind ? basis(it.kind) : null, it.text, ' ', refLinks(it.refs), it.storage_origin ? h('div', {}, basis(it.storage_origin.kind), it.storage_origin.text, ' ', refLinks(it.storage_origin.refs)) : null); }
function items(list, empty = '該当する記述は見つかりませんでした。') { return list?.length ? h('ul', { class: 'items' }, list.map(itemLi)) : h('p', { class: 'note' }, empty); }
function codeText(text) { return h('code', {}, text); }

// ---------- 現在の対象と手順 ----------
function renderCurrent() {
  const s = state.source;
  $('current').hidden = !s;
  if (!s) return;
  $('current-repository').textContent = s.repository;
  $('current-commit').replaceChildren(h('span', { title: s.commit }, shortCommit(s.commit)), `（${s.reference} を読んだ時点）`);
  $('current-function').textContent = state.fn ? `${state.fn.name}（${state.fn.path} ${state.fn.line}行目）` : '未選択';
  const check = currentCheck();
  $('current-check').textContent = check ? `${check.text}（${check.id}・条件の版${check.version}）` : '未選択';
  const job = state.execution?.job, evidence = check ? checkView(check.id)?.evidence.at(-1) : null;
  $('current-trial').textContent = job?.status === 'running' && job.check_id === check?.id ? `${job.id}（実行中）` : evidence ? `${evidence.run_id}${evidence.applicable ? '' : '（再確認待ち）'}` : '未実行';
}
const STEP_GUIDES = {
  understand: ['コードを読みました。まだ実行していません。', '目的と主な構成を確認したら、次に確かめたい動作を書きます。'],
  'expect-edit': ['確認項目を登録しました。次に、検証するコードを選びます。', '実装と確認方法を対応づけたら、試験の設定へ進めます。疑問や確認の記録は必要なときに開けます。'],
  expect: ['何ができればよいかを決めます。', '確認したい動作を登録し、該当するコードと検証方法を指定します。ここではまだ実行しません。'],
  trial: ['まず、何を起動して何を入力するかを確認します。', '次に試す状況、判定の条件を順に決めます。この画面ではまだ実行しません。'],
  'trial-fault': ['次に、試す状況を選びます。', '通常時と失敗時を別々の試行として記録します。ここではまだ実行しません。'],
  'trial-checks': ['何を観測すれば期待どおりと言えるかを決めます。', '条件を確認したら、最後に実行内容の確認画面へ進みます。'],
  result: ['実際に取得した記録から結果を見ます。', '確認できたことと未確認事項を分けて表示します。必要なら別の状況や修正後の版で試します。'],
};
function goto(step, focus = true, record = true) {
  if (!['open', 'repository'].includes(step) && (!STEP_GUIDES[step] || !state.source)) step = 'open';
  state.step = step;
  renderCurrent();
  const atStart = ['open', 'repository'].includes(step);
  const panelStep = step === 'repository' ? 'open' : step === 'expect-edit' ? 'expect' : step.startsWith('trial') ? 'trial' : step;
  $('open').hidden = !atStart;
  $('open').setAttribute('aria-labelledby', 'repository-heading');
  if (atStart) $('code').hidden = true;
  $('steps').hidden = atStart;
  $('current').hidden = atStart;
  $('person').hidden = !['expect', 'expect-edit', 'trial', 'trial-fault', 'trial-checks', 'result'].includes(step);
  $('change-target').hidden = atStart;
  $('step-guide').hidden = atStart;
  for (const id of ['understand', 'expect', 'trial', 'result']) $(id).hidden = id !== panelStep;
  for (const li of document.querySelectorAll('.steps li')) {
    if (li.dataset.step === panelStep) li.setAttribute('aria-current', 'step'); else li.removeAttribute('aria-current');
  }
  if (step === 'expect' || step === 'expect-edit') renderExpect();
  if (step.startsWith('trial')) renderTrial();
  if (step === 'result') renderResult();
  if (STEP_GUIDES[step]) $('step-guide').replaceChildren(h('strong', {}, STEP_GUIDES[step][0]), h('p', {}, STEP_GUIDES[step][1]));
  if (!atStart) fitSteps();
  if (focus) { $(panelStep).focus(); $('main').scrollIntoView({ block: 'start' }); }
  if (record) {
    const url = new URL(location.href);
    if (state.source && !atStart) { url.searchParams.set('url', 'https://github.com/' + state.source.repository); url.searchParams.set('ref', state.source.commit); }
    else url.search = '';
    if (state.checkId && !atStart) url.searchParams.set('check', state.checkId);
    url.hash = step;
    if (url.href !== location.href) history.pushState(null, '', url);
  }
}
for (const a of document.querySelectorAll('.step-link')) a.addEventListener('click', e => { e.preventDefault(); goto(a.getAttribute('href').slice(1)); });
$('change-target').addEventListener('click', e => { e.preventDefault(); goto('open'); });
async function restoreLocation() {
  const params = new URLSearchParams(location.search), step = location.hash.slice(1);
  if (!params.get('url') || step === 'open') { goto(step === 'repository' ? 'repository' : 'open', false, false); return; }
  $('repo-url').value = params.get('url'); $('repo-ref').value = params.get('ref') || '';
  if (!state.source || state.source.commit !== params.get('ref') || 'https://github.com/' + state.source.repository !== params.get('url')) {
    if (!await openRepository(params.get('url'), params.get('ref') || '', false)) return;
  }
  state.checkId = params.get('check') || state.checkId;
  goto(STEP_GUIDES[step] ? step : step === 'trial-confirm' ? 'trial' : 'understand', false, false);
}
window.addEventListener('popstate', () => guarded(restoreLocation));
// 横並びで手順の名前が入りきらない幅・文字の大きさでは、縦並びにして名前を隠さない。
function fitSteps() {
  const nav = $('steps');
  if (nav.hidden) return;
  nav.dataset.orientation = 'horizontal';
  if (nav.scrollWidth > nav.clientWidth + 1) nav.dataset.orientation = 'vertical';
}
let fitting = 0;
new ResizeObserver(() => { cancelAnimationFrame(fitting); fitting = requestAnimationFrame(fitSteps); }).observe(document.documentElement);

// ---------- 1. 開く・理解する ----------
$('open-form').addEventListener('submit', async e => {
  e.preventDefault();
  const url = $('repo-url'), error = $('repo-url-error');
  if (!/^https:\/\/github\.com\/[^/\s]+\/[^/\s]+\/?$/.test(url.value.trim())) {
    url.setAttribute('aria-invalid', 'true'); error.hidden = false; error.textContent = '＊https://github.com/所有者/リポジトリ の形で入力してください。'; url.focus(); return;
  }
  url.removeAttribute('aria-invalid'); error.hidden = true;
  await openRepository(url.value.trim(), $('repo-ref').value.trim());
});
async function openRepository(url, reference, record = true) {
  return guarded(async () => {
    setStatus('GitHubから読み込んでいます。対象のコードは実行しません。');
    const res = await api('POST', '/api/v1/understand', { operation: 'open', url, reference });
    if (state.source?.repository !== res.source.repository || state.source?.commit !== res.source.commit) { if (state.source?.repository !== res.source.repository) state.pendingAI = null; state.aiSequence++; }
    if (state.source?.repository !== res.source.repository) { state.checkId = null; state.lastRun = null; state.trialForm = null; state.retest = null; state.expectDraft = ''; }
    state.source = res.source; state.overview = res.result; state.overviewRefs = res.references;
    state.files.clear(); state.fn = null; state.plan = null;
    state.code = null; state.selectedLine = null;
    $('code').hidden = true; $('code-view').replaceChildren(); $('code-meta').replaceChildren();
    $('steps').hidden = false; fitSteps();
    if (!$('person').hasChildNodes()) $('person').replaceChildren(personFields());
    renderCurrent(); renderUnderstand();
    // Show the new source before waiting for case data; later replies must keep the user's selected step.
    goto('understand', true, record);
    loadPlan(); await loadCase();
    setStatus(`コミット ${shortCommit(res.source.commit)} に固定して読み込みました。`);
    return res;
  });
}
async function loadPlan() {
  try { const res = await api('POST', '/api/v1/understand', { operation: 'plan', repository: state.source.repository, commit: state.source.commit }); state.plan = res.result; if (state.step.startsWith('trial')) renderTrial(); }
  catch (e) { state.plan = { error: e.message }; }
}
function readScope() {
  const s = state.source, o = state.overview;
  const failures = (s.omitted || []).filter(x => !['初回の読取範囲外', 'サイズ上限'].includes(x.reason));
  const omitted = new Map((s.omitted || []).map(x => [x.path, x.reason]));
  const unread = s.inventory.filter(f => !f.read && !failures.some(x => x.path === f.path)).map(f => ({ path: f.path, reason: omitted.get(f.path) || UNANALYZED }));
  return h('div', { class: 'box' },
    h('h3', { class: 'dads-u-std-20B-150' }, '読んだ範囲'),
    h('p', {}, `${s.repository} のコミット ${s.commit} を読みました（指定: ${s.reference}）。以降の説明と試験は、このコミットを対象にします。`),
    h('p', {}, `ファイル一覧 ${s.inventory.length} 件のうち、${s.files_read.length} 件の本文を読みました。読み取っていないファイルは ${unread.length} 件です。`),
    s.truncated ? banner('warning', 'ファイル一覧が途中で切れています', h('p', {}, 'GitHubの一覧に入りきらないファイルがあります。説明に含まれない処理がある可能性があります。')) : null,
    failures.length ? banner('warning', `${failures.length} 件のファイルを取得できませんでした`, h('ul', { class: 'items' }, failures.map(f => h('li', {}, `${f.path}: ${f.reason}`)))) : null,
    state.overviewRefs ? h('p', { class: 'note' }, `説明に付けたコードの参照 ${state.overviewRefs.total} 件のうち ${state.overviewRefs.verified} 件は、固定コミットのファイルと行に存在することを麦漆が確かめました。参照が存在しても、説明が正しいことは保証しません。`) : null,
    disclosure(`読み取っていないファイル（${unread.length} 件）`, unread.length ? [h('ul', { class: 'items' }, unread.slice(0, 500).map(f => h('li', {}, f.path, `（${f.reason}）`, ' ', button('読む', () => openCode(f.path, 1, 1), 'text')))), unread.length > 500 ? h('p', { class: 'note' }, `先頭の500件を表示しています。`) : null] : h('p', {}, 'ありません。')),
    disclosure(`ファイル一覧（${s.inventory.length} 件）`, h('ul', { class: 'items' }, s.inventory.slice(0, 500).map(f => h('li', {}, h('button', { class: 'linklike', type: 'button', onclick: () => openCode(f.path, 1, 1) }, f.path), f.read ? '（読んだ）' : '（未読）')))),
    o ? h('p', { class: 'note' }, '根拠の表示: ', basis('code'), KIND_TITLE.code, '。', basis('author'), KIND_TITLE.author, '。', basis('inference'), KIND_TITLE.inference, '。') : null);
}
function renderUnderstand() {
  const o = state.overview;
  const body = $('understand-body');
  if (!o) { body.replaceChildren(); return; }
  const flows = o.flows.map(f => h('div', { class: 'box' },
    h('h4', {}, `入口: ${f.entry.path} ${f.entry.line}行目`),
    h('p', {}, f.entry.reason, ' ', refLinks([{ path: f.entry.path, line: f.entry.line, verified: true }])),
    h('p', {}, basis(f.kind), f.basis),
    flowList('受け取る入力', f.inputs), flowList('保存・削除', f.storage), flowList('外部への送信', f.sending), flowList('表示', f.outputs), flowList('失敗の扱い（例外・終了）', f.errors),
    disclosure(`たどった関数（${f.steps.length} 件）`, h('ol', { class: 'items' }, f.steps.map(s => h('li', { style: `margin-left:${s.depth * 1.25}rem` }, `${s.function}（${s.path} ${s.line}〜${s.end_line}行目）`, ' ', refLinks(s.refs), s.called_from ? ` ← ${s.called_from.path} ${s.called_from.line}行目から呼ばれる候補` : '')))),
    f.unresolved_calls ? h('p', { class: 'note' }, `呼び出し先を読み取っていないため、たどれなかった呼び出しが ${f.unresolved_calls} 件あります。`) : null));
  const behaviorsByFile = path => o.behaviors.filter(b => b.path === path);
  fill(body,
    disclosure('読んだ範囲とコードの版', readScope()),
    h('h3', { class: 'dads-u-std-20B-150' }, '目的'), items(o.purpose.map(p => ({ ...p, text: p.origin ? `${p.text}（出所: ${p.origin}）` : p.text })), 'READMEやリポジトリの説明が見つかりませんでした。'),
    disclosure('起動のしかたと入口', items(o.start, '起動方法の記述は見つかりませんでした。')),
    h('h3', { class: 'dads-u-std-20B-150' }, '使用言語と実行への対応状況'), h('ul', { class: 'items' }, o.languages.map(l => h('li', {}, `${l.name}（${l.files} ファイル）: ${l.execution}`))),
    h('h3', { class: 'dads-u-std-20B-150' }, '主な構成と役割'),
    o.components.map(c => h('div', { class: 'box' },
      h('h4', {}, c.path, `（${c.role}、${c.lines} 行）`),
      c.description ? items([c.description]) : null,
      items(behaviorsByFile(c.path), '入力・保存・送信・エラー処理に当たる記述は見つかりませんでした。'),
      disclosure(`関数・メソッド（${c.functions.length} 件）`, h('ul', { class: 'items' }, c.functions.map(fn => h('li', {}, h('button', { class: 'linklike', type: 'button', onclick: () => inspectFunction(c.path, fn.line) }, fn.name), `（${fn.line}行目）`)))))),
    disclosure('入力から保存・送信まで', flows.length ? flows : h('p', { class: 'note' }, '入口を特定できなかったため、処理の関係を作れませんでした。')),
    disclosure('既存のテスト', o.tests.length ? h('ul', { class: 'items' }, o.tests.map(t => h('li', {}, `${t.path}（${t.kind}）`, t.read ? refLinks(t.refs) : '（未読）'))) : h('p', { class: 'note' }, 'テストのファイルは見つかりませんでした。')),
    h('h3', { class: 'dads-u-std-20B-150' }, 'コードを読むだけでは確かめていないこと'), items(o.unknown),
    h('p', { class: 'note' }, o.limits),
    h('div', { class: 'button-row' }, button('検証する動作を決める', () => goto('expect'), 'solid-fill', 'md')));
}
function flowList(title, list) {
  if (!list.length) return h('p', {}, h('strong', {}, `${title}: `), '見つかりませんでした。');
  return h('div', {}, h('strong', {}, title), h('ul', { class: 'items' }, list.slice(0, 12).map(e => h('li', {}, basis(e.certainty === 'name' ? 'inference' : 'code'), `${e.function}: ${e.label}`, ' ', codeText(e.code), ' ', refLinks(e.refs)))));
}

// ---------- 質問・関数 ----------
$('ask-form').addEventListener('submit', async e => {
  e.preventDefault();
  const question = $('question').value.trim();
  if (!question) { $('question').setAttribute('aria-invalid', 'true'); $('question').focus(); setStatus('質問を入力してください。'); return; }
  $('question').removeAttribute('aria-invalid');
  await guarded(async () => {
    setStatus('コードを調べています。');
    const res = await api('POST', '/api/v1/understand', { operation: 'inquire', repository: state.source.repository, commit: state.source.commit, question, path: null, line: null });
    state.source = res.source;
    renderAnswer(res.result, res.references);
    setStatus('調べた結果を表示しました。');
  });
});
async function inspectFunction(path, line) {
  return guarded(async () => {
    setStatus('関数を調べています。');
    const res = await api('POST', '/api/v1/understand', { operation: 'function', repository: state.source.repository, commit: state.source.commit, path, line });
    state.source = res.source;
    if (res.result.found) state.fn = { name: res.result.name, path: res.result.path, line: res.result.line };
    renderCurrent();
    renderAnswer({ question: `${path} ${line}行目の関数`, intents: [], findings: [], functions: [res.result], missing: res.result.found ? [] : [{ text: res.result.reason, kind: 'code', refs: [] }], limits: [], loaded_files: res.result.loaded_files || [] }, res.references);
    if (state.step !== 'understand') goto('understand', false);
    $('answer').scrollIntoView({ block: 'start' });
    setStatus(res.result.found ? `${res.result.name} を調べました。` : '関数を特定できませんでした。');
  });
}
function detailSection(title, list, render) { return h('div', {}, h('h5', {}, title), list?.length ? h('ul', { class: 'items' }, list.map(render)) : h('p', { class: 'note' }, 'ありません（読み取った範囲で見つかりませんでした）。')); }
function functionBlock(d) {
  if (!d.found) return h('div', { class: 'box' }, h('p', {}, d.reason));
  return h('div', { class: 'box' },
    h('h4', {}, `${d.name}（${d.path} ${d.line}〜${d.end_line}行目）`, ' ', refLinks(d.refs)),
    h('p', {}, basis('code'), '宣言: ', codeText(d.signature)),
    detailSection('受け取る値（引数）', d.parameters, p => h('li', {}, codeText(p))),
    detailSection('返す値（return の記述）', d.returns, r => h('li', {}, codeText(r.code), ' ', refLinks(r.refs))),
    detailSection('分岐と繰り返し', d.branches, b => h('li', {}, codeText(b.code), ' ', refLinks(b.refs))),
    detailSection('発生させる例外と、捕捉する記述', d.raised, r => h('li', {}, `${r.exception} を発生させる: `, codeText(r.code), ' ', refLinks(r.refs), h('div', {}, basis(r.handled.kind), r.handled.text, ' ', refLinks(r.handlers.flatMap(x => x.refs))))),
    detailSection('この関数の中で捕捉する例外', d.caught, c => h('li', {}, codeText(c.code), ' ', refLinks(c.refs))),
    detailSection('副作用（入力・保存・送信・表示・終了）', d.effects, e => h('li', {}, basis(e.certainty === 'name' ? 'inference' : 'code'), `${e.label}: `, codeText(e.code), ' ', refLinks(e.refs))),
    detailSection('呼び出す関数', d.callees, c => h('li', {}, codeText(c.name), ` ${c.line}行目。`, c.candidates.length ? ['対応先の候補: ', c.candidates.map(x => h('button', { class: 'linklike', type: 'button', onclick: () => inspectFunction(x.path, x.line) }, `${x.name}（${x.path} ${x.line}行目）`))] : c.reason, ' ', refLinks(c.refs))),
    detailSection('呼び出し元の候補', [...d.callers.map(c => ({ ...c, why: c.reason })), ...d.name_callers.map(c => ({ ...c, why: c.reason }))], c => h('li', {}, basis('inference'), `${c.function || c.path} ${c.line}行目: ${c.why || ''}`, ' ', refLinks(c.refs))),
    detailSection('関係する設定（コマンドのオプション・環境変数）', [...d.config.options.map(o => ({ text: `${o.flags.join(' / ')}${o.help ? `: ${o.help}` : ''}${o.default != null ? `（既定値 ${o.default}）` : ''}`, refs: o.refs })), ...d.config.env.map(e => ({ text: `環境変数を読む: ${e.code}`, refs: e.refs }))], o => h('li', {}, o.text, ' ', refLinks(o.refs))),
    detailSection('既存のテストで、この関数やオプションに触れている行', d.tests, t => h('li', {}, codeText(t.code), ' ', refLinks(t.refs))),
    h('p', { class: 'note' }, d.limits),
    h('div', { class: 'button-row' }, button('この関数を開く', () => openCode(d.path, d.line, d.end_line)), button('この関数を確認項目に関連づける', () => { state.pendingRef = { repository: state.source.repository, commit: state.source.commit, path: d.path, line: d.line, end_line: d.end_line }; goto('expect'); })));
}
function renderAnswer(r, references) {
  $('answer').replaceChildren(h('div', { class: 'box' },
    h('h4', {}, `質問: ${r.question}`),
    r.intents?.length ? h('p', {}, `調べた観点: ${r.intents.map(i => i.label).join('、')}`) : null,
    r.loaded_files?.length ? h('p', {}, `質問に関係するため、追加で読んだファイル: ${r.loaded_files.join('、')}`) : null,
    r.failed_files?.length ? banner('warning', '読めなかったファイルがあります', h('ul', { class: 'items' }, r.failed_files.map(f => h('li', {}, `${f.path}: ${f.reason}`)))) : null,
    h('h5', {}, '見つかった記述'), items(r.findings, '該当する記述は見つかりませんでした。'),
    r.functions?.map(functionBlock),
    r.missing?.length ? [h('h5', {}, '見つからなかったこと'), items(r.missing)] : null,
    r.needed_files?.length ? h('p', { class: 'note' }, `さらに関係する可能性がある未読のファイル: ${r.needed_files.join('、')}`) : null,
    r.limits?.length ? [h('h5', {}, 'この結果の限界'), items(r.limits)] : null,
    references ? h('p', { class: 'note' }, `示した参照 ${references.total} 件のうち ${references.verified} 件が、固定コミットのファイルと行に存在することを確かめました。`) : null));
}

// ---------- AI ----------
async function loadAi() {
  try { state.ai = await api('GET', '/api/v1/ai'); } catch (e) { state.ai = { status: 'failed', message: e.message }; }
  $('ai-status').textContent = state.ai.message;
  $('ask-ai').hidden = state.ai.status !== 'configured';
}
$('ask-ai').addEventListener('click', async () => {
  const question = $('question').value.trim();
  if (!question) { setStatus('質問を入力してください。'); $('question').focus(); return; }
  await guarded(async () => {
    const source = { repository: state.source.repository, commit: state.source.commit }, sequence = ++state.aiSequence;
    setStatus('AIに問い合わせています。根拠の行は麦漆が照合します。');
    const res = await api('POST', '/api/v1/ai', { ...source, question, path: null, line: null });
    if (sequence !== state.aiSequence || source.repository !== state.source?.repository || source.commit !== state.source?.commit) return;
    const node = res.status === 'ok'
      ? h('div', { class: 'box' }, h('h4', {}, basis('ai'), 'AIによる説明', res.model ? `（${res.model}）` : ''), h('p', {}, res.answer || ''),
          h('p', { class: 'note' }, `質問: ${question}`),
          h('ul', { class: 'items' }, (res.claims || []).map(c => h('li', {}, c.text, ' ', refLinks(c.refs || []), c.has_verified_reference ? null : h('span', { class: 'ref-unverified' }, '（固定コミットで確認できる根拠の行がない）')))),
          res.unknowns?.length ? [h('h5', {}, 'AIが分からないとしたこと'), h('ul', { class: 'items' }, res.unknowns.map(u => h('li', {}, u)))] : null,
          h('p', { class: 'note' }, res.meaning), res.unverified_references ? h('p', { class: 'ref-unverified' }, `固定コミットに存在しない参照が ${res.unverified_references} 件ありました。`) : null,
          res.references_outside_context ? h('p', { class: 'ref-unverified' }, `今回AIへ渡した抜粋と照合できない参照が ${res.references_outside_context} 件ありました。`) : null,
          aiCandidates(res, source))
      : banner('warning', 'AIによる説明を表示できません', h('p', {}, res.message), h('p', {}, '静的解析による結果は「調べる」で表示できます。'));
    $('answer').prepend(node);
    setStatus(res.status === 'ok' ? 'AIによる説明を表示しました。' : 'AIによる説明は取得できませんでした。');
  });
});

function aiCandidates(response, source) {
  const candidates = response.test_candidates || [];
  if (!candidates.length) return null;
  return h('section', { 'aria-label': 'AIからの確認候補' }, h('h5', {}, '次に確かめる候補'),
    h('p', { class: 'note' }, '候補はまだ保存・実行していません。根拠の行と未確認事項を読み、必要な候補を自分の条件に直してから保存します。'),
    candidates.map(candidate => h('div', { class: 'statement', 'data-ai-candidate': '' },
      h('p', {}, h('strong', {}, candidate.expectation)), h('p', {}, candidate.reason), refLinks(candidate.refs),
      candidate.unknowns.length ? [h('h6', {}, 'この候補でまだ分からないこと'), h('ul', { class: 'items' }, candidate.unknowns.map(u => h('li', {}, u)))] : null,
      candidate.can_use ? button('この候補を編集する', () => guarded(async () => {
        if (source.repository !== state.source?.repository || source.commit !== state.source?.commit) throw Error('対象の版が変わりました。現在のコードで問い合わせ直してください。');
        if (state.expectDraft?.trim()) throw Error('書きかけの確認項目があります。「検証する動作を決める」で保存するか、入力を消してから候補を選んでください。');
        state.pendingAI = { source, model: response.model, provenance: candidate.provenance, candidate, unknowns: response.unknowns };
        state.pendingRef = null;
        state.expectDraft = candidate.expectation; goto('expect'); $('expect-text').focus();
      })) : h('p', { class: 'ref-unverified' }, candidate.unavailable_reason || '今回AIへ渡した抜粋に根拠をそろえられないため、この候補は取り込めません。'))));
}

// ---------- コード表示 ----------
async function fileText(path) {
  if (state.files.has(path)) return state.files.get(path);
  const res = await api('POST', '/api/v1/understand', { operation: 'file', repository: state.source.repository, commit: state.source.commit, path });
  state.source = res.source;
  state.files.set(path, res.file);
  return res.file;
}
async function openCode(path, line = 1, end = line, commit = state.source.commit) {
  return guarded(async () => {
    if (commit !== state.source.commit) throw Error(`このコードはコミット ${shortCommit(commit)} のものです。結果の画面の「試したコミットのコードを開く」から開いてください。`);
    const file = await fileText(path);
    state.code = { path, line, end };
    state.selectedLine = line;
    const lines = file.text.split('\n');
    if (lines.length > 1 && lines.at(-1) === '') lines.pop();
    $('code-meta').replaceChildren(`${path}（コミット ${shortCommit(state.source.commit)}、${lines.length} 行）`, ' ', h('a', { href: file.url.replace(/#L1$/, `#L${line}`), target: '_blank', rel: 'noopener noreferrer' }, 'GitHubで開く'));
    const view = $('code-view');
    view.replaceChildren(...lines.map((text, i) => h('div', { class: 'code-line', 'data-line': i + 1, 'data-mark': i + 1 >= line && i + 1 <= end, 'data-selected': i + 1 === line },
      h('span', { class: 'num' }, h('button', { type: 'button', 'aria-label': `${i + 1}行目を選ぶ`, onclick: () => selectLine(i + 1) }, String(i + 1))), h('span', { class: 'text' }, text))));
    $('code').hidden = false;
    renderCurrent();
    const target = view.querySelector(`[data-line="${line}"]`);
    if (target) view.scrollTop = target.offsetTop - view.clientHeight / 3;
    setStatus(`${path} の ${line}行目を表示しました。`);
  });
}
function selectLine(n) {
  state.selectedLine = n;
  for (const el of $('code-view').querySelectorAll('[data-selected]')) el.removeAttribute('data-selected');
  $('code-view').querySelector(`[data-line="${n}"]`)?.setAttribute('data-selected', '');
  setStatus(`${state.code.path} の ${n}行目を選びました。`);
}
$('code-close').addEventListener('click', () => { $('code').hidden = true; });
$('code-function').addEventListener('click', () => state.code && inspectFunction(state.code.path, state.selectedLine));
$('code-link').addEventListener('click', () => {
  if (!state.code) return;
  const line = state.selectedLine;
  state.pendingRef = { repository: state.source.repository, commit: state.source.commit, path: state.code.path, line, end_line: line >= state.code.line && line <= state.code.end ? state.code.end : line };
  goto('expect');
});

// ---------- 事例（期待・記録） ----------
async function loadCase() {
  try {
    const view = await api('GET', '/api/v1/case');
    const execution = await api('GET', '/api/v1/execution');
    if (state.view && view.workspace.revision < state.view.workspace.revision) return;
    state.view = view; state.execution = execution;
  } catch (e) { showError(e); }
  renderCurrent();
  if (['open', 'repository'].includes(state.step)) $('current').hidden = true;
  if (state.step === 'expect' || state.step === 'expect-edit') renderExpect();
  if (state.step.startsWith('trial')) renderTrial();
  if (state.step === 'result') renderResult();
}
function person() { try { return JSON.parse(sessionStorage.getItem('eel-person')) || { name: '', role: 'operator' }; } catch { return { name: '', role: 'operator' }; } }
function personFields() {
  const p = person();
  const name = input({ value: p.name, autocomplete: 'name' });
  const role = select(Object.entries(ROLE), {});
  role.control.value = p.role;
  const save = () => sessionStorage.setItem('eel-person', JSON.stringify({ name: name.control.value.trim(), role: role.control.value }));
  name.control.addEventListener('input', save); role.control.addEventListener('change', save);
  return h('fieldset', { class: 'identity-strip' }, h('legend', {}, '記録者'),
    h('div', { class: 'inline-fields' }, field({ label: '名前', control: name, required: true }), field({ label: '役割', control: role })), h('p', { class: 'note' }, '入力した名前と役割を記録に残します。本人確認は行いません。'));
}
function requirePerson() {
  const p = person();
  if (!p.name) { const e = Error('記録する前に「記録者」の名前を入力してください。'); throw e; }
  return p;
}
function repoChecks() {
  if (!state.view || !state.source) return [];
  const prefix = repoSlug(state.source.repository) + '-';
  const concernOrigins = new Set(state.view.workspace.statements.flatMap(statement => {
    try {
      const origin = JSON.parse(statement.origin);
      return origin?.kind === 'repository_concern' && origin.repository === state.source.repository
        ? [`repository-concern:${statement.id}`] : [];
    } catch { return []; }
  }));
  return state.view.workspace.checks.filter(check => check.id.startsWith(prefix)
    || check.source?.repository === state.source.repository || concernOrigins.has(check.origin));
}
function currentCheck() { return state.view?.workspace.checks.find(c => c.id === state.checkId) || null; }
function checkView(id) { return state.view?.checks.find(c => c.check_id === id); }
async function change(action, reason) {
  const p = requirePerson();
  const view = await api('POST', '/api/v1/change', { expected_revision: state.view.workspace.revision, actor: `${p.name}（${ROLE[p.role]}）`, reason, action });
  state.view = view;
  renderCurrent();
  return view;
}
function statementBlock(s) {
  const v = state.view.statements.find(x => x.id === s.id);
  const versions = Object.entries(s.check_versions || {}).map(([k, n]) => `${k} の版${n}`).join('、');
  const resolveText = textarea({ rows: 2 });
  return h('div', { class: 'statement', 'data-kind': s.kind },
    h('p', {}, h('strong', {}, STATEMENT_KIND[s.kind] || s.kind), `: ${s.text}`),
    s.source ? h('p', {}, '対応づけたコード: ', refLinksFor(s.source)) : null,
    h('p', { class: 'statement-meta' }, `記入: ${s.recorded_by || '不明'}・出所: ${s.origin}・事例の版 ${s.recorded_revision || '不明'}${versions ? `・記入時の条件: ${versions}` : ''}`),
    v ? h('p', { class: 'statement-meta' }, `確認の状態: ${v.status_label}`) : null,
    s.confirmations?.length ? h('ul', { class: 'items statement-meta' }, s.confirmations.map(c => h('li', {}, `${c.actor}が事例の版 ${c.revision} で確認した${c.note ? `: ${c.note}` : ''}`))) : null,
    s.resolution ? h('p', { class: 'statement-meta' }, `対応: ${s.resolution.text}（${s.resolution.actor}、事例の版 ${s.resolution.revision}）`) : null,
    h('div', { class: 'button-row' },
      button('内容を確認したことを記録する', () => guarded(async () => { await change({ op: 'confirm_statement', statement_id: s.id, role: person().role, note: '' }, '記録の内容を読んで確認した'); renderExpect(); setStatus('確認を記録しました。'); })),
      v?.open ? disclosure('対応を記録する', field({ label: '対応の内容', control: resolveText }), button('対応を記録', () => guarded(async () => { if (!resolveText.control.value.trim()) throw Error('対応の内容を入力してください。'); await change({ op: 'resolve_statement', statement_id: s.id, text: resolveText.control.value.trim() }, '指摘・質問への対応を記録した'); renderExpect(); renderResult(); }))) : null));
}
function refLinksFor(source) {
  const same = state.source && source.commit === state.source.commit && source.repository === state.source.repository;
  return h('span', {}, `${source.path} ${source.line}${source.end_line !== source.line ? `〜${source.end_line}` : ''}行目（コミット ${shortCommit(source.commit)}）`, ' ',
    same ? button('開く', () => openCode(source.path, source.line, source.end_line), 'text') : h('span', { class: 'note' }, '（現在読んでいるコミットとは異なります）'),
    ' ', h('a', { href: `https://github.com/${source.repository}/blob/${source.commit}/${source.path.split('/').map(encodeURIComponent).join('/')}#L${source.line}`, target: '_blank', rel: 'noopener noreferrer' }, 'GitHubで開く'));
}
function scopeHelp() {
  return disclosure('「受付が成功」「書き込みが成功」「後から読み出せた」の違い', h('ul', { class: 'items' },
    h('li', {}, h('strong', {}, '受付が成功: '), '対象が操作を受け取り、正常に終わったことです。終了コードや画面の表示など、対象自身の報告で分かります。保存できたことまでは示しません。'),
    h('li', {}, h('strong', {}, '書き込みが成功: '), '保存先に内容が書かれたことです。麦漆は、試行の後に保存先を直接読み出して確かめます。'),
    h('li', {}, h('strong', {}, '後から読み出せた: '), '書き込んだプロセスが終わった後、別のプロセスで読み出して内容が得られたことです。')));
}
function proposals() {
  const ex = state.plan?.examples || [];
  if (!ex.length) return null;
  const textareaEl = () => document.getElementById('expect-text');
  return disclosure(`既存のテストからの例（${ex.length} 件、未保存）`,
    h('p', { class: 'note' }, 'テストに書かれた操作と結果の例です。自動で確認項目には登録されません。参考にして、確認したい動作を自分の言葉で記入してください。'),
    h('ul', { class: 'items' }, ex.slice(0, 12).map(e => h('li', {}, codeText(e.command), e.expected.length ? [' の後に ', codeText(e.expected.join(' / ')), ' が表示される'] : ' の後に何も表示されない', ' ', refLinks(e.refs),
      e.alias ? h('div', { class: 'note' }, `（${e.command.split(' ')[0]} は ${e.alias} の別名）`) : null,
      ' ', button('この例を参考に書く', () => { const t = textareaEl(); if (t) { t.value = `（テスト ${e.path} の例を参考に）利用者が「${e.command}」を実行したとき、`; t.focus(); } }, 'text')))));
}
function renderExpect() {
  const body = $('expect-body');
  if (!state.view) { body.replaceChildren(h('p', {}, '記録を読み込んでいます。')); return; }
  if (state.step === 'expect-edit' && currentCheck()) {
    fill(body, expectationCard(currentCheck()), h('div', { class: 'journey-actions' }, button('確認項目の一覧に戻る', () => goto('expect'))));
    return;
  }
  const text = textarea({ rows: 3, value: state.expectDraft || '' });
  text.control.addEventListener('input', () => { state.expectDraft = text.control.value; });
  text.control.id = 'expect-text';
  const form = h('form', { novalidate: true, onsubmit: e => { e.preventDefault(); addExpectation(text.control); } },
    field({ label: '誰が何をしたとき、何がどうなってほしいか', id: 'expect-text', control: text, required: true, support: '例: 利用者がタスクを追加したとき、後から一覧で同じタスクを読み出せる。例: 保存できなかったとき、利用者に失敗が分かる。' }),
    h('div', { class: 'button-row' }, h('button', { class: 'dads-button', 'data-type': 'solid-fill', 'data-size': 'md', type: 'submit' }, 'この動作を確かめる')));
  const pending = state.pendingRef ? banner('info', '対応づけるコードを選んでいます', h('p', {}, `${state.pendingRef.path} ${state.pendingRef.line}〜${state.pendingRef.end_line}行目（コミット ${shortCommit(state.pendingRef.commit)}）。下の確認項目の「実装と確認方法を対応づける」で使います。`), button('選択をやめる', () => { state.pendingRef = null; renderExpect(); }, 'text')) : null;
  const checks = repoChecks();
  fill(body,
    pending,
    state.pendingAI ? banner('info', 'AIの確認候補を編集しています（未保存）',
      h('p', {}, `${state.pendingAI.model}による提案です。保存しても試験済みや合意済みにはなりません。`),
      h('p', {}, `対象: ${state.pendingAI.source.repository}（${shortCommit(state.pendingAI.source.commit)}）`),
      h('p', {}, state.pendingAI.candidate.reason), h('span', {}, state.pendingAI.candidate.refs.map(r => refLinksFor({ ...state.pendingAI.source, path: r.path, line: r.line, end_line: r.end_line }))),
      h('ul', { class: 'items' }, [...state.pendingAI.unknowns, ...state.pendingAI.candidate.unknowns].map(u => h('li', {}, u))),
      button('この候補を使わず書き直す', () => { state.pendingAI = null; state.expectDraft = ''; renderExpect(); }, 'text')) : null,
    h('h3', { class: 'dads-u-std-20B-150' }, '確認したい動作を書く'), form, proposals(), scopeHelp(),
    h('h3', { class: 'dads-u-std-20B-150' }, `登録した確認項目（${checks.length} 件）`),
    checks.length ? checks.map(expectationCard) : h('p', { class: 'note' }, '確認項目はまだ登録されていません。'));
}
async function addExpectation(control) {
  const value = control.value.trim();
  if (!value) { control.setAttribute('aria-invalid', 'true'); control.focus(); setStatus('確認したい動作を入力してください。'); return; }
  await guarded(async () => {
    const p = requirePerson();
    const prefix = repoSlug(state.source.repository);
    let n = repoChecks().length + 1;
    while (state.view.workspace.checks.some(c => c.id === `${prefix}-${n}`)) n++;
    const id = `${prefix}-${n}`;
    const draft = state.pendingAI;
    if (draft && (draft.source.repository !== state.source.repository || draft.source.commit !== state.source.commit)) throw Error('AI候補の対象の版が変わりました。現在のコードから候補を選び直してください。');
    const origin = draft ? `AIの未検証の確認候補から${p.name}が確認項目を保存。出所: ${draft.provenance}` : `${p.name}（${ROLE[p.role]}）が画面で入力した確認項目`;
    const ref = draft?.candidate.refs[0];
    const source = ref ? { ...draft.source, path: ref.path, line: ref.line, end_line: ref.end_line } : null;
    await change({ op: 'add_check', check: { id, version: 1, text: value, origin, source, observation: null, rule: 'manual_only', dependencies: [], questions: [] } }, '期待する動作を記録した');
    await change({ op: 'add_statement', statement: { id: randomId('s'), kind: draft ? 'proposal' : 'statement', role: p.role, text: value, origin: draft ? origin : `${p.name}が自分の言葉で書いた確認項目`, source, check_ids: [id] } }, draft ? 'AI候補から編集した提案として残した' : '確認項目を書いた人の発言として残した');
    state.checkId = id; state.expectDraft = ''; state.pendingAI = null;
    renderCurrent(); goto('expect-edit');
    setStatus('確認項目を登録しました。次に、該当するコードと検証方法を指定してください。');
  });
}
function expectationCard(check) {
  const cv = checkView(check.id);
  const statements = state.view.workspace.statements.filter(s => s.check_ids.includes(check.id) && !s.evidence_id);
  const explain = textarea({ rows: 3 }), method = textarea({ rows: 2 }), refUrl = input({ placeholder: 'https://github.com/所有者/リポジトリ/blob/コミット/パス#L10' });
  const pending = state.pendingRef;
  const disc = textarea({ rows: 2 }), question = textarea({ rows: 2 }), revise = textarea({ rows: 2, value: check.text });
  const unanswered = check.questions.filter(q => !q.answer);
  const aiOrigin = readAiOrigin(check.origin);
  return h('div', { class: 'box', 'data-check': check.id },
    h('h4', {}, check.text),
    disclosure('記録の出所と現在の状態', h('p', { class: 'statement-meta' }, `識別子 ${check.id}・条件の版 ${check.version}・出所: ${aiOrigin ? 'AIの未検証の確認候補から記入者が保存' : check.origin}`), cv ? verdictBox(cv.verdict, cv.reason) : null),
    aiOrigin ? disclosure('AI候補の出所と未確認事項',
      h('p', {}, `モデル: ${aiOrigin.model}。質問: ${aiOrigin.question}`),
      h('p', {}, `元の候補: ${aiOrigin.candidate.expectation}`), h('p', {}, aiOrigin.candidate.reason),
      h('ul', { class: 'items' }, [...aiOrigin.unknowns, ...aiOrigin.candidate.unknowns].map(u => h('li', {}, u))),
      h('p', { class: 'note' }, '保存時の提案です。試験結果や人の確認を表すものではありません。'),
      disclosure('内容を識別する値', h('p', {}, `コード抜粋 SHA-256: ${aiOrigin.context_sha256}`), h('p', {}, `AI応答 SHA-256: ${aiOrigin.response_sha256}`))) : null,
    h('p', { class: 'note' }, cv?.evidence.length ? '試行の結果は「結果を見る」で確認できます。' : 'まだ試していません。コードと確認方法を対応づけたら試験へ進めます。'),
    h('p', {}, h('strong', {}, '対応づけたコード: '), check.source ? refLinksFor(check.source) : 'まだありません。'),
    unanswered.length ? h('div', {}, h('strong', {}, '未回答の質問'), h('ul', { class: 'items' }, unanswered.map(q => answerForm(check, q)))) : null,
    disclosure('確認項目と説明の履歴', statements.map(statementBlock)),
    h('details', { class: 'dads-disclosure', open: Boolean(pending) }, h('summary', { class: 'dads-disclosure__summary' }, '実装と確認方法を対応づける（技術者）'), h('div', { class: 'dads-disclosure__content' },
      pending ? h('p', {}, `使うコード: ${pending.path} ${pending.line}〜${pending.end_line}行目（コミット ${shortCommit(pending.commit)}）`) : field({ label: 'コードの行', control: refUrl, support: 'コードの画面で行を選んで「この行を確認項目に関連づける」を押すと入ります。固定コミットのGitHubの行のURLも使えます。' }),
      field({ label: 'この動作を実装しているコードと処理内容', control: explain, required: true }),
      field({ label: 'どう確かめるか（確認の方法）', control: method, required: true, support: '例: 試験用の値でタスクを追加し、麦漆が保存先 tasks を読み出して値があることと、別のプロセスで一覧に出ることを確かめる。' }),
      button('対応づけを記録', () => guarded(async () => {
        const p = requirePerson();
        const source = pending || refFromUrl(refUrl.control.value.trim());
        if (!explain.control.value.trim() || !method.control.value.trim()) throw Error('説明と確認の方法を入力してください。');
        if (!check.source || JSON.stringify(check.source) !== JSON.stringify(source)) await change({ op: 'bind_source', check_id: check.id, source }, '確認項目に対応するコードの行を登録した');
        await change({ op: 'add_statement', statement: { id: randomId('s'), kind: 'implementation', role: p.role, text: `${explain.control.value.trim()}\n確認の方法: ${method.control.value.trim()}`, origin: `${p.name}による実装の説明`, check_ids: [check.id], source } }, '実装と確認方法の説明を記録した');
        state.pendingRef = null; state.checkId = check.id;
        if (state.retest?.checkId === check.id) state.retest = null;
        renderExpect(); setStatus('対応づけを記録しました。');
      }), 'solid-fill', 'md'))),
    disclosure('食い違いを指摘する', field({ label: '確認項目・説明・コードが一致しない箇所', control: disc }), button('指摘を記録', () => guarded(async () => {
      const p = requirePerson(); if (!disc.control.value.trim()) throw Error('指摘の内容を入力してください。');
      await change({ op: 'add_statement', statement: { id: randomId('s'), kind: 'discrepancy', role: p.role, text: disc.control.value.trim(), origin: `${p.name}の指摘`, check_ids: [check.id] } }, '食い違いを指摘した'); renderExpect();
    }))),
    disclosure('答えが必要な質問を残す', field({ label: '質問', control: question, support: '回答されるまで、この項目は「判定できない」と表示されます。' }), button('質問を記録', () => guarded(async () => {
      if (!question.control.value.trim()) throw Error('質問を入力してください。');
      await change({ op: 'add_question', check_id: check.id, question: { id: randomId('q'), text: question.control.value.trim(), answer: null, origin: null } }, '確認項目について質問を残した'); renderExpect();
    }))),
    disclosure('確認項目を修正する', field({ label: '修正後の内容', control: revise, support: '文を直すと条件の版が上がり、前の版の試行結果は再確認待ちになります。前の記録は消えません。' }), button('変更を保存する', () => guarded(async () => {
      const p = requirePerson(); const t = revise.control.value.trim(); if (!t || t === check.text) throw Error('直した文を入力してください。');
      await change({ op: 'revise_check', check_id: check.id, text: t, origin: `${p.name}（${ROLE[p.role]}）が画面で確認項目を修正`, rule: check.rule, dependencies: check.dependencies }, '確認項目を修正した');
      await change({ op: 'add_statement', statement: { id: randomId('s'), kind: 'statement', role: p.role, text: t, origin: `${p.name}が修正した確認項目`, check_ids: [check.id] } }, '修正した確認項目を発言として残した'); renderExpect();
    }))),
    h('div', { class: 'button-row' }, button('この動作を検証する', () => { state.checkId = check.id; renderCurrent(); goto('trial'); }, 'solid-fill', 'md'), button('結果を見る', () => { state.checkId = check.id; renderCurrent(); goto('result'); })));
}

function readAiOrigin(origin) {
  try {
    const marker = '。出所: ', index = origin.indexOf(marker);
    if (index < 0) return null;
    const record = JSON.parse(origin.slice(index + marker.length));
    const strings = values => Array.isArray(values) && values.every(v => typeof v === 'string');
    return record?.kind === 'ai_candidate' && typeof record.model === 'string' && typeof record.question === 'string'
      && typeof record.candidate?.expectation === 'string' && typeof record.candidate.reason === 'string'
      && strings(record.unknowns) && strings(record.candidate.unknowns) ? record : null;
  } catch { return null; }
}
function answerForm(check, q) {
  const a = textarea({ rows: 2 });
  return h('li', {}, q.text, disclosure('回答する', field({ label: '回答', control: a }), button('回答を記録', () => guarded(async () => {
    const p = requirePerson(); if (!a.control.value.trim()) throw Error('回答を入力してください。');
    await change({ op: 'answer', check_id: check.id, question_id: q.id, answer: a.control.value.trim(), origin: `${p.name}（${ROLE[p.role]}）の回答` }, '質問に回答した'); renderExpect();
  }))));
}
function refFromUrl(value) {
  const m = /^https:\/\/github\.com\/([^/]+\/[^/]+)\/blob\/([a-f0-9]{40})\/(.+)#L(\d+)(?:-L(\d+))?$/.exec(value);
  if (!m) throw Error('コードの画面で行を選ぶか、固定コミットのGitHubの行のURL（…/blob/40桁のコミット/パス#L行）を入力してください。');
  return { repository: m[1], commit: m[2], path: decodeURIComponent(m[3]), line: Number(m[4]), end_line: Number(m[5] || m[4]) };
}
function verdictBox(verdict, reason, extra = null) {
  const v = VERDICT[verdict] || VERDICT.unknown;
  return h('div', { class: 'verdict', 'data-verdict': verdict }, h('span', { class: 'verdict-label' }, h('span', { class: 'mark', 'aria-hidden': 'true' }, v.mark), ' ', v.label), h('div', {}, h('p', {}, reason), extra));
}

// ---------- 3. 試す ----------
function trialDefaults(kind) {
  const plan = state.plan;
  const entry = plan?.entries?.find(e => e.supported);
  const options = plan?.options || [];
  const dir = options.find(o => /dir/i.test([o.dest, o.metavar, ...o.flags].join(' ')));
  const list = options.find(o => /list|name|file/i.test([o.dest, o.metavar].join(' ')) && o.default && o.default !== "''" && o.default !== '""');
  const flag = o => o.flags.find(f => f.startsWith('--')) || o.flags[0];
  const base = [...(dir ? [flag(dir), '.'] : []), ...(list ? [flag(list), list.default] : [])];
  const storage = list ? list.default : '';
  const common = { entrypoint: entry?.entrypoint || '', launch: entry?.launch || 'file', runtime: entry?.runtime || 'python3', storage, timeout_ms: 10000, readback: null, python_root: '', dependencies_text: '', server: null, containers_text: '', http: null };
  if (kind === 'normal') return { ...common, fault: 'normal', args: [...base, '{{test_value}}'], readback: entry ? { launch: entry.launch || 'file', entrypoint: entry.entrypoint, args: base } : null,
    checks: [{ scope: '受付', method: 'exit_code', expect: 'zero', text: '' }, { scope: '書き込み', method: 'stored_file', expect: 'contains', text: '{{test_value}}' }, ...(entry ? [{ scope: '後からの読み出し', method: 'readback_output', expect: 'contains', text: '{{test_value}}' }] : [])] };
  return { ...common, fault: 'storage_read_only', args: [...base, '{{test_value}}'],
    checks: [{ scope: '終了コードで失敗が分かる', method: 'exit_code', expect: 'nonzero', text: '' }, { scope: 'エラー表示で失敗が分かる', method: 'stderr', expect: 'contains', text: 'Permission denied' }, { scope: '書き込み', method: 'stored_file', expect: 'not_contains', text: '{{test_value}}' }] };
}
function lastRecipe(checkId) {
  const ev = state.view?.workspace.evidence.filter(e => e.check_id === checkId && e.execution?.recipe?.trial).at(-1);
  return ev?.execution.recipe || null;
}
function recipeToForm(r) {
  return { entrypoint: r.entrypoint, launch: r.launch || 'file', runtime: r.runtime, args: r.args, storage: r.trial.storage || '', timeout_ms: r.timeout_ms, fault: r.trial.fault,
    readback: r.trial.readback ? { launch: 'file', ...r.trial.readback } : null, python_root: r.python?.root || '',
    dependencies_text: (r.python?.dependencies || []).map(d => [d.repository, d.commit, d.root].filter(Boolean).join(' ')).join('\n'),
    server: r.server ? { launch: 'file', ...r.server } : null,
    containers_text: r.containers ? JSON.stringify(r.containers.services, null, 2) : '',
    http: r.http ? { ...r.http } : null,
    checks: r.trial.checks.map(c => ({ scope: c.scope, method: c.method, expect: c.expect, text: c.text || '', target: c.target || '' })) };
}
/// フォームの内容から、Pythonの読み込み先の設定を作る。ファイルを起動するだけで依存先がない場合は作らない。
function pythonSetup(f) {
  const dependencies = parseDependencies(f.dependencies_text || '');
  const launches = [f.launch, f.readback?.launch, f.server?.launch].filter(Boolean);
  if (f.runtime !== 'python3' || (!f.python_root && !dependencies.length && launches.every(l => l === 'file'))) return null;
  return { root: f.python_root || null, dependencies };
}
function pythonSetupOrNull(f) {
  try { return pythonSetup(f); } catch { return null; }
}
function parseContainers(text) {
  if (!String(text || '').trim()) return null;
  let services;
  try { services = JSON.parse(text); } catch { throw Error('コンテナの指定はJSONの配列で書いてください。'); }
  if (!Array.isArray(services) || !services.length) throw Error('コンテナを1件以上書いてください。');
  return { services };
}
function launchCommand(f, launch, entrypoint, args) {
  if (f.runtime === 'http') return `HTTP ${f.http?.method || 'GET'} ${f.http?.service || ''} ${f.http?.path || entrypoint || ''}`;
  const runtime = f.runtime === 'node' ? 'node' : 'python3';
  const flags = pythonSetupOrNull(f) ? ['-S', '-P'] : [];
  const start = launch === 'module' ? ['-m', entrypoint] : launch === 'script' ? [`（コマンド ${entrypoint} の宣言どおりに関数を呼ぶ）`] : [entrypoint];
  return [runtime, ...flags, ...start, ...args].join(' ');
}
function parseDependencies(text) {
  return text.split('\n').map(s => s.trim()).filter(Boolean).map(line => {
    const [repository, commit, root, ...rest] = line.split(/\s+/);
    if (rest.length || !/^[\w.-]+\/[\w.-]+$/.test(repository || '') || !/^[0-9a-f]{40}$/.test(commit || '')) throw Error(`依存先は「所有者/リポジトリ 40桁のコミット 読み込み先（任意）」の形で1行に1つ書いてください: ${line}`);
    return root ? { repository, commit, root } : { repository, commit };
  });
}
function renderTrial() {
  const body = $('trial-body');
  const checks = repoChecks();
  if (!state.view) { body.replaceChildren(h('p', {}, '記録を読み込んでいます。')); return; }
  const pick = select([['', '選んでください'], ...checks.map(c => [c.id, c.text])]);
  pick.control.value = state.checkId || '';
  pick.control.addEventListener('change', () => { state.checkId = pick.control.value || null; state.trialForm = null; renderCurrent(); goto('trial', false); });
  const check = currentCheck();
  const plan = state.plan;
  const exec = state.execution;
  const notes = [];
  if (!plan) notes.push(h('p', {}, '試験の準備に使う情報を読み込んでいます。'));
  else if (plan.error) notes.push(banner('error', '試験の準備に使う情報を作れませんでした', h('p', {}, plan.error)));
  else if (!plan.executable) notes.push(banner('warning', 'この対象の実行は未対応です', h('p', {}, plan.unsupported_reason), h('p', {}, '理解と試験の計画は表示できますが、麦漆では起動しません。')));
  const header = [field({ label: '検証する動作', control: pick }), checks.length ? null : h('p', {}, '先に「検証する動作を決める」で確認項目を登録してください。')];
  if (!check) { fill(body, header, notes); return; }
  if (!check.source) notes.push(banner('warning', 'コードとの対応づけがありません', h('p', {}, '試験の結果をコードへたどれるように、先に「実装と確認方法を対応づける」でコードの行を選んでください。'), button('確認項目を開く', () => goto('expect'))));
  else if (check.source.commit !== state.source.commit) notes.push(banner('warning', '対応づけたコードのコミットが、現在読んでいるコミットと異なります', h('p', {}, `対応づけ: ${shortCommit(check.source.commit)}、現在: ${shortCommit(state.source.commit)}。現在のコミットで試すには、結果の画面の「修正した版で再試験する」から対応づけを更新してください。試験は対応づけたコミットで行います。`)));
  const form = state.trialForm || (lastRecipe(check.id) ? recipeToForm(lastRecipe(check.id)) : trialDefaults('normal'));
  state.trialForm = form;
  fill(body, header, notes,
    h('p', {}, h('strong', {}, '検証する動作: '), check.text),
    plan && !plan.error ? disclosure('コードから読み取った試験の手がかり', planInfo(plan)) : null,
    trialFormView(check, form, plan, exec));
}
function planInfo(plan) {
  return h('div', {},
    h('h3', { class: 'dads-u-std-20B-150' }, '試す前に確認すること'),
    disclosure(`起動ファイルの候補（${plan.entries.length} 件）`, h('ul', { class: 'items' }, plan.entries.map(e => h('li', {}, basis(e.kind), `${e.entrypoint}: ${e.reason}${e.supported ? '' : '（実行は未対応）'}`, ' ', refLinks(e.refs))))),
    disclosure(`コマンドのオプション（${plan.options.length} 件）`, h('ul', { class: 'items' }, plan.options.map(o => h('li', {}, codeText(o.flags.join(' / ')), o.help ? `: ${o.help}` : '', o.default != null ? `（既定値 ${o.default}）` : '', ' ', refLinks(o.refs))))),
    disclosure(`保存先の手がかり（${plan.storage.length} 件）`, items(plan.storage)),
    disclosure(`既存のテストとコマンド（${plan.test_commands.length} 件）`, h('ul', { class: 'items' }, plan.test_commands.map(c => h('li', {}, codeText(c.text), ` ${c.reason}`, ' ', refLinks(c.refs)))), plan.examples.length ? h('p', {}, `テストの例 ${plan.examples.length} 件は「検証する動作を決める」で参照できます。`) : null),
    disclosure(`依存パッケージ（${plan.dependencies.length} 件）`, plan.dependencies.length ? h('ul', { class: 'items' }, plan.dependencies.map(d => h('li', {}, `${d.name}（${d.file}）`, ' ', refLinks(d.refs)))) : h('p', {}, '見つかりませんでした。')),
    h('h4', {}, '実行に不足している設定'), h('ul', { class: 'items' }, plan.missing.map(m => h('li', {}, basis(m.kind), m.text, ' ', refLinks(m.refs || [])))),
    h('p', { class: 'note' }, h('strong', {}, '隔離の範囲: '), plan.isolation));
}
function trialFormView(check, form, plan, exec) {
  const entries = (plan?.entries || []).filter(e => e.supported);
  const launchSelect = value => { const s = select(LAUNCHES); s.control.value = value || 'file'; return s; };
  const entry = input({ value: form.entrypoint, list: 'entry-candidates' });
  const launch = launchSelect(form.launch);
  const runtime = select([['python3', 'Python 3'], ['node', 'Node.js'], ['http', 'HTTP（コンテナへ）']]); runtime.control.value = form.runtime;
  const args = textarea({ rows: 3, value: form.args.join('\n') });
  const storage = input({ value: form.storage });
  const readbackEntry = input({ value: form.readback?.entrypoint || '', list: 'entry-candidates' });
  const readbackLaunch = launchSelect(form.readback?.launch);
  const readbackArgs = textarea({ rows: 2, value: (form.readback?.args || []).join('\n') });
  const pythonRoot = input({ value: form.python_root || '' });
  const dependencies = textarea({ rows: 2, value: form.dependencies_text || '' });
  const serverEntry = input({ value: form.server?.entrypoint || '', list: 'entry-candidates' });
  const serverLaunch = launchSelect(form.server?.launch);
  const serverArgs = textarea({ rows: 3, value: (form.server?.args || []).join('\n') });
  const serverReady = input({ type: 'number', value: form.server?.ready_timeout_ms || 10000, min: 1, max: 60000, inputmode: 'numeric' });
  const timeout = input({ type: 'number', value: form.timeout_ms, min: 100, max: 300000, inputmode: 'numeric' });
  const containers = textarea({ rows: 8, value: form.containers_text || '' });
  const httpService = input({ value: form.http?.service || '' });
  const httpMethod = select([['GET', 'GET'], ['POST', 'POST'], ['PUT', 'PUT']]); httpMethod.control.value = form.http?.method || 'GET';
  const httpPath = input({ value: form.http?.path || '/' });
  const httpBody = input({ value: form.http?.body_file || '' });
  const httpType = input({ value: form.http?.content_type || '' });
  const faults = h('fieldset', {}, h('legend', {}, '故障の種類'), (plan?.faults || Object.keys(FAULT_LABEL).map(id => ({ id, label: FAULT_LABEL[id], applicable: '' }))).map(f => {
    return h('label', { class: 'dads-radio', 'data-size': 'md' },
      h('span', { class: 'dads-radio__radio' }, h('input', { class: 'dads-radio__input', type: 'radio', name: 'fault', value: f.id, checked: f.id === form.fault, disabled: f.id === 'network_unavailable' && exec?.mode !== 'bubblewrap' })),
      h('span', { class: 'dads-radio__label' }, h('strong', {}, FAULT_LABEL[f.id] || f.label), f.how ? ` ${f.how}` : '', f.applicable ? `（${f.applicable}）` : ''));
  }));
  const rows = h('div', {});
  const addRow = c => {
    const scope = input({ value: c.scope, list: 'scope-candidates' });
    const method = select(METHODS.map(([v, l, s]) => [v, `${l}（${s}）`])); method.control.value = c.method;
    const expect = select([]);
    const text = input({ value: c.text });
    const setExpect = () => {
      const isExit = method.control.value === 'exit_code';
      const options = isExit ? [['zero', '0（正常終了）で終わる'], ['nonzero', '0以外（失敗）で終わる']] : [['contains', '次の文字列を含む'], ['not_contains', '次の文字列を含まない']];
      const prev = expect.control.value || c.expect;
      expect.control.replaceChildren(...options.map(([v, l]) => h('option', { value: v }, l)));
      expect.control.value = options.some(o => o[0] === prev) ? prev : options[0][0];
      text.control.disabled = isExit;
    };
    method.control.addEventListener('change', setExpect); setExpect();
    const row = h('div', { class: 'check-row' }, h('div', { class: 'inline-fields' }, field({ label: '確認する範囲', control: scope }), field({ label: '観測の方法', control: method }), field({ label: '期待する結果', control: expect }), field({ label: '文字列', control: text, support: '{{test_value}} は試験用の値に置き換わります。' })), button('この条件を外す', () => row.remove(), 'text'));
    row.read = () => ({ scope: scope.control.value.trim(), method: method.control.value, expect: expect.control.value, text: method.control.value === 'exit_code' ? null : text.control.value });
    rows.append(row);
  };
  form.checks.forEach(addRow);
  const read = () => {
    const fault = document.querySelector('input[name="fault"]:checked')?.value || 'normal';
    const lines = t => t.split('\n').map(s => s.trim()).filter(Boolean);
    return { entrypoint: entry.control.value.trim(), launch: launch.control.value, runtime: runtime.control.value, args: lines(args.control.value), storage: storage.control.value.trim(), timeout_ms: Number(timeout.control.value) || 10000, fault,
      readback: readbackEntry.control.value.trim() ? { launch: readbackLaunch.control.value, entrypoint: readbackEntry.control.value.trim(), args: lines(readbackArgs.control.value) } : null,
      python_root: pythonRoot.control.value.trim(), dependencies_text: dependencies.control.value,
      server: serverEntry.control.value.trim() ? { launch: serverLaunch.control.value, entrypoint: serverEntry.control.value.trim(), args: lines(serverArgs.control.value), ready_timeout_ms: Number(serverReady.control.value) || 10000 } : null,
      containers_text: containers.control.value,
      http: httpService.control.value.trim() ? { service: httpService.control.value.trim(), method: httpMethod.control.value, path: httpPath.control.value.trim() || '/', body_file: httpBody.control.value.trim() || null, content_type: httpType.control.value.trim() || null } : null,
      checks: [...rows.children].map(r => r.read()) };
  };
  const preview = h('div', {});
  const updatePreview = () => { state.trialForm = read(); preview.replaceChildren(previewBox(check, state.trialForm, exec)); };
  const runtimeNote = h('div', {});
  const submit = h('button', { class: 'dads-button', 'data-type': 'solid-fill', 'data-size': 'lg', type: 'submit' }, '実行前に確認する');
  const updateRuntime = () => {
    const rt = exec?.runtimes?.find(r => r.name === runtime.control.value);
    runtimeNote.replaceChildren(rt ? h('p', { class: rt.available ? 'note' : 'ref-unverified' }, rt.available ? `実行環境: ${rt.detail}` : `実行環境がないため試せません: ${rt.detail}`) : h('p', { class: 'note' }, '実行環境を確認できません。'));
    submit.disabled = !check.source || !rt?.available || (runtime.control.value !== 'http' && !plan?.executable);
  };
  runtime.control.addEventListener('change', () => { updateRuntime(); updatePreview(); });
  for (const [box, kind] of [[entry, launch], [readbackEntry, readbackLaunch], [serverEntry, serverLaunch]])
    box.control.addEventListener('change', () => { const c = entries.find(e => e.entrypoint === box.control.value.trim()); if (c) kind.control.value = c.launch || 'file'; });
  updateRuntime();
  const container = h('form', { novalidate: true, oninput: updatePreview, onchange: updatePreview, onsubmit: e => { e.preventDefault(); state.trialForm = read(); showTrialConfirmation(check, state.trialForm, exec); } },
    h('datalist', { id: 'entry-candidates' }, entries.map(e => h('option', { value: e.entrypoint, label: LAUNCH_LABEL[e.launch || 'file'] }))),
    h('datalist', { id: 'scope-candidates' }, SCOPES.map(s => h('option', { value: s }))),
    h('h3', { class: 'dads-u-std-20B-150' }, '試験の内容'),
    h('div', { class: 'button-row' }, button('通常時の設定例を使う', () => { state.trialForm = trialDefaults('normal'); renderTrial(); }), button('保存失敗時の設定例を使う', () => { state.trialForm = trialDefaults('failure'); renderTrial(); }),
      h('span', { class: 'note' }, 'コードのオプション定義から作った設定例です。実行前に内容を確認し、必要な箇所を修正してください。')),
    h('div', { class: 'inline-fields' }, field({ label: '起動対象', control: entry, required: true, support: 'ファイルは固定コミットの中の相対パス。モジュールは pynetdicom のような名前。コマンドは pyproject.toml の [project.scripts] に書かれた名前。' }), field({ label: '起動のしかた', control: launch }), field({ label: '実行環境', control: runtime })), runtimeNote,
    field({ label: '引数', control: args, support: '1行に1つ。試験用の値を入れる位置に {{test_value}} と書くと、試行ごとに新しい架空の値（eel-test-…）に置き換わります。先に起動するプロセスのポートは {{port}}、固定コミットの複製の最上位は {{source}} と書きます（例: {{source}}/tests/data/sample.dcm）。' }),
    field({ label: '保存先', control: storage, support: '試行用の作業ディレクトリからの相対パス。例: tasks。指定すると、試行の後に麦漆が保存先を直接読み出します。' }),
    h('fieldset', {}, h('legend', {}, 'Pythonの読み込み先と依存先（任意）'),
      h('p', { class: 'note' }, '依存先は、公開GitHubのリポジトリを固定コミットで指定します。試験を始めると、麦漆が固定コミットだけを取得し（git fetch）、読み込み先のディレクトリを読み取り専用で複製します。インストール用のスクリプトやビルドは実行しません。Pythonだけで書かれた依存先に限ります。モジュール・コマンドで起動するときや依存先を指定したときは、ホストに入っているパッケージを読まない設定（-S -P）で起動します。'),
      h('div', { class: 'inline-fields' }, field({ label: '対象の読み込み先', control: pythonRoot, support: '対象リポジトリの中のディレクトリ。空なら最上位。例: src' }),
        field({ label: '依存先', control: dependencies, support: '1行に1つ、「所有者/リポジトリ 40桁のコミット 読み込み先（任意）」。例: pydicom/pydicom 20aa4b73aef76bffb6fbc46a8827588dfa9bb89d src' }))),
    h('fieldset', {}, h('legend', {}, '試験専用のコンテナ（任意）'),
      h('p', { class: 'note' }, 'イメージはタグだけではなく @sha256: と64桁のダイジェストを付けます。試行ごとに新しいコンテナ・ネットワーク・ボリュームを作り、待ち受けは 127.0.0.1 だけに公開し、終わったら消します。Dockerが使えないときはほかの方式へ切り替えません。対象リポジトリの compose をそのまま実行はしません。'),
      field({ label: 'コンテナ（JSONの配列）', control: containers, support: '各要素は name, image, role（app / database / other）, expose, ready（tcp または {"http":{"path":"/","tls":false}}）, ready_timeout_ms。任意で env, volumes, command。例のポートは {{port:名前}} で引数に使えます。' }),
      h('div', { class: 'inline-fields' }, field({ label: 'HTTPの操作先', control: httpService }), field({ label: 'HTTPの方法', control: httpMethod }), field({ label: 'HTTPのパス', control: httpPath })),
      h('div', { class: 'inline-fields' }, field({ label: 'HTTPの本文ファイル', control: httpBody, support: '固定コミットの中の相対パス。空なら本文なし。' }), field({ label: 'Content-Type', control: httpType }))),
    h('fieldset', {}, h('legend', {}, '先に起動するプロセス（任意）'),
      h('p', { class: 'note' }, '受信サーバーのように待ち受けるプロセスを先に起動し、指定したポートが接続を受け付けてから、上の「起動対象」を動かします。終わったら麦漆が止めます。止めたことを先に起動するプロセスの失敗とは扱いません。準備できなかったときは「判定できない」にします。'),
      h('div', { class: 'inline-fields' }, field({ label: '先に起動するプロセス', control: serverEntry }), field({ label: '起動のしかた', control: serverLaunch }), field({ label: '準備の待機上限（ミリ秒）', control: serverReady })),
      field({ label: '先に起動するプロセスの引数', control: serverArgs, support: '1行に1つ。待ち受けるポートの位置に {{port}} と書くと、麦漆が 127.0.0.1 の空いているポートを割り当てます。待ち受けるアドレスは 127.0.0.1 にしてください（local方式ではOSで制限しません）。' })),
    h('fieldset', {}, h('legend', {}, '後から読み出す（任意）'), h('p', { class: 'note' }, '書き込んだプロセス（と先に起動するプロセス）が終わった後、別のプロセスで同じ作業ディレクトリを読み出します。'),
      h('div', { class: 'inline-fields' }, field({ label: '読み出し用の起動対象', control: readbackEntry }), field({ label: '起動のしかた', control: readbackLaunch }), field({ label: '読み出しの引数', control: readbackArgs, support: '1行に1つ。' }))),
    faults,
    h('fieldset', {}, h('legend', {}, '判定の条件'), h('p', { class: 'note' }, '終了コードだけでは成功と判定しません。終了コード以外の条件を少なくとも1つ入れてください。「確認する範囲」が同じ条件は、同じことを別の方法で確かめる条件として扱い、結果が食い違ったときは「判定できない」になります。別のことを確かめる条件には、別の範囲の名前を付けてください。'), rows, button('条件を追加', () => { addRow({ scope: '', method: 'stdout', expect: 'contains', text: '' }); updatePreview(); })),
    field({ label: '待機の上限（ミリ秒）', control: timeout }),
    disclosure('実行内容の要約', preview),
    h('div', { class: 'button-row' }, submit));
  for (const group of [...container.querySelectorAll(':scope > fieldset')]) {
    const title = group.querySelector('legend')?.textContent || '';
    if (!title.includes('任意')) continue;
    const details = disclosure(title); group.replaceWith(details); details.lastChild.append(group);
  }
  // 起動設定・故障・判定条件を一つずつ選ぶ。入力は既存の試験案に保持する。
  const stage = state.step === 'trial-fault' ? 'fault' : state.step === 'trial-checks' ? 'checks' : 'setup';
  const children = [...container.children], faultIndex = children.indexOf(faults);
  children.forEach((child, i) => {
    child.hidden = stage === 'setup' ? i >= faultIndex : stage === 'fault' ? child !== faults : i <= faultIndex;
  });
  runtimeNote.hidden = false;
  const actions = h('div', { class: 'journey-actions' });
  if (stage === 'setup') actions.append(button('確認項目に戻る', () => goto('expect-edit')), button('次へ：試す状況を選ぶ', () => { state.trialForm = read(); goto('trial-fault'); }, 'solid-fill', 'md'));
  if (stage === 'fault') actions.append(button('起動と入力に戻る', () => { state.trialForm = read(); goto('trial'); }), button('次へ：判定の条件を確認する', () => { state.trialForm = read(); goto('trial-checks'); }, 'solid-fill', 'md'));
  if (stage === 'checks') {
    children.at(-1).hidden = true;
    actions.append(button('試す状況に戻る', () => { state.trialForm = read(); goto('trial-fault'); }), submit);
  }
  container.append(actions);
  setTimeout(updatePreview);
  return container;
}
function showTrialConfirmation(check, form, exec) {
  const body = $('trial-body');
  const run = button('試験を始める', async () => {
    if (run.disabled) return;
    run.disabled = true;
    await runTrial(check, form);
    run.disabled = false;
  }, 'solid-fill', 'lg');
  run.disabled = !check.source;
  body.replaceChildren(h('h3', {}, 'この内容で実行します'), h('p', {}, '次のボタンを押すと、固定版を取得して対象を起動します。'), previewBox(check, form, exec), h('div', { class: 'journey-actions' }, button('設定に戻る', () => { history.back(); }), run));
  history.pushState(null, '', location.href.replace(/#.*$/, '') + '#trial-confirm');
  $('trial').focus(); $('main').scrollIntoView({ block: 'start' });
}
function previewBox(check, f, exec) {
  const commit = check.source?.commit;
  let python = null, pythonError = null;
  try { python = pythonSetup(f); } catch (e) { pythonError = e.message; }
  return h('div', { class: 'box' },
    h('h4', {}, '実行する内容'),
    h('dl', {},
      h('dt', {}, '対象'), h('dd', {}, `${check.source?.repository || state.source.repository} のコミット ${commit || '（対応づけが必要）'}`),
      h('dt', {}, '依存先'), h('dd', {}, pythonError ? h('span', { class: 'ref-unverified' }, pythonError)
        : python?.dependencies.length ? h('ul', { class: 'items' }, python.dependencies.map(d => h('li', {}, `${d.repository} のコミット ${d.commit} の ${d.root || '最上位'}（試験を始めると固定コミットだけを取得します。インストールはしません）`)))
        : 'なし'),
      h('dt', {}, '試験専用のコンテナ'), h('dd', {}, (() => { try { const c = parseContainers(f.containers_text); return c ? c.services.map(s => `${s.name}（${s.image}、127.0.0.1 の {{port:${s.name}}}）`).join('、') : 'なし'; } catch (e) { return h('span', { class: 'ref-unverified' }, e.message); } })()),
      h('dt', {}, 'HTTPの操作'), h('dd', {}, f.http ? codeText(`${f.http.method} ${f.http.service} ${f.http.path}`) : 'なし'),
      h('dt', {}, '先に起動するプロセス'), h('dd', {}, f.server ? [codeText(launchCommand(f, f.server.launch, f.server.entrypoint, f.server.args)), `（${LAUNCH_LABEL[f.server.launch]}。${f.server.ready_timeout_ms}ミリ秒以内に {{port}} のポートが接続を受け付けなければ「判定できない」）`] : 'なし'),
      h('dt', {}, '起動のしかた'), h('dd', {}, codeText(launchCommand(f, f.launch, f.entrypoint, f.args)), f.launch === 'file' ? '' : `（${LAUNCH_LABEL[f.launch]}）`),
      h('dt', {}, '入力'), h('dd', {}, '上の引数だけ。input.json には空のJSON（{}）を置きます。試験用の値は試行ごとに新しく作ります。'),
      h('dt', {}, '保存先'), h('dd', {}, f.storage ? `作業ディレクトリの ${f.storage}` : '指定なし（麦漆による保存先の読み出しは行いません）'),
      h('dt', {}, '故障'), h('dd', {}, FAULT_LABEL[f.fault]),
      h('dt', {}, '後からの読み出し'), h('dd', {}, f.readback ? [codeText(launchCommand(f, f.readback.launch, f.readback.entrypoint, f.readback.args)), f.readback.launch === 'file' ? '' : `（${LAUNCH_LABEL[f.readback.launch]}）`] : '行いません'),
      h('dt', {}, '判定の条件'), h('dd', {}, h('ul', { class: 'items' }, f.checks.map(c => h('li', {}, `${c.scope || '（範囲が未入力）'}: ${METHODS.find(m => m[0] === c.method)?.[1]} が ${{ zero: '0で終わる', nonzero: '0以外で終わる', contains: `「${c.text}」を含む`, not_contains: `「${c.text}」を含まない` }[c.expect]}（${METHODS.find(m => m[0] === c.method)?.[2]}）`)))),
      h('dt', {}, '実行の方式'), h('dd', {}, `${exec?.mode || '不明'}方式。${exec?.mode === 'local' ? 'OSによる隔離は行いません。' : ''}固定コミットのファイルを読み取り専用で複製し、試行ごとの新しい作業ディレクトリで起動します。依存パッケージのインストールは行いません。既存の保存データは使わず、置き換えません。${f.server ? '先に起動するプロセスとの通信は 127.0.0.1 のポートで行います。' : ''}${f.containers_text?.trim() ? 'コンテナの待ち受けは 127.0.0.1 だけに公開し、試行の後に消します。' : ''}`)));
}
async function runTrial(check, f) {
  await guarded(async () => {
    requirePerson();
    if (f.runtime !== 'http' && !f.entrypoint) throw Error('起動対象を入力してください。');
    if (f.runtime === 'http' && !f.http) throw Error('HTTPの操作先を入力してください。');
    const python = pythonSetup(f);
    const containers = parseContainers(f.containers_text);
    if ([f.launch, f.readback?.launch, f.server?.launch].some(l => l && l !== 'file') && f.runtime !== 'python3') throw Error('モジュール・宣言されたコマンドでの起動は、Python 3 だけで使えます。');
    if (f.server && !f.server.args.some(a => a.includes('{{port}}'))) throw Error('先に起動するプロセスの引数に、ポートの位置として {{port}} を入れてください。');
    if (!f.checks.length || f.checks.some(c => !c.scope)) throw Error('判定の条件を入力し、それぞれの確認する範囲を書いてください。');
    if (f.checks.every(c => c.method === 'exit_code')) throw Error('終了コードだけでは判定しません。終了コード以外の条件を追加してください。');
    const criterion = { assertion: 'eel.trial', subject: check.id };
    let current = currentCheck();
    if (current.rule !== 'observed_assertion' || JSON.stringify(current.observation) !== JSON.stringify(criterion)) {
      await change({ op: 'configure_observation', check_id: check.id, criterion }, '試験案の判定条件で判定するように設定した');
      current = currentCheck();
    }
    const recipe = { schema_version: 1, check_id: check.id, source: current.source, runtime: f.runtime, launch: f.launch, entrypoint: f.runtime === 'http' ? (f.http?.path || '/') : f.entrypoint, args: f.args, input: {}, criterion, timeout_ms: f.timeout_ms,
      python, server: f.server, containers, http: f.http,
      trial: { schema_version: 1, fault: f.fault, storage: f.storage || null, readback: f.readback, checks: f.checks.map((c, i) => ({ id: `c${i + 1}`, scope: c.scope, method: c.method, expect: c.expect, text: c.method === 'exit_code' ? null : c.text, target: c.target || null })) } };
    setStatus(`固定コミット ${shortCommit(current.source.commit)} を試験用に取得しています（git fetch のみ。まだ実行しません）。`);
    await api('POST', '/api/v1/prepare', { repository: current.source.repository, commit: current.source.commit });
    const p = person();
    setStatus(python?.dependencies.length ? '依存先の固定コミットを取得してから（git fetch のみ）、対象を起動しています。' : '対象を起動しています。');
    const job = await api('POST', '/api/v1/execute', { expected_revision: state.view.workspace.revision, actor: `${p.name}（${ROLE[p.role]}）`, reason: `${FAULT_LABEL[f.fault]}で試験した`, recipe });
    state.lastRun = `${job.id}（実行中）`; renderCurrent();
    let status;
    for (let i = 0; i < 900; i++) {
      await new Promise(r => setTimeout(r, 500));
      status = await api('GET', '/api/v1/execution');
      if (status.job?.id === job.id && status.job.status !== 'running') break;
    }
    state.execution = status;
    if (status.job.status !== 'completed') throw Error(`試験を完了できませんでした: ${status.job.error || '状態を確認できません'}`);
    state.view = await api('GET', '/api/v1/case');
    const ev = checkView(check.id)?.evidence.at(-1);
    state.lastRun = ev ? `${ev.run_id}（${FAULT_LABEL[f.fault]}）` : job.id;
    renderCurrent();
    setStatus('試験が終わり、結果を記録しました。');
    goto('result');
  });
}

// ---------- 4. 結果 ----------
const DIFF_LABEL = { '/source': 'コード参照（コミット・ファイル・行）', '/execution/recipe': '試験の内容（起動・引数・判定条件・故障）', '/execution/tree': 'コミットのファイル一式（tree）', '/execution/exit_code': '終了コード', '/execution/trial': '判定条件ごとの結果と保存先の読み出し', '/execution/python': '依存先の固定コミットとパッケージ情報', '/execution/containers': '試験専用コンテナのイメージと準備', '/execution/runtime_version': '実行環境のバージョン', '/execution/runtime_path': '実行環境の場所', '/execution/mode': '実行の方式', '/check_version': '条件の版', '/manifest/command': '実行したコマンド', '/manifest/condition': '確認項目と版', '/manifest/input': '入力', '/manifest/fault': '故障', '/manifest/source_fingerprint': '対象ファイルの指紋', '/manifest/program_version': '対象の版', '/manifest/mode': '実行の方式（記録上）', '/manifest/os': 'OS', '/manifest/architecture': 'CPUの種類', '/manifest/timeout_ms': '待機の上限', '/manifest/initial_storage': '試行前の保存先', '/manifest/observation_scope': '観測の範囲', '/observation': '判定する観測', '/rule': '判定規則' };
function renderResult() {
  const body = $('result-body');
  const checks = repoChecks();
  if (!state.view) { body.replaceChildren(h('p', {}, '記録を読み込んでいます。')); return; }
  const pick = select([['', '選んでください'], ...checks.map(c => [c.id, c.text])]);
  pick.control.value = state.checkId || '';
  pick.control.addEventListener('change', () => { state.checkId = pick.control.value || null; renderCurrent(); goto('result', false); });
  const check = currentCheck();
  if (!check) { body.replaceChildren(field({ label: '確認項目', control: pick })); return; }
  const cv = checkView(check.id);
  const evidence = [...cv.evidence].reverse();
  const raw = id => state.view.workspace.evidence.find(e => e.id === id);
  fill(body,
    h('p', { class: 'note', role: 'status' }, state.view.verification_passed
      ? '記録した全条件の検証を通過。実運用の許可を示すものではありません。'
      : '検証未通過。未確認または未達の条件があり、「採用する」という判断は記録できません。'),
    retestNotice(check),
    field({ label: '確認項目', control: pick }),
    h('p', {}, h('strong', {}, '検証する動作: '), check.text, `（条件の版 ${check.version}）`),
    h('h3', { class: 'dads-u-std-20B-150' }, '現在の判定'),
    verdictBox(cv.verdict, cv.reason, h('p', {}, h('strong', {}, '次に確認すること: '), cv.next.text)),
    h('p', { class: 'note' }, '現在の判定は、最新の試行だけから決めます。最新の試行で観測が足りないときに、前の試行の成功で補いません。'),
    h('h3', { class: 'dads-u-std-20B-150' }, `試行の記録（${evidence.length} 件、新しい順）`),
    evidence.length ? [evidenceCard(check, evidence[0], raw(evidence[0].id)), evidence.length > 1 ? disclosure('過去の試行を見る', evidence.slice(1).map(v => evidenceCard(check, v, raw(v.id)))) : null] : h('p', { class: 'note' }, 'まだ試行はありません。', button('試験の画面へ進む', () => goto('trial'), 'solid-fill', 'md')),
    evidence.length >= 2 ? compareForm(check, evidence) : null,
    disclosure('修正した版で再試験する', retestForm(check)),
    disclosure('判断を記録・確認する', decisionForm(check), decisionsList(check)));
  const requested = new URLSearchParams(location.search).get('evidence');
  if (requested && state.focusedEvidence !== requested) {
    const target = [...body.querySelectorAll('[data-evidence]')].find(node => node.dataset.evidence === requested);
    if (target) {
      for (let parent = target.parentElement; parent && parent !== body; parent = parent.parentElement) if (parent.tagName === 'DETAILS') parent.open = true;
      target.tabIndex = -1;
      target.focus();
      state.focusedEvidence = requested;
    }
  }
}
function evidenceCard(check, v, e) {
  const s = v.summary;
  const comments = state.view.workspace.statements.filter(x => x.evidence_id === v.id);
  const comment = textarea({ rows: 2 });
  const trace = e?.execution;
  return h('div', { class: 'box', 'data-evidence': v.id },
    h('h4', {}, `試行 ${v.run_id}`),
    v.applicable ? null : banner('warning', 'この実行結果は現在の確認項目に適用できません', h('p', {}, `再確認が必要な理由: ${v.invalidated_by.join('、')}`)),
    verdictBox(v.observed.verdict, v.observed.reason),
    s ? h('div', {},
      h('p', {}, h('strong', {}, '試したこと: '), s.tried),
      h('p', {}, h('strong', {}, '故障: '), s.fault, s.fault_applied ? `（${s.fault_applied}）` : ''),
      h('p', {}, h('strong', {}, '試験用の値: '), codeText(s.test_value || '（なし）')),
      s.dependencies?.length ? h('p', {}, h('strong', {}, '依存先: '), s.dependencies.join('、'), '（固定コミットの読み込み先だけを複製。インストールはしていない）') : null,
      s.server ? h('p', {}, h('strong', {}, '先に起動するプロセス: '), s.server.started ? `${s.server.detail}。${s.server.exited_before_stop ? `麦漆が止める前に終了していた（終了コード ${s.server.exit_code ?? 'なし'}）` : s.server.stopped_by_platform ? '操作の後に麦漆が止めた' : ''}` : s.server.detail) : null,
      s.containers ? h('p', {}, h('strong', {}, '試験専用コンテナ: '), s.containers.services.map(x => `${x.name}（${x.ready ? `${x.ready_after_ms}msで準備` : '準備できず'}、127.0.0.1:${x.port}）`).join('、'), s.containers.leftovers?.length ? `。残った資源: ${s.containers.leftovers.join('、')}` : '。試行の後に消した') : null,
      h('figure', { class: 'dads-table' }, h('figcaption', { class: 'dads-table__caption' }, '条件ごとの判定結果'),
        h('table', { class: 'dads-table__table', 'data-cell-border': 'bottom' },
          h('thead', {}, h('tr', {}, ['確認する範囲', '期待する結果', '観測した結果', '記録の取得元', '詳細'].map(t => h('th', { class: 'dads-table__col-header', scope: 'col' }, t)))),
          h('tbody', {}, s.items.map(i => h('tr', {}, h('td', {}, i.scope), h('td', {}, i.expected), h('td', { class: 'observed', 'data-observed': i.observed || 'not_observed' }, OBSERVED[i.observed] || '？ 記録がない'), h('td', {}, i.source), h('td', {}, i.detail)))))),
      s.state ? h('p', {}, h('strong', {}, '保存先の読み出し（麦漆）: '), s.state.detail, s.state.excerpt ? h('pre', { class: 'output' }, s.state.excerpt) : null) : null)
      : h('p', { class: 'note' }, 'この試行は判定条件の表がない形式です。技術的な詳細を確認してください。'),
    h('p', {}, h('strong', {}, '次に確認すること: '), v.observed.next_action || '（記録なし）'),
    v.observed.unconfirmed?.length ? disclosure(`確かめていないこと（${v.observed.unconfirmed.length} 件）`, h('ul', { class: 'items' }, v.observed.unconfirmed.map(u => h('li', {}, u)))) : null,
    trace ? h('p', {}, h('strong', {}, 'コードへ戻る: '), `試したコミット ${shortCommit(trace.recipe.source.commit)} の `, button(`${trace.recipe.source.path} ${trace.recipe.source.line}行目（対応づけたコード）`, () => openTrialCode(trace.recipe.source, trace.recipe.source.path, trace.recipe.source.line, trace.recipe.source.end_line), 'text'), ...launchLinks(trace)) : null,
    trace ? disclosure('技術的な詳細（コマンド・出力・記録）',
      h('p', {}, `実行の方式: ${trace.mode}。実行環境: ${trace.runtime_version}（${trace.runtime_path}）。終了コード: ${trace.exit_code ?? 'なし'}。待機上限: ${trace.timed_out ? '達した' : '達していない'}。`),
      s ? h('p', {}, 'コマンド: ', codeText(s.command.join(' '))) : null,
      s?.readback_command ? h('p', {}, '読み出しのコマンド: ', codeText(s.readback_command.join(' '))) : null,
      s?.server ? [h('p', {}, `先に起動するプロセスのコマンド（ポート ${s.server.port}）: `, codeText(s.server.command.join(' '))),
        h('p', {}, '先に起動するプロセスの標準出力（抜粋）'), h('pre', { class: 'output' }, s.server.stdout_excerpt || '（なし）'),
        h('p', {}, '先に起動するプロセスの標準エラー（抜粋）'), h('pre', { class: 'output' }, s.server.stderr_excerpt || '（なし）')] : null,
      s?.containers ? [h('p', {}, `Docker ${s.containers.engine}。ネットワーク ${s.containers.network}`),
        s.containers.http ? h('p', {}, s.containers.http.detail) : null,
        s.containers.database_query ? h('p', {}, s.containers.database_query) : null,
        ...s.containers.services.flatMap(x => [h('p', {}, `コンテナ ${x.name}（${x.image}、記録したダイジェスト ${x.digest}）: ${x.detail}`), h('pre', { class: 'output' }, x.log_excerpt || '（ログなし）')])] : null,
      trace.python ? [h('p', {}, `Pythonの読み込み先（この順）: ${trace.python.import_path.join('、')}。起動の設定: ${trace.python.flags.join(' ')}`),
        h('ul', { class: 'items' }, trace.python.packages.map(p => h('li', {}, `${p.role === 'target' ? '対象' : '依存先'} ${p.repository}（コミット ${shortCommit(p.commit)}、読み込み先 ${p.root || '最上位'}、${p.files} ファイル）: ${p.note}`)))] : null,
      h('p', {}, '標準出力（抜粋）'), h('pre', { class: 'output' }, trace.stdout_excerpt || '（なし）'),
      h('p', {}, '標準エラー（抜粋）'), h('pre', { class: 'output' }, trace.stderr_excerpt || '（なし）'),
      trace.trial?.readback ? [h('p', {}, '読み出しの標準出力（抜粋）'), h('pre', { class: 'output' }, trace.trial.readback.stdout_excerpt || '（なし）')] : null,
      h('p', {}, '試験の設定（JSON）'), h('pre', { class: 'output' }, JSON.stringify(trace.recipe, null, 2))) : null,
    disclosure(`この結果への質問・異論（${comments.length} 件）`, h('h5', {}, '質問・異論'), comments.map(statementBlock),
      field({ label: '質問・異論', control: comment, support: '技術者もそれ以外の人も書けます。記入者と事例の版を一緒に残します。' }),
      button('質問・異論を記録', () => guarded(async () => {
        const p = requirePerson(); if (!comment.control.value.trim()) throw Error('質問・異論を入力してください。');
        await change({ op: 'add_statement', statement: { id: randomId('s'), kind: 'comment', role: p.role, text: comment.control.value.trim(), origin: `${p.name}の質問・異論`, check_ids: [check.id], evidence_id: v.id } }, '試行の結果に質問・異論を書いた');
        renderResult();
        const recorded = [...$('result-body').querySelectorAll('[data-evidence]')].find(node => node.dataset.evidence === v.id);
        const discussion = [...(recorded?.querySelectorAll('details') || [])].find(node => node.querySelector(':scope > summary')?.textContent.startsWith('この結果への質問・異論'));
        if (discussion) discussion.open = true;
        setStatus('質問・異論を記録しました。');
      }))));
}
/// 起動したもののコードへのリンク。モジュール・コマンドは、麦漆が解決した固定版の中のファイルへつなぐ。
function launchLinks(trace) {
  const source = trace.recipe.source;
  const resolved = trace.python?.launches || [];
  const starts = [['operate', '起動したもの', trace.recipe.launch || 'file', trace.recipe.entrypoint], ...(trace.recipe.server ? [['wait', '先に起動するプロセス', trace.recipe.server.launch || 'file', trace.recipe.server.entrypoint]] : [])];
  return starts.flatMap(([role, label, launch, entrypoint]) => {
    const r = launch === 'file' ? { repository: source.repository, path: entrypoint } : resolved.find(x => x.role === role);
    if (!r) return [];
    return [' と ', r.repository === source.repository
      ? button(`${label} ${entrypoint}${launch === 'file' ? '' : `（${r.path}）`}`, () => openTrialCode(source, r.path, 1, 1), 'text')
      : `${label} ${entrypoint}（依存先 ${r.repository} の ${r.path}）`];
  });
}
async function openTrialCode(source, path, line, end) {
  if (source.commit !== state.source.commit) {
    const ok = await openRepository(`https://github.com/${source.repository}`, source.commit);
    if (!ok) return;
    goto('result', false);
  }
  await openCode(path, line, end);
}
function compareForm(check, evidence) {
  const options = evidence.map(v => [v.id, `${v.run_id}（${VERDICT[v.observed.verdict]?.label}）`]);
  const before = select(options), after = select(options);
  before.control.value = evidence[1].id; after.control.value = evidence[0].id;
  const out = h('div', { 'data-trial-comparison': '', 'aria-live': 'polite' });
  let generation = 0;
  const clearComparison = () => { generation++; out.replaceChildren(); };
  before.control.addEventListener('change', clearComparison);
  after.control.addEventListener('change', clearComparison);
  return h('div', {}, h('h3', { class: 'dads-u-std-20B-150' }, '二つの試行を比べる'),
    h('div', { class: 'inline-fields' }, field({ label: '前の試行', control: before }), field({ label: '後の試行', control: after })),
    button('比べる', () => guarded(async () => {
      clearComparison();
      const request = { expected_revision: state.view.workspace.revision, check_id: check.id, before_id: before.control.value, after_id: after.control.value };
      const current = generation;
      const isCurrent = () => current === generation && out.isConnected && state.view.workspace.revision === request.expected_revision && before.control.value === request.before_id && after.control.value === request.after_id;
      let res;
      try { res = await api('POST', '/api/v1/compare', request); }
      catch (error) { if (isCurrent()) throw error; return; }
      // 選択・事例の版・画面が変わった後に、古い応答を表示しない。
      if (!isCurrent()) return;
      const changed = res.differences.filter(d => d.status !== 'same');
      const label = d => DIFF_LABEL[d.key] || d.key;
      const short = v => { const t = v === null || v === undefined ? '（記録なし）' : JSON.stringify(v); return t.length > 600 ? t.slice(0, 600) + '…' : t; };
      out.replaceChildren(h('div', { class: 'box' },
        h('p', {}, `前: ${VERDICT[res.before.evidence.observed.verdict].label}（${res.before.evidence.run_id}）、後: ${VERDICT[res.after.evidence.observed.verdict].label}（${res.after.evidence.run_id}）`),
        h('p', {}, `比べた項目 ${res.differences.length} 件のうち、同じ ${res.differences.length - changed.length} 件、異なる・確認できない ${changed.length} 件。`),
        h('figure', { class: 'dads-table' }, h('figcaption', { class: 'dads-table__caption' }, '異なる項目・確認できない項目'), h('table', { class: 'dads-table__table', 'data-cell-border': 'bottom' },
          h('thead', {}, h('tr', {}, ['項目', '状態', '前', '後'].map(t => h('th', { class: 'dads-table__col-header', scope: 'col' }, t)))),
          h('tbody', {}, changed.map(d => h('tr', {}, h('td', {}, label(d)), h('td', {}, d.status === 'changed' ? '異なる' : '確認できない'), h('td', {}, h('pre', { class: 'output' }, short(d.before))), h('td', {}, h('pre', { class: 'output' }, short(d.after)))))))),
        h('ul', { class: 'items' }, res.limits.map(l => h('li', {}, l))),
        h('p', {}, h('a', { href: '/user-trial?' + new URLSearchParams({ check: res.check_id, before: res.before.evidence.id, after: res.after.evidence.id, revision: res.case_revision }) }, 'この比較から試用を準備する')),
        h('p', { class: 'note' }, '対象の版と検証記録を下書きへ引き継ぎます。試用する操作・条件と、起動した試用環境のURLは技術者が確認して記入します。')));
      setStatus('二つの試行を比べました。');
    })), out);
}
function retestForm(check) {
  const ref = input({ placeholder: 'main、v1.2、40桁のコミット' });
  return h('div', {}, h('h3', { class: 'dads-u-std-20B-150' }, '修正後の版を読み込む'),
    h('p', { class: 'note' }, '修正したブランチ・タグ・コミットを読み込み、対応づけたコードの行を新しいコミットで確かめてから試します。前の試行と判断の記録は残ります。結果が変わっても、修正が原因だとは断定しません。'),
    field({ label: '修正した版（ブランチ、タグ、またはコミット）', control: ref, required: true }),
    button('修正した版を読み込む', () => guarded(async () => {
      if (!ref.control.value.trim()) throw Error('修正した版を入力してください。');
      const prev = check.source;
      const res = await openRepository(`https://github.com/${state.source.repository}`, ref.control.value.trim());
      if (!res || !prev) return;
      state.checkId = check.id;
      state.retest = { checkId: check.id, prev };
      goto('result', false);
      await openCode(prev.path, prev.line, prev.end_line);
      $('retest-notice')?.focus();
    })));
}
function retestNotice(check) {
  const r = state.retest;
  if (!r || r.checkId !== check.id || !state.source || r.prev.commit === state.source.commit) return null;
  const { prev } = r;
  const box = banner('info', '新しいコミットでの対応づけを確かめてください', h('p', {}, `前の対応づけ: ${prev.path} ${prev.line}〜${prev.end_line}行目（コミット ${shortCommit(prev.commit)}）。現在のコミット ${shortCommit(state.source.commit)} のコードで、同じ処理がこの行にあるかを確かめてください。行が変わった場合は、コードの画面で行を選び直して「この行を確認項目に関連づける」を使います。`),
    h('div', { class: 'button-row' },
      button('前と同じ行をコードの画面で開く', () => openCode(prev.path, prev.line, prev.end_line)),
      button(`同じ行（${prev.line}〜${prev.end_line}行目）で対応づけを更新し、試験の画面へ進む`, () => guarded(async () => {
        await change({ op: 'bind_source', check_id: check.id, source: { ...prev, commit: state.source.commit } }, '修正した版のコミットへ、確認項目に対応するコードの行を更新した');
        state.retest = null; state.trialForm = null; goto('trial');
      }), 'solid-fill', 'md')));
  box.id = 'retest-notice'; box.setAttribute('tabindex', '-1');
  return box;
}
function decisionForm(check) {
  const action = select(Object.entries(DECISION));
  const reason = textarea({ rows: 2 });
  const reference = input({ placeholder: '例: 修正したコミットやPRのURL' });
  return h('div', {}, h('h3', { class: 'dads-u-std-20B-150' }, '判断を記録する'),
    h('p', { class: 'note' }, '判断には、その時点の判定と根拠が一緒に保存されます。判断を記録しても「合意済み」にはなりません。記録の確認は、それぞれの人が「内容を確認した」で残します。'),
    h('div', { class: 'inline-fields' }, field({ label: '判断', control: action }), field({ label: '改修への参照（任意）', control: reference })),
    field({ label: '理由', control: reason, required: true }),
    button('判断を記録', () => guarded(async () => {
      const p = requirePerson(); if (!reason.control.value.trim()) throw Error('理由を入力してください。');
      const r = state.view.workspace.revision;
      const view = await api('POST', '/api/v1/change', { expected_revision: r, actor: `${p.name}（${ROLE[p.role]}）`, reason: reason.control.value.trim(), action: { op: 'decide', role: p.role, action: action.control.value, check_ids: [check.id], change_reference: reference.control.value.trim() || null, confidence: null, elapsed_ms: null } });
      state.view = view; renderResult(); setStatus('判断を記録しました。');
    }), 'solid-fill', 'md'));
}
function decisionsList(check) {
  const list = state.view.workspace.decisions.filter(d => d.check_ids.includes(check.id)).reverse();
  if (!list.length) return null;
  return h('div', {}, h('h4', {}, `記録した判断（${list.length} 件）`), h('ul', { class: 'items' }, list.map(d => {
    const basisView = d.basis.find(b => b.check_id === check.id);
    return h('li', {}, `${DECISION[d.action] || d.action}: ${d.reason}（${d.actor}、事例の版 ${d.based_on_revision}${d.change_reference ? `、改修: ${d.change_reference}` : ''}）`, basisView ? h('div', { class: 'note' }, `判断の時点の判定: ${VERDICT[basisView.verdict].label}。${basisView.reason}`) : null);
  })));
}

// ---------- 起動 ----------
loadAi();
const params = new URLSearchParams(location.search);
if (params.get('url')) { $('repo-url').value = params.get('url'); $('repo-ref').value = params.get('ref') || ''; }



if (params.get('from') === 'system' && params.get('url')) {
  const back = new URL('/', location.origin);
  back.searchParams.set('url', params.get('url')); back.searchParams.set('ref', params.get('ref') || '');
  if (params.get('focus')) back.searchParams.set('focus', params.get('focus'));
  if (params.get('diagram')) back.searchParams.set('diagram', params.get('diagram'));
  $('back-to-system').href = back.pathname + back.search;
}
restoreLocation();
