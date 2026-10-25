import { mountDiagram, objectTitle, selectDiagramObject, resizeDiagram, zoomDiagram } from '/ui/repository-diagram.js';
const $ = id => document.getElementById(id);
const token = document.querySelector('meta[name="review-token"]').content;
const state = { audience: null, source: null, overview: null, graph: null, selected: null, view: null, sequence: 0, loading: false, saving: false, draftKey: null, preparation: null, dialogue: null, dialogueSequence: 0 };
const AUDIENCE = { operator: '非技術者', developer: '技術者' };
function el(tag, attrs = {}, ...children) {
  const n = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === 'onClick') n.addEventListener('click', v);
    else if (k === 'className') n.className = v;
    else n.setAttribute(k, v);
  }
  for (const c of children.flat()) if (c !== null && c !== undefined) n.append(c instanceof Node ? c : document.createTextNode(String(c)));
  return n;
}
function button(text, onClick, type = 'outline') { return el('button', { type: 'button', className: 'dads-button', 'data-type': type, 'data-size': 'sm', onClick }, text); }
function remembered(key) { try { return sessionStorage.getItem(key); } catch { return null; } }
function remember(key, value) { try { sessionStorage.setItem(key, value); } catch { /* 保存できなくても現在の画面は使う。 */ } }
function status(text) { $('status').textContent = text; }
function error(e) { $('alert').replaceChildren(el('p', { className: 'input-error' }, e.message)); }
async function api(path, body) {
  const r = await fetch('/api/v1/' + path, { method: body ? 'POST' : 'GET', headers: { 'X-Review-Token': token, ...(body ? { 'Content-Type': 'application/json' } : {}) }, ...(body ? { body: JSON.stringify(body) } : {}) });
  const data = await r.json();
  if (!r.ok) throw Error(data.error || `HTTP ${r.status}`);
  return data;
}
async function run(task, current = () => true) { $('alert').replaceChildren(); try { return await task(); } catch (e) { if (current()) error(e); return null; } }
function screen(name) {
  for (const id of ['audience', 'repository', 'system']) $(id + '-screen').hidden = id !== name;
  $('change-audience').hidden = !state.audience;
  $('change-audience').textContent = state.audience ? `立場：${AUDIENCE[state.audience]}（変更）` : '';
}
function saveDraft() { if (state.draftKey) remember(state.draftKey, $('concern-text').value); }
function draftKey() { return state.source ? `eel-concern:${state.source.repository}@${state.source.commit}:${state.selected?.id || 'whole'}` : null; }
function findObject(id) {
  const node = state.graph?.nodes.find(n => n.id === id); if (node) return { ...node, object_type: 'element' };
  const edge = state.graph?.edges.find(e => e.id === id); return edge ? { ...edge, object_type: 'relation' } : null;
}
function concernLabel(object) {
  if (!object) return 'システム全体';
  return object.object_type === 'relation' ? `${state.graph.nodes.find(n => n.id === object.from)?.title} → ${state.graph.nodes.find(n => n.id === object.to)?.title}：${object.title}` : object.title;
}
function selectNode(node) {
  saveDraft(); state.selected = node; state.draftKey = draftKey(); state.dialogue = null; $('concern-ai-answer').replaceChildren();
  $('concern-text').value = remembered(state.draftKey) || '';
  $('selected-heading').textContent = node?.object_type === 'relation' ? `${state.graph.nodes.find(n => n.id === node.from)?.title} → ${state.graph.nodes.find(n => n.id === node.to)?.title}` : objectTitle(node, false);
  $('selected-description').textContent = (state.audience === 'developer' && node?.technical_description ? node.technical_description : node?.description) || (state.audience === 'operator' && state.graph?.overview?.operator_workflow) || state.graph?.overview?.workflow || '全体への不安や疑問も、そのまま書けます。';
  $('select-whole').hidden = !node;
  selectDiagramObject($('repository-diagram'), node?.id);
  const details = $('selected-details'); details.replaceChildren();
  const technical = state.audience === 'developer';
  if (node) {
    if (node.scope_note) details.append(el('p', { className: 'note' }, node.scope_note));
    details.append(el('p', { className: 'note' }, node.basis?.note || 'コードの記述から整理した推定です。動作はまだ検証していません。'));
    if (node.object_type === 'relation') details.append(el('p', {}, `やり取り：${node.title}`));
    if (technical && (node.technology?.length || node.protocol)) details.append(el('p', { className: 'component-technology' }, `技術・接続：${node.protocol || node.technology.join(' / ')}`));
    if (technical && node.details?.length) {
      const internal = el('details', { className: 'component-internal', open: '' }, el('summary', {}, '内部の処理と起動設定'));
      const groups = new Map();
      for (const d of node.details) { if (!groups.has(d.title)) groups.set(d.title, []); groups.get(d.title).push(d); }
      for (const [title, items] of groups) internal.append(el('h3', {}, title), el('ul', {}, items.map(d => el('li', {}, d.text))));
      details.append(internal);
    }
  }
  if (state.graph) {
    const connections = node ? state.graph.edges.filter(e => e.from === node.id || e.to === node.id) : state.graph.edges;
    if (connections.length && node?.object_type !== 'relation') {
      const list = el('ul', { className: 'component-connections' });
      for (const e of connections) {
        const from = state.graph.nodes.find(n => n.id === e.from), to = state.graph.nodes.find(n => n.id === e.to);
        list.append(el('li', {}, button(`${from.title} → ${to.title}：${e.title}`, () => selectNode({ ...e, object_type: 'relation' }), 'text')));
      }
      if (node) details.append(el('h3', {}, 'この部分とのやり取り'), list);
      else details.append(el('details', {}, el('summary', {}, '図の接続を文章で読む'), list));
    }
  }
  const evidence = $('selected-evidence'); evidence.replaceChildren();
  if (node) {
    const d = el('details', {}, el('summary', {}, '説明の根拠となる設定・コード'));
    for (const ref of node.refs || []) {
      const href = `https://github.com/${state.source.repository}/blob/${state.source.commit}/${ref.path.split('/').map(encodeURIComponent).join('/')}#L${ref.line}`;
      d.append(el('p', {}, el('a', { href, target: '_blank', rel: 'noopener noreferrer' }, `${ref.path} ${ref.line}行目`), technical && ref.excerpt ? el('code', {}, ref.excerpt) : null));
    }
    evidence.append(d);
  }
  if (state.source && state.graph) {
    const url = new URL(location.href);
    if (url.searchParams.get('url') === 'https://github.com/' + state.source.repository && url.searchParams.get('ref') === state.source.commit && url.searchParams.get('diagram') === state.graph.model_revision) {
      if (node) url.searchParams.set('focus', node.id); else url.searchParams.delete('focus');
      history.replaceState(null, '', url.pathname + url.search);
    }
  }
  run(restoreDialogue);
}
$('concern-text').addEventListener('input', saveDraft);
async function renderGraph() {
  await mountDiagram($('repository-diagram'), state.graph, { technical: state.audience === 'developer', selected: state.selected?.id, onSelect: node => {
    selectNode(node);
    if ($('diagram-dialog').open) { diagramClosedBySelection = true; $('diagram-dialog').close(); }
  } });
  highlightRelated();
}
function renderOverview() {
  if (!state.graph) return;
  const overview = state.graph.overview;
  $('overview-heading').textContent = overview.title;
  $('system-purpose').textContent = state.audience === 'developer' ? overview.purpose : overview.operator_purpose || overview.purpose;
  $('system-workflow').textContent = state.audience === 'developer' ? overview.workflow : overview.operator_workflow || overview.workflow;
  $('writing-update-note').hidden = Boolean(overview.operator_workflow && state.graph.writing_policy);
}
function setAudience(audience) {
  state.audience = audience; remember('eel-audience', audience);
  // 名前や権限を推測しない。詳細な検証画面にも自己申告の立場だけを引き継ぐ。
  remember('eel-person', JSON.stringify({ name: 'ブラウザ利用者（本人確認なし）', role: audience }));
  if (state.source) { screen('system'); renderOverview(); run(renderGraph); selectNode(state.selected); renderConcerns(); }
  else {
    screen('repository');
    const params = new URLSearchParams(location.search);
    if (params.get('url')) { $('repository-url').value = params.get('url'); $('repository-ref').value = params.get('ref') || ''; openRepository(params.get('url'), params.get('ref') || '', false); }
  }
}
for (const b of document.querySelectorAll('[data-audience]')) b.addEventListener('click', () => setAudience(b.dataset.audience));
$('change-audience').addEventListener('click', () => { saveDraft(); screen('audience'); });
$('change-repository').addEventListener('click', () => { saveDraft(); state.sequence++; state.dialogueSequence++; state.dialogue = null; state.loading = false; $('load-repository').disabled = false; status(''); state.source = null; state.overview = null; state.graph = null; state.selected = null; state.draftKey = null; state.preparation = null; $('repository-url').value = ''; $('repository-ref').value = ''; document.querySelector('.repository-options').open = false; screen('repository'); history.pushState(null, '', '/'); });
$('select-whole').addEventListener('click', () => selectNode(null));
$('diagram-fit').addEventListener('click', () => resizeDiagram($('repository-diagram'), 'fit'));
$('diagram-readable').addEventListener('click', () => resizeDiagram($('repository-diagram'), 'readable'));
$('diagram-zoom-in').addEventListener('click', () => zoomDiagram($('repository-diagram'), 1.3));
$('diagram-zoom-out').addEventListener('click', () => zoomDiagram($('repository-diagram'), 1 / 1.3));
$('repository-diagram').addEventListener('diagram-scale', e => { $('diagram-scale').textContent = `${e.detail}%`; });
const mapPanel = document.querySelector('.system-map-panel');
const mapPlace = document.createComment('構成図の元の位置'); mapPanel.before(mapPlace);
let diagramClosedBySelection = false;
$('diagram-expand').addEventListener('click', () => { $('diagram-dialog-body').append(mapPanel); $('diagram-dialog').showModal(); });
$('diagram-close').addEventListener('click', () => $('diagram-dialog').close());
$('diagram-dialog').addEventListener('close', () => {
  mapPlace.after(mapPanel);
  if (diagramClosedBySelection) { $('selected-heading').focus({ preventScroll: true }); $('selected-heading').scrollIntoView({ block: 'start' }); }
  else $('diagram-expand').focus();
  diagramClosedBySelection = false;
});
$('repository-form').addEventListener('submit', e => { e.preventDefault(); openRepository($('repository-url').value.trim(), $('repository-ref').value.trim()); });
async function openRepository(url, reference, record = true) {
  if (!/^https:\/\/github\.com\/[A-Za-z0-9-]+\/[A-Za-z0-9_.-]+\/?$/.test(url)) {
    $('repository-error').hidden = false; $('repository-error').textContent = 'https://github.com/所有者/リポジトリ の形で入力してください。'; $('repository-url').setAttribute('aria-invalid', 'true'); $('repository-url').focus(); return;
  }
  const seq = ++state.sequence; state.dialogueSequence++; state.dialogue = null; state.loading = true; saveDraft();
  $('repository-error').hidden = true; $('repository-url').removeAttribute('aria-invalid'); $('load-repository').disabled = true;
  status('リポジトリを読み込み、構成図を作成しています。');
  await run(async () => {
    const read = await api('understand', { operation: 'open', url, reference });
    status('READMEとコードの抜粋から、役割と情報の流れを整理しています。');
    const params = new URLSearchParams(location.search);
    const revision = params.get('url') === url && params.get('ref') === read.source.commit ? params.get('diagram') : null;
    const [map, view] = await Promise.all([
      api('system-diagram', { repository: read.source.repository, commit: read.source.commit, ...(revision ? { revision } : {}) }), api('case')
    ]);
    if (seq !== state.sequence) return;
    state.source = read.source; state.overview = read.result; state.graph = map.result; state.view = view; state.selected = null; state.draftKey = null; state.preparation = null;
    $('system-heading').textContent = read.source.repository;
    renderOverview();
    $('repository-version').textContent = `読み込んだ版：${read.source.commit.slice(0, 12)}`;
    $('map-scope').textContent = `${map.result.scope}。実際の配置や動作は未確認です。`;
    $('diagram-summary').textContent = map.result.summary;
    $('map-basis').replaceChildren(el('p', {}, map.result.basis), el('p', {}, map.result.unknown), ...(map.result.notices || []).map(n => el('p', {}, n)), el('p', {}, `本文を読んだファイル：${map.result.read_files}件。構成に関係する候補の未読：${map.result.unread_files}件。${map.result.inventory_truncated ? 'ファイル一覧自体も途中で切れています。' : ''}`));
    const focus = params.get('url') === url ? findObject(params.get('focus')) : null;
    selectNode(focus); renderConcerns(); screen('system');
    await renderGraph();
    if (seq !== state.sequence) return;
    if (record) history.pushState(null, '', `/?url=${encodeURIComponent(url)}&ref=${encodeURIComponent(read.source.commit)}&diagram=${encodeURIComponent(map.result.model_revision)}${focus ? `&focus=${encodeURIComponent(focus.id)}` : ''}`);
    status('構成図を表示しました。気になる部分を選んでください。');
  }, () => seq === state.sequence);
  if (seq === state.sequence) { state.loading = false; $('load-repository').disabled = false; }
}
function id(prefix) { return `${prefix}-${crypto.randomUUID()}`; }
function meta(statement) { try { return JSON.parse(statement.origin); } catch { return null; } }
function concerns() {
  return (state.view?.workspace.statements || []).filter(s => {
    const m = meta(s); return m?.kind === 'repository_concern' && m.repository === state.source.repository;
  });
}
async function change(action, reason) {
  // 保存時に最新版を読む。競合を無視して古い状態で上書きしない。
  const view = await api('case');
  const next = await api('change', { expected_revision: view.workspace.revision, actor: `ブラウザ利用者（${AUDIENCE[state.audience]}・本人確認なし）`, reason, action });
  if (!state.view || next.workspace.revision >= state.view.workspace.revision) state.view = next;
  return next;
}
$('concern-form').addEventListener('submit', e => {
  e.preventDefault();
  if (state.saving || !state.source || !$('concern-text').value.trim()) return;
  const source = state.source, node = state.selected, model = state.graph, label = concernLabel(node), audience = state.audience, text = $('concern-text').value.trim(), savedDraftKey = state.draftKey;
  const ref = node?.refs?.[0];
  state.saving = true; $('save-concern').disabled = true;
  run(async () => {
    const origin = JSON.stringify({ kind: 'repository_concern', repository: source.repository, commit: source.commit, node: node?.id || null, object_type: node?.object_type || 'whole', model_id: model.model_id, model_revision: model.model_revision, label });
    await change({ op: 'add_statement', statement: { id: id('concern'), kind: 'unanswered', role: audience, text, origin, source: ref ? { repository: source.repository, commit: source.commit, path: ref.path, line: ref.line, end_line: ref.end_line || ref.line } : null, check_ids: [] } }, '構成図から不安・疑問を記録した');
    remember(savedDraftKey, '');
    if (state.source?.repository === source.repository && state.source.commit === source.commit) {
      if (state.draftKey === savedDraftKey && $('concern-text').value.trim() === text) $('concern-text').value = '';
      renderConcerns(); status('疑問を記録しました。検証したい内容を具体化できます。');
    }
  }).finally(() => { state.saving = false; $('save-concern').disabled = false; });
});
function checkFor(statement) {
  return state.view.workspace.checks.find(c => c.origin === `repository-concern:${statement.id}`);
}
function renderConcerns() {
  const root = $('concerns'); root.replaceChildren();
  const list = concerns();
  if (!list.length) { root.append(el('p', { className: 'note' }, 'まだ疑問はありません。構成図の部分を選ぶか、システム全体への疑問を書いてください。')); return; }
  for (const statement of list) {
    const m = meta(statement), check = checkFor(statement);
    const card = el('article', { className: 'concern-card', 'data-concern': statement.id }, el('p', { className: 'concern-location' }, `${m.label}${m.commit === state.source.commit ? '' : '（以前の版への疑問）'}`), el('h3', {}, statement.text));
    const target = m.commit === state.source.commit ? findObject(m.node) : null;
    if (target && (!m.model_revision || m.model_revision === state.graph.model_revision)) card.append(button('図のこの部分を見る', () => { selectNode(target); $('repository-diagram').scrollIntoView({ block: 'center' }); }));
    else if (m.model_revision) card.append(button('疑問を残した図を見る', () => run(async () => {
      await api('system-diagram', { repository: m.repository, commit: m.commit, revision: m.model_revision });
      const url = new URL('/', location.origin); url.searchParams.set('url', 'https://github.com/' + m.repository); url.searchParams.set('ref', m.commit); url.searchParams.set('diagram', m.model_revision); if (m.node) url.searchParams.set('focus', m.node);
      history.pushState(null, '', url.pathname + url.search); await openRepository('https://github.com/' + m.repository, m.commit, false);
    })));

    if (m.commit === state.source.commit && m.model_revision && m.model_revision !== state.graph.model_revision) card.append(el('p', { className: 'note' }, '疑問を残した後に、図の読み取り範囲や説明が変わっています。'));
    if (m.commit === state.source.commit && m.node && !target) card.append(el('p', { className: 'note' }, '疑問を残した部分は、現在の図では特定できていません。元の出所を保持しています。'));
    if (check) {
      const cv = state.view.checks.find(c => c.check_id === check.id);
      card.append(el('p', {}, `確かめたい動作：${check.text}`));
      card.append(el('p', { className: 'note' }, cv?.evidence.length ? `検証結果：${({ satisfied: '条件を満たす', not_satisfied: '条件を満たさない', unknown: '検証未通過（証拠不足・判定不能）' })[cv.verdict]}。${cv.reason}` : '検証未通過。検証内容を記録しましたが、まだ実行していません。'));
      card.append(workbenchLink('検証の設定へ進む', check, m, 'trial'));
      if (cv?.evidence.length) card.append(workbenchLink('結果を見て改善を検討する', check, m, 'result'));
    } else {
      card.append(button('検証したい内容を決める', () => prepareConcern(card, statement), 'solid-fill'));
    }
    root.append(card);
    if (!check && state.preparation?.statementId === statement.id) prepareConcern(card, statement);
  }
}
function prepareConcern(card, statement) {
  if (card.querySelector('form')) return;
  if (state.preparation?.statementId !== statement.id) state.preparation = { statementId: statement.id, text: '' };
  const label = el('label', { for: `expect-${statement.id}`, className: 'system-label' }, 'どんな動作を確かめたいですか');
  const text = el('textarea', { id: `expect-${statement.id}`, rows: 3, className: 'system-input', required: '', maxlength: 2000 });
  text.value = state.preparation.text; text.addEventListener('input', () => { state.preparation = { statementId: statement.id, text: text.value }; });
  const form = el('form', {}, label, text, el('p', { className: 'note' }, '期待する動作を具体的に書いてください。起動方法や確認方法は、次の画面で技術者が設定します。'));
  const submit = el('button', { type: 'submit', className: 'dads-button', 'data-type': 'solid-fill', 'data-size': 'sm' }, '検証内容を記録する');
  form.append(submit);
  form.addEventListener('submit', e => {
    e.preventDefault(); if (!text.value.trim() || submit.disabled) return; submit.disabled = true;
    const m = meta(statement);
    run(async () => {
      // 二重保存や再読込後の再操作では既存の条件を再利用する。
      state.view = await api('case');
      if (!checkFor(statement)) {
        const slug = m.repository.replace(/[^A-Za-z0-9]+/g, '-').replace(/^-|-$/g, '').slice(0, 80);
        const prefix = `${slug}-`, checks = state.view.workspace.checks;
        let n = 1; while (checks.some(c => c.id === prefix + n)) n++;
        const source = statement.source;
        await change({ op: 'add_check', check: { id: prefix + n, version: 1, text: text.value.trim(), origin: `repository-concern:${statement.id}`, source: source || null, observation: null, rule: 'manual_only', dependencies: [], questions: [] } }, `疑問「${statement.text}」から検証したい動作を記録した`);
      }
      state.preparation = null; renderConcerns(); status('検証内容を記録しました。設定へ進めます。');
    }).finally(() => { submit.disabled = false; });
  });
  card.append(form); text.focus();
}
function workbenchLink(label, check, m, step) {
  const href = `/workbench?url=${encodeURIComponent('https://github.com/' + m.repository)}&ref=${encodeURIComponent(check.source?.commit || m.commit)}&system_ref=${encodeURIComponent(m.commit)}&check=${encodeURIComponent(check.id)}${m.node ? `&focus=${encodeURIComponent(m.node)}` : ''}${m.model_revision ? `&diagram=${encodeURIComponent(m.model_revision)}` : ''}&from=system#${step}`;
  return el('a', { href, className: 'dads-button', 'data-type': 'outline', 'data-size': 'sm' }, label);
}
async function restore() {
  const audience = remembered('eel-audience'); state.audience = AUDIENCE[audience] ? audience : null;
  const params = new URLSearchParams(location.search);
  if (!state.audience) { screen('audience'); return; }
  screen('repository');
  if (params.get('url')) { $('repository-url').value = params.get('url'); $('repository-ref').value = params.get('ref') || ''; await openRepository(params.get('url'), params.get('ref') || '', false); }
}
window.addEventListener('popstate', () => { saveDraft(); state.sequence++; state.dialogueSequence++; state.dialogue = null; state.loading = false; $('load-repository').disabled = false; restore(); });
restore();

async function loadAvailability() {
  const info = await api('ai');
  $('ai-availability').hidden = info.status === 'configured';
  $('ai-availability').textContent = info.status === 'configured' ? '' : '現在、構成図の生成を利用できません。運営者の設定が完了するまでお待ちください。保存済みの図は開けます。';
}
$('regenerate-diagram').addEventListener('click', () => {
  if (state.loading || !state.source) return;
  const source = state.source, seq = state.sequence, submit = $('regenerate-diagram'); submit.disabled = true;
  status('同じコミットを読み直し、構成図を作り直しています。');
  run(async () => {
    const map = await api('system-diagram', { repository: source.repository, commit: source.commit, refresh: true });
    if (seq !== state.sequence) return;
    const url = new URL(location.href); url.searchParams.set('diagram', map.result.model_revision); url.searchParams.delete('focus');
    history.replaceState(null, '', url.pathname + url.search);
    await openRepository('https://github.com/' + source.repository, source.commit, false);
  }, () => seq === state.sequence).finally(() => { submit.disabled = false; });
});
function dialogueBinding() {
  return { repository: state.source.repository, commit: state.source.commit, revision: state.graph.model_revision, object_id: state.selected?.id || null };
}
function highlightRelated() {
  const related = state.dialogue?.record?.related_object_ids || [];
  for (const node of $('repository-diagram').querySelectorAll('[data-object]')) node.setAttribute('data-discussion', String(related.includes(node.dataset.object)));
}
async function restoreDialogue() {
  if (!state.source || !state.graph) return;
  const seq = ++state.dialogueSequence, binding = dialogueBinding();
  $('ask-concern-ai').disabled = true;
  try {
    const dialogue = await api('system-diagram/dialogue/read', binding);
    if (seq !== state.dialogueSequence) return;
    state.dialogue = dialogue; renderDialogue();
  } finally { if (seq === state.dialogueSequence) $('ask-concern-ai').disabled = false; }
}
function renderDialogue() {
  const root = $('concern-ai-answer'), dialogue = state.dialogue; root.replaceChildren(); highlightRelated();
  if (!dialogue?.history?.length) return;
  const history = el('details', { className: 'dialogue-history' }, el('summary', {}, `この部分についての会話（${dialogue.turn}往復）`));
  for (const turn of dialogue.history) history.append(el('p', { className: 'dialogue-user' }, `あなた：${turn.question}`), el('p', { className: 'dialogue-reply' }, turn.answer));
  root.append(history, el('h3', {}, 'AIからの返答'), el('p', { className: 'dialogue-reply' }, dialogue.answer));
  const sourceRead = dialogue.history.at(-1).source_read;
  if (sourceRead?.loaded_files?.length) root.append(el('details', {}, el('summary', {}, '今回追加で読んだファイル'), el('ul', {}, sourceRead.loaded_files.map(path => el('li', {}, path)))));
  if (sourceRead?.failed_files?.length) root.append(el('p', { className: 'ref-unverified' }, `追加で読めなかったファイル：${sourceRead.failed_files.map(f => f.path).join('、')}。この部分は未確認です。`));
  const record = dialogue.record;
  const summary = el('section', { className: 'dialogue-record' }, el('h3', {}, 'ここまでに整理したこと'), el('p', { className: 'note' }, '会話から整理した案です。期待する動作や検証内容を確認してから記録できます。'));
  const details = el('dl', {});
  for (const [label, value] of [['気になっていること', record.concern], ['操作する人', record.actor], ['操作・場面', record.operation], ['期待する動作', record.expectation]]) details.append(el('dt', {}, label), el('dd', {}, value || 'まだ分かっていません'));
  summary.append(details);
  const names = record.related_object_ids.map(id => concernLabel(findObject(id)));
  if (names.length) summary.append(el('p', { className: 'note' }, `この相談に関係する部分：${names.join('、')}。図では枠線で示しています。`));
  if (dialogue.claims?.length) {
    const basis = el('details', {}, el('summary', {}, 'コードの記述と根拠'));
    for (const claim of dialogue.claims) {
      basis.append(el('p', {}, claim.text));
      for (const ref of claim.refs) if (ref.in_context && ref.verified) basis.append(el('p', {}, el('a', { href: ref.url, target: '_blank', rel: 'noopener noreferrer' }, `${ref.path} ${ref.line}〜${ref.end_line}行目`)));
    }
    summary.append(basis);
  }
  for (const [label, values] of [['まだ分からないこと', record.unknowns], ['あなたが報告した結果', record.reported_observations], ['未実施の改善案', record.improvement_ideas]]) if (values.length) summary.append(el('h4', {}, label), el('ul', {}, values.map(v => el('li', {}, v))));
  if (record.test_plan.length) summary.append(el('h4', {}, 'まだ実行していない検証案'), el('ol', {}, record.test_plan.map(step => el('li', {}, el('p', {}, step.operation), el('p', {}, `期待：${step.expected}`), el('p', {}, `記録するもの：${step.observe}`)))));
  root.append(summary);
  if (record.expectation) {
    const linked = [...state.view.workspace.statements].reverse().find(s => meta(s)?.dialogue_id === dialogue.thread_id && checkFor(s));
    const existing = linked ? checkFor(linked) : null;
    if (existing) {
      const result = state.view.checks.find(c => c.check_id === existing.id);
      root.append(el('p', {}, `記録している検証内容：${existing.text}`));
      if (result?.evidence.length) root.append(workbenchLink('検証結果を確認する', existing, meta(linked), 'result'));
    }
    const form = el('form', { className: 'dialogue-handoff' });
    const inputId = `dialogue-expect-${dialogue.thread_id.slice(0, 12)}`;
    const input = el('textarea', { id: inputId, className: 'system-input', rows: 3, maxlength: 2000, required: '' });
    const earlierProposal = linked && dialogue.history[meta(linked).dialogue_turn - 1]?.record.expectation;
    input.value = existing && record.expectation === earlierProposal ? existing.text : record.expectation;
    const submit = el('button', { type: 'submit', className: 'dads-button', 'data-type': 'solid-fill', 'data-size': 'md' }, existing ? 'この内容で再検証する' : 'この内容を検証する');
    form.append(el('label', { className: 'system-label', for: inputId }, '検証する動作'), input, el('p', { className: 'note' }, '内容は編集できます。未確認事項も一緒に残し、起動方法と観測方法の設定へ進みます。'), submit);
    form.addEventListener('submit', e => { e.preventDefault(); if (!input.value.trim() || submit.disabled) return; submit.disabled = true; run(() => handoffDialogue(dialogue, input.value.trim())).finally(() => { submit.disabled = false; }); });
    root.append(form);
  }
}
async function handoffDialogue(dialogue, expectation) {
  const binding = dialogue.binding, record = dialogue.record, statementId = `dialogue-${dialogue.thread_id.slice(0, 24)}-${dialogue.turn}`;
  const label = concernLabel(findObject(binding.object_id));
  const origin = { kind: 'repository_concern', repository: binding.repository, commit: binding.commit, node: binding.object_id, model_revision: binding.revision, label, dialogue_id: dialogue.thread_id, dialogue_turn: dialogue.turn };
  const ref = dialogue.claims?.flatMap(c => c.refs).find(r => r.verified && r.in_context);
  const source = ref ? { repository: binding.repository, commit: binding.commit, path: ref.path, line: ref.line, end_line: ref.end_line } : null;
  state.view = await api('case');
  const previousStatement = [...state.view.workspace.statements].reverse().find(s => meta(s)?.dialogue_id === dialogue.thread_id && checkFor(s));
  const previousCheck = previousStatement ? checkFor(previousStatement) : null;
  if (previousCheck?.text === expectation) {
    location.assign(workbenchLink('検証の設定へ進む', previousCheck, meta(previousStatement), 'trial').href); return;
  }
  let statement = state.view.workspace.statements.find(s => s.id === statementId);
  if (!statement) {
    await change({ op: 'add_statement', statement: { id: statementId, kind: 'unanswered', role: state.audience, text: record.concern || dialogue.history[0].question, origin: JSON.stringify(origin), source, check_ids: [] } }, '構成図についての会話から疑問を記録した');
    statement = state.view.workspace.statements.find(s => s.id === statementId);
  }
  let check = checkFor(statement);
  if (!check || check.text !== expectation) {
    if (previousCheck) {
      await change({ op: 'revise_check', check_id: previousCheck.id, text: expectation, origin: `repository-concern:${statementId}`, rule: 'manual_only', dependencies: previousCheck.dependencies }, '会話で整理し直した確認項目を利用者が確認し、検証内容の版を更新した');
      const updated = state.view.workspace.checks.find(c => c.id === previousCheck.id);
      for (const text of record.unknowns) if (!updated.questions.some(q => q.text === text)) await change({ op: 'add_question', check_id: previousCheck.id, question: { id: id('dialogue-unknown'), text, answer: null, origin: null } }, '整理し直した検証内容へ未確認事項を残した');
    } else {
      await change({ op: 'add_check', check: { id: `${statementId}-check`, version: 1, text: expectation, origin: `repository-concern:${statementId}`, source, observation: null, rule: 'manual_only', dependencies: [], questions: record.unknowns.map((text, n) => ({ id: `unknown-${n + 1}`, text, answer: null, origin: null })) } }, '会話で整理した期待を利用者が確認し、未確認事項とともに検証内容として記録した');
    }
    check = checkFor(statement);
  }
  if (!check) throw Error('検証内容を確認できませんでした。');
  location.assign(workbenchLink('検証の設定へ進む', check, origin, 'trial').href);
}
$('ask-concern-ai').addEventListener('click', () => {
  const question = $('concern-text').value.trim();
  if (!question) { status('不安や疑問を入力してください。'); $('concern-text').focus(); return; }
  const submit = $('ask-concern-ai'); if (submit.disabled || !state.source) return; submit.disabled = true;
  const seq = ++state.dialogueSequence, binding = dialogueBinding(), turn = state.dialogue?.turn || 0;
  status('図とコード、これまでの会話から疑問を整理しています。');
  run(async () => {
    const answer = await api('system-diagram/dialogue', { binding, audience: state.audience, expected_turn: turn, question });
    if (seq !== state.dialogueSequence) return;
    state.dialogue = answer;
    if ($('concern-text').value.trim() === question) { $('concern-text').value = ''; saveDraft(); }
    renderDialogue(); status('会話と整理案を保存しました。続けて返答するか、検証したい動作を確認できます。');
  }, () => seq === state.dialogueSequence).finally(() => { if (seq === state.dialogueSequence) submit.disabled = false; });
});
run(loadAvailability);
