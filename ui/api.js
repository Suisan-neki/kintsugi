// @ts-check
// HTTP操作と長い処理の待受を共通化する。DOMには文字だけを挿入する。
export class ApiError extends Error {
  /** @param {string} message @param {number} status */
  constructor(message, status) { super(message); this.status = status; }
}
/** @returns {string} */
function token() {
  const meta = document.querySelector('meta[name="review-token"]');
  if (!(meta instanceof HTMLMetaElement)) throw Error('操作トークンを読み取れません');
  return meta.content;
}
/** @param {Response} response @returns {Promise<any>} */
async function decode(response) {
  let data;
  try { data = await response.json(); } catch { throw new ApiError(`サーバーの応答を読み取れませんでした（HTTP ${response.status}）`, response.status); }
  if (!response.ok) throw new ApiError(data?.error || `HTTP ${response.status}`, response.status);
  return data;
}
/** @param {string} method @param {string} path @param {unknown} [body] @param {string} [operationToken] @returns {Promise<any>} */
export async function apiRequest(method, path, body, operationToken = token()) {
  const headers = { 'X-Review-Token': operationToken, ...(body !== undefined ? { 'Content-Type': 'application/json' } : {}), ...(method === 'POST' ? { Prefer: 'respond-async' } : {}) };
  const response = await fetch(path, { method, headers, body: body !== undefined ? JSON.stringify(body) : undefined });
  let data = await decode(response);
  if (response.status !== 202 || data?.kind !== 'request') return data;
  if (!/^\d+$/.test(data.id)) throw new ApiError('処理の識別子が不正です', 500);
  const jobPath = '/api/v1/jobs/' + data.id;
  const notice = typeof document === 'undefined' ? null : document.createElement('div');
  const label = typeof document === 'undefined' ? null : document.createElement('span');
  if (notice && label) {
    notice.className = 'request-job note'; notice.setAttribute('role', 'status');
    const cancel = document.createElement('button'); cancel.type = 'button'; cancel.textContent = '結果の待受を中止';
    cancel.title = '送信済みのAI処理と保存は完了する場合があります';
    cancel.addEventListener('click', async () => {
      cancel.disabled = true;
      try { await decode(await fetch(jobPath, { method: 'DELETE', headers: { 'X-Review-Token': operationToken, 'Content-Type': 'application/json' } })); }
      catch (error) { if (label) label.textContent = error instanceof Error ? error.message : String(error); }
    });
    notice.append(label, document.createTextNode(' '), cancel);
    document.getElementById('status')?.after(notice);
  }
  try {
    while (data.status === 'queued' || data.status === 'running') {
      if (label) label.textContent = data.status === 'queued' ? '処理の開始を待っています' : '読取・AI処理を実行しています';
      await new Promise(resolve => setTimeout(resolve, 250));
      data = await decode(await fetch(jobPath, { headers: { 'X-Review-Token': operationToken } }));
    }
    if (data.status === 'cancelled') throw new ApiError('結果の待受を中止しました。送信済みの処理と保存は完了する場合があります', 409);
    if (data.status !== 'completed' || typeof data.response_status !== 'number') throw new ApiError('処理状態を読み取れません', 500);
    if (data.response_status >= 400) throw new ApiError(data.result?.error || `HTTP ${data.response_status}`, data.response_status);
    return data.result;
  } finally { notice?.remove(); }
}
