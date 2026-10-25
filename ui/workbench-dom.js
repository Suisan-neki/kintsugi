// 画面部品。取得した文字列をHTMLとして解釈しない。
export function h(tag, attrs = {}, ...children) {
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
export function button(text, onclick, type = 'outline', size = 'sm', attrs = {}) {
  return h('button', { class: 'dads-button', 'data-type': type, 'data-size': size, type: 'button', onclick, ...attrs }, text);
}
let uid = 0;
const nextId = prefix => `${prefix}-${++uid}`;
export function field({ label, id = nextId('f'), support, required, control }) {
  const supportId = support ? `${id}-support` : null;
  control.id = id;
  if (supportId) control.setAttribute('aria-describedby', supportId);
  if (required) control.required = true;
  return h('div', { class: 'dads-form-control-label', 'data-size': 'md' },
    h('label', { class: 'dads-form-control-label__label', for: id }, label, required ? h('span', { class: 'dads-form-control-label__requirement', 'data-required': 'true' }, '※必須') : null),
    support ? h('p', { id: supportId, class: 'dads-form-control-label__support-text' }, support) : null,
    h('div', {}, control));
}
export function input(attrs = {}) { const i = h('input', { class: 'dads-input-text__input', type: 'text', 'data-size': 'md', ...attrs }); const wrap = h('span', { class: 'dads-input-text' }, i); wrap.input = i; return wrapField(wrap, i); }
export function textarea(attrs = {}) { const t = h('textarea', { class: 'dads-textarea__textarea', rows: 3, ...attrs }); return wrapField(h('span', { class: 'dads-textarea' }, t), t); }
export function select(options, attrs = {}) {
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
export function fill(el, ...children) { el.replaceChildren(...children.flat(Infinity).filter(c => c !== null && c !== undefined && c !== false)); }
export function disclosure(summary, ...content) {
  return h('details', { class: 'dads-disclosure' }, h('summary', { class: 'dads-disclosure__summary' }, summary), h('div', { class: 'dads-disclosure__content' }, content));
}
