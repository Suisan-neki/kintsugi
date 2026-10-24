// 人が操作した内容は自己申告として保存する。リンクを開いただけで試用済みにはしない。
const $ = id => document.getElementById(id);
const token = document.querySelector('meta[name="review-token"]').content;
let view, busy = false, displacedDraft = null;
let sessionId = new URL(location.href).searchParams.get('session');
const outcomes = { completed: '操作を終えた', blocked: '途中で進めなくなった', not_tried: '試せなかった' };
function h(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs)) {
    if (key.startsWith('on')) node.addEventListener(key.slice(2), value);
    else if (key === 'class') node.className = value;
    else if (key === 'value') node.value = value;
    else if (value !== false && value != null) node.setAttribute(key, value === true ? '' : value);
  }
  node.append(...children.flat().filter(x => x != null));
  return node;
}
const button = (text, onclick) => h('button', { type: 'button', class: 'dads-button', 'data-type': 'outline', 'data-size': 'sm', onclick }, text);
const field = (label, id, control, help) => {
  if (help) control.setAttribute('aria-describedby', id + '-help');
  return h('div', {}, h('label', { class: 'system-label', for: id }, label), control, help ? h('p', { id: id + '-help', class: 'note' }, help) : null);
};
const text = (id, attrs = {}) => h('input', { id, class: 'system-input', required: true, maxlength: 200, ...attrs });
const area = (id, attrs = {}) => h('textarea', { id, class: 'system-input', required: true, rows: 3, maxlength: 4000, ...attrs });
const detail = (title, ...body) => h('details', {}, h('summary', {}, title), ...body);
const read = id => $(id).value.trim();
async function api(method, path, body) {
  const response = await fetch('/api/v1/' + path, { method, headers: { 'X-Review-Token': token, ...(body ? { 'Content-Type': 'application/json' } : {}) }, body: body ? JSON.stringify(body) : undefined });
  const result = await response.json();
  if (!response.ok) {
    const error = new Error(result.error || `HTTP ${response.status}`);
    error.status = response.status;
    throw error;
  }
  return result;
}
async function guarded(task) {
  if (busy) return;
  busy = true;
  $('main').setAttribute('aria-busy', 'true');
  $('status').textContent = '';
  $('alert').replaceChildren();
  try { await task(); } catch (error) {
    const alert = h('div', { class: 'box', tabindex: '-1' }, h('p', {}, error.message));
    if (error.status === 409) alert.append(h('p', {}, 'ほかの操作で記録が更新されています。入力を控えてから、最新の記録を読み直してください。'));
    $('alert').replaceChildren(alert);
    alert.focus();
  } finally { busy = false; $('main').removeAttribute('aria-busy'); }
}
async function change(action, actor, reason) {
  view = await api('POST', 'change', { expected_revision: view.workspace.revision, actor, reason, action });
}
function selectSession(id) {
  sessionId = id;
  const url = new URL(location.href);
  for (const key of ['check', 'before', 'after', 'revision']) url.searchParams.delete(key);
  url.searchParams.set('session', id);
  history.replaceState(null, '', url);
  render();
  $('session-heading').focus();
}
function planDetails(plan) {
  return h('div', {}, h('h3', {}, plan.concern), h('p', {}, plan.task), h('p', {}, '試す条件：', plan.conditions));
}
function planCurrent(plan) {
  const entries = value => JSON.stringify(Object.entries(value || {}).sort(([a], [b]) => a.localeCompare(b)));
  return entries(plan.check_versions) === entries(Object.fromEntries(view.workspace.checks.map(check => [check.id, check.version])))
    && entries(plan.context) === entries(view.workspace.context);
}
function evidenceLink(evidence, variant) {
  const url = new URL('/workbench', location.origin);
  url.searchParams.set('url', 'https://github.com/' + variant.source.repository);
  url.searchParams.set('ref', variant.source.commit);
  url.searchParams.set('check', evidence.check_id);
  url.searchParams.set('evidence', evidence.id);
  url.hash = 'result';
  return h('li', {}, h('a', { href: url.pathname + url.search + url.hash, target: '_blank', rel: 'noopener noreferrer' }, `検証記録 ${evidence.id} を見る`));
}
function variantDetails(variant) {
  const evidence = variant.evidence_ids.map(id => view.workspace.evidence.find(entry => entry.id === id)).filter(Boolean);
  return detail('対象の版と技術者の検証記録',
    h('p', {}, `${variant.source.repository} @ ${variant.source.commit}`),
    h('p', {}, `${variant.source.path} ${variant.source.line}〜${variant.source.end_line}行`),
    h('p', {}, variant.evidence_ids.length ? `対応する検証記録：${variant.evidence_ids.join('、')}` : '技術者の検証記録は対応づけられていません。'),
    evidence.length ? h('ul', {}, ...evidence.map(entry => evidenceLink(entry, variant))) : null,
    h('p', { class: 'note' }, '登録した版とURLの対応は技術者による申告です。利用者の感想は技術検証の合否や採用の合意とは別に残します。'));
}
function renderPlans() {
  const plans = view.workspace.user_trial_plans || [];
  $('plans').replaceChildren(...(plans.length ? plans.map(plan => {
    if (!planCurrent(plan)) return h('article', { class: 'box' }, planDetails(plan), h('p', { class: 'note', role: 'status' }, 'この計画の作成後に、検証する条件が変わっています。新たに試用を始める場合は、技術者が現在の条件で計画を作り直してください。'));
    const input = text('participant-' + plan.id, { placeholder: '例：参加者01', autocomplete: 'off' });
    const order = h('select', { id: 'order-' + plan.id, class: 'system-input' }, h('option', { value: 'a,b' }, 'A → B'), h('option', { value: 'b,a' }, 'B → A'));
    const form = h('form', { onsubmit: event => { event.preventDefault(); guarded(async () => {
      await change({ op: 'start_user_trial', plan_id: plan.id, participant: input.value.trim(), order: order.value.split(',') }, input.value.trim(), '変更前後を試す記録を開始した');
      selectSession(view.workspace.user_trial_sessions.at(-1).id);
      $('status').textContent = '試用記録を開始しました。案内された順番で操作してください。';
    }); } }, field('参加者の識別名', input.id, input, '氏名でなく、案内された識別名を使えます。'), field('試す順番', order.id, order),
    h('button', { type: 'submit', class: 'dads-button', 'data-type': 'solid-fill', 'data-size': 'md' }, 'この内容を試す'));
    return h('article', { class: 'box' }, planDetails(plan), form);
  }) : [h('p', {}, '試用計画がまだありません。技術者が準備してから、この画面で試せます。')]));
}
function renderSession() {
  const session = (view.workspace.user_trial_sessions || []).find(s => s.id === sessionId);
  $('active-session').hidden = !session;
  $('trial-list').hidden = Boolean(session);
  $('choose-plan').hidden = !session;
  if (!session) return;
  const plan = view.workspace.user_trial_plans.find(p => p.id === session.plan_id);
  const next = session.order[session.feedback.length];
  const records = session.feedback.map(entry => h('article', { class: 'box' }, h('h3', {}, entry.variant.toUpperCase() + 'の記録'),
    h('p', {}, outcomes[entry.outcome]), h('p', {}, '実際の操作：', entry.operation), h('p', {}, '感想・理由：', entry.feedback),
    h('p', { class: 'note' }, '本人の申告として保存した記録です。')));
  const content = [h('p', {}, `参加者：${session.participant}　順番：${session.order.map(x => x.toUpperCase()).join(' → ')}`), planDetails(plan), ...records];
  if (!planCurrent(plan)) content.splice(2, 0, h('p', { class: 'note', role: 'status' }, 'この試用を始めた後に、現在の検証条件が変わっています。ここで保存する感想は、上に表示した試用計画と版への記録として残ります。現在の条件で試した結果には置き換えません。'));
  if (next) {
    const variant = plan.variants.find(v => v.label === next);
    const operation = area('operation'), feedback = area('feedback');
    const outcome = h('select', { id: 'outcome', class: 'system-input', required: true }, h('option', { value: '' }, '選んでください'), ...Object.entries(outcomes).map(([value, label]) => h('option', { value }, label)));
    const form = h('form', { id: 'feedback-form', 'data-session': session.id, 'data-variant': next, onsubmit: event => { event.preventDefault(); guarded(async () => {
      await change({ op: 'record_user_trial_feedback', session_id: session.id, variant: next, outcome: outcome.value, operation: operation.value.trim(), feedback: feedback.value.trim() }, session.participant, '試用した操作と感想を本人の申告として記録した');
      render();
      $('status').textContent = `${next.toUpperCase()}の感想を保存しました。`;
      $('session-heading').focus();
    }); } }, field('実際に行った操作', operation.id, operation, '試せなかった場合は、試そうとした操作を書いてください。'), field('操作の結果', outcome.id, outcome),
    field('気づいたこと・困ったこと・試せなかった理由', feedback.id, feedback),
    h('button', { type: 'submit', class: 'dads-button', 'data-type': 'solid-fill', 'data-size': 'md' }, `${next.toUpperCase()}の感想を保存する`));
    content.push(h('article', { class: 'box' }, h('h3', {}, `次は${next.toUpperCase()}を試す`),
      h('p', {}, '別のタブでアプリを開き、上の操作を試したあと、この画面に戻ってください。'),
      h('p', {}, h('a', { href: variant.url, target: '_blank', rel: 'noopener noreferrer' }, `${next.toUpperCase()}のアプリを開く`)),
      variantDetails(variant), form));
  } else content.push(h('p', { role: 'status' }, 'A・Bの記録を保存しました。ご協力ありがとうございました。'));
  $('session-body').replaceChildren(...content);
}
function renderDisplacedDraft() {
  $('unsaved-draft').replaceChildren(...(displacedDraft ? [h('div', { class: 'box', role: 'status' },
    h('p', {}, `${displacedDraft.variant.toUpperCase()}の未保存の入力を残しています。ほかの操作で試用記録が進んだため、次の版の入力欄には移していません。`),
    detail('未保存の内容を見る', h('p', {}, `試用記録：${displacedDraft.session}`),
      h('p', {}, `操作：${displacedDraft.operation}`), h('p', {}, `結果：${outcomes[displacedDraft.outcome] || '未選択'}`), h('p', {}, `感想：${displacedDraft.feedback}`)),
    button('この未保存の入力を閉じる', () => { displacedDraft = null; renderDisplacedDraft(); }))] : []));
}
function render() {
  renderDisplacedDraft();
  renderPlans();
  renderSession();
  const sessions = view.workspace.user_trial_sessions || [];
  $('sessions').replaceChildren(...(sessions.length ? sessions.map(session => h('article', { class: 'box' },
    h('p', {}, `${session.participant}：${session.order.map(x => x.toUpperCase()).join(' → ')}（${session.feedback.length}/2件記録）`),
    button(session.feedback.length === 2 ? '記録を見る' : '続きを記録する', () => selectSession(session.id)))) : [h('p', {}, 'まだ試用記録はありません。')]));
}
for (const label of ['a', 'b']) {
  $('variant-fields').append(h('fieldset', {}, h('legend', {}, label.toUpperCase() + 'の版と起動先'),
    field('OSSリポジトリ（所有者/名前）', label + '-repository', text(label + '-repository')),
    field('固定コミット（40桁）', label + '-commit', text(label + '-commit', { pattern: '[a-f0-9]{40}', maxlength: 40 })),
    field('対応するコードのパス', label + '-path', text(label + '-path', { maxlength: 1024 })),
    field('参照の開始行', label + '-line', text(label + '-line', { type: 'number', min: 1, max: 1000000, value: 1 })),
    field('参照の終了行', label + '-end-line', text(label + '-end-line', { type: 'number', min: 1, max: 1000000, value: 1 })),
    field('試用環境のURL', label + '-url', text(label + '-url', { type: 'url', maxlength: 2048 }), 'localhostまたはループバックのURL。起動と終了は技術者が行います。'),
    field('検証記録のID（任意）', label + '-evidence', text(label + '-evidence', { required: false, maxlength: 2000 }), '複数ある場合はカンマで区切ります。')));
}
$('plan-form').addEventListener('submit', event => {
  event.preventDefault();
  guarded(async () => {
    const variants = ['a', 'b'].map(label => ({ label, source: { repository: read(label + '-repository'), commit: read(label + '-commit'), path: read(label + '-path'), line: Number(read(label + '-line')), end_line: Number(read(label + '-end-line')) }, url: read(label + '-url'), evidence_ids: read(label + '-evidence').split(',').map(x => x.trim()).filter(Boolean) }));
    await change({ op: 'add_user_trial_plan', plan: { id: 'web-' + crypto.randomUUID(), concern: read('concern'), task: read('task'), conditions: read('conditions'), variants } }, read('preparer'), '固定版と条件を対応づけたWeb試用計画を登録した');
    $('plan-form').reset();
    $('handoff-notice').hidden = true;
    const url = new URL(location.href);
    for (const key of ['check', 'before', 'after', 'revision']) url.searchParams.delete(key);
    history.replaceState(null, '', url);
    $('prepare').open = false;
    render();
    $('status').textContent = '試用計画を登録しました。利用者が試す内容を選べます。';
  });
});
async function reload() {
  // 下書きは同じ試用記録・同じ版だけに戻す。別タブで次へ進んだ場合は分けて保持する。
  const feedbackForm = $('feedback-form');
  const draft = feedbackForm ? {
    session: feedbackForm.dataset.session, variant: feedbackForm.dataset.variant,
    operation: read('operation'), feedback: read('feedback'), outcome: read('outcome'),
  } : null;
  const drafts = [...document.querySelectorAll('input[id], textarea[id], select[id]')]
    .filter(el => !['operation', 'feedback', 'outcome'].includes(el.id)).map(el => [el.id, el.value]);
  view = await api('GET', 'case');
  render();
  for (const [id, value] of drafts) if ($(id)) $(id).value = value;
  if (draft) {
    const current = $('feedback-form');
    if (current?.dataset.session === draft.session && current?.dataset.variant === draft.variant) {
      for (const id of ['operation', 'feedback', 'outcome']) $(id).value = draft[id];
    } else if (draft.operation || draft.feedback || draft.outcome) {
      displacedDraft = draft;
      renderDisplacedDraft();
    }
  }
}
$('choose-plan').addEventListener('click', () => {
  sessionId = null;
  const url = new URL(location.href);
  url.searchParams.delete('session');
  history.replaceState(null, '', url);
  render();
});
$('reload').addEventListener('click', () => guarded(async () => { await reload(); $('status').textContent = '最新の記録を読み込みました。'; }));
async function prepareFromComparison() {
  const params = new URLSearchParams(location.search);
  const keys = ['check', 'before', 'after', 'revision'];
  if (!keys.some(key => params.has(key))) return;
  const reject = () => { throw Error('比較した記録を引き継げません。技術者の検証画面で、最新の二つの試行を選び直してください。'); };
  if (params.has('session') || keys.some(key => params.getAll(key).length !== 1 || !params.get(key))) reject();
  const revision = params.get('revision');
  if (!/^[1-9][0-9]*$/.test(revision) || !Number.isSafeInteger(Number(revision)) || Number(revision) !== view.workspace.revision) reject();
  const check = view.workspace.checks.find(check => check.id === params.get('check'));
  if (!check) reject();
  const comparison = await api('POST', 'compare', { expected_revision: Number(revision), check_id: check.id, before_id: params.get('before'), after_id: params.get('after') });
  const trials = [comparison.before, comparison.after];
  const sources = trials.map(trial => trial.source);
  if (sources.some(source => !source || !/^[a-f0-9]{40}$/.test(source.commit)) || sources[0].commit === sources[1].commit) reject();
  if (trials.some((trial, index) => trial.execution_source && (trial.execution_source.repository !== sources[index].repository || trial.execution_source.commit !== sources[index].commit))) reject();
  // 全項目の対応を確認してから反映する。入力の下書きだけを作り、事例へは保存しない。
  $('concern').value = check.text;
  for (const [index, label] of ['a', 'b'].entries()) {
    const source = sources[index];
    for (const key of ['repository', 'commit', 'path', 'line']) $(label + '-' + key).value = source[key];
    $(label + '-end-line').value = source.end_line;
    $(label + '-evidence').value = trials[index].evidence.id;
  }
  $('handoff-notice').textContent = '比較した版と検証記録を下書きに引き継ぎました。試用する操作・条件・記入者と、手動で起動した試用環境のURLを記入してください。まだ登録していません。';
  $('handoff-notice').hidden = false;
  $('prepare').open = true;
  $('task').focus();
}
await guarded(async () => { await reload(); await prepareFromComparison(); });
