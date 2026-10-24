// Prepare a request from existing evidence. This tool never starts an app or sends the request.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const maximum = 4 * 1024 * 1024;
function check(condition, message) {
  if (!condition) throw new Error(message);
}
function text(value, name, limit = 16384) {
  check(typeof value === 'string' && value.trim() && Buffer.byteLength(value) <= limit, `${name}を空でない${limit}バイト以内の文字列で指定してください`);
  return value;
}
function identifier(value) {
  check(typeof value === 'string' && /^[A-Za-z0-9_-]{1,128}$/.test(value), '識別子は英数字・ハイフン・下線の1〜128文字です');
  return value;
}
function keys(value, allowed, name) {
  check(value && typeof value === 'object' && !Array.isArray(value), `${name}はJSONオブジェクトで指定してください`);
  check(Object.keys(value).every(key => allowed.includes(key)), `${name}に未対応の項目があります。認証情報は入力しないでください`);
}
function source(value) {
  check(value && /^[A-Za-z0-9-]+\/[A-Za-z0-9_.-]+$/.test(value.repository)
    && /^[a-f0-9]{40}$/.test(value.commit), '検証記録にはOpenEMRまたはそのforkの固定コミットが必要です');
  check(typeof value.path === 'string' && value.path.length <= 1024
    && !/[\\\x00-\x1f\x7f]/.test(value.path)
    && value.path.split('/').every(part => part && part !== '.' && part !== '..')
    && Number.isInteger(value.line) && value.line >= 1
    && Number.isInteger(value.end_line) && value.end_line >= value.line && value.end_line <= 1000000,
  '検証記録のコード参照を確認してください');
  return { repository: value.repository, commit: value.commit, path: value.path, line: value.line, end_line: value.end_line };
}
const sameRevision = (left, right) => left?.repository === right?.repository && left?.commit === right?.commit;
function localUrl(value) {
  text(value, '試用先URL', 2048);
  const url = new URL(value);
  const host = url.hostname;
  check(['http:', 'https:'].includes(url.protocol)
    && (host === 'localhost' || host === '[::1]' || /^127(?:\.\d{1,3}){3}$/.test(host))
    && !url.username && !url.password && !value.includes('?') && !value.includes('#'),
  '試用先は認証情報・クエリー・フラグメントを含まないループバックのHTTP(S) URLにしてください');
  return url.href;
}
function readJson(file) {
  check(fs.statSync(file).size <= maximum, '入力ファイルが4MiBを超えています');
  return JSON.parse(fs.readFileSync(file, 'utf8'));
}
export function loadWorkspace(location) {
  if (!fs.statSync(location).isDirectory()) {
    const value = readJson(location);
    return value.workspace || value;
  }
  check(!fs.existsSync(path.join(location, 'restore.pending')), '復元が完了していません');
  const revisions = path.join(location, 'revisions');
  const files = fs.readdirSync(revisions).filter(file => /^\d{20}\.json$/.test(file)).sort();
  check(files.length, '事例の保存済みの版がありません');
  const name = files.at(-1);
  const workspace = readJson(path.join(revisions, name));
  check(BigInt(workspace.revision) === BigInt(name.slice(0, 20)), '事例の版とファイル名が一致しません');
  return workspace;
}

export function buildRequest(input, workspace, environment = null) {
  keys(input, ['id', 'actor', 'reason', 'concern', 'task', 'conditions', 'variants'], '試用計画');
  check(Number.isSafeInteger(workspace?.revision) && workspace.revision >= 1 && Array.isArray(workspace.evidence), '事例の版と検証記録が必要です');
  identifier(input.id);
  for (const key of ['actor', 'reason', 'concern', 'task', 'conditions']) text(input[key], key);
  check(!(workspace.user_trial_plans || []).some(plan => plan.id === input.id), '同じ識別子の試用計画が既にあります');
  check(Array.isArray(input.variants) && input.variants.length === 2, 'A・Bの2件を指定してください');
  const selected = new Map();
  const variants = ['a', 'b'].map(label => {
    const values = input.variants.filter(variant => variant.label === label);
    check(values.length === 1, 'a・bをそれぞれ1件ずつ指定してください');
    const value = values[0];
    keys(value, ['label', 'url', 'evidence_ids'], `${label}の設定`);
    check(Array.isArray(value.evidence_ids) && value.evidence_ids.length > 0, `${label}には既存の検証記録IDを1件以上指定してください`);
    check(new Set(value.evidence_ids).size === value.evidence_ids.length, '検証記録IDが重複しています');
    const evidence = value.evidence_ids.map(id => {
      identifier(id);
      const matches = workspace.evidence.filter(item => item.id === id);
      check(matches.length === 1, `検証記録 ${id} が事例内で一意に見つかりません`);
      const item = matches[0];
      source(item.source);
      check(item.execution?.recipe?.runtime === 'browser', `${id}はOpenEMRのブラウザ実行記録ではありません`);
      check(item.execution.recipe.containers?.services?.some(service => service.name === 'openemr'), `${id}にはOpenEMRサービスの実行設定がありません`);
      check(sameRevision(item.source, item.execution.recipe.source), `${id}の参照と実行レシピの版が一致しません`);
      return item;
    });
    check(evidence.every(item => sameRevision(item.source, evidence[0].source)), `${label}の検証記録が異なる版を参照しています`);
    selected.set(label, evidence);
    return { label, source: source(evidence[0].source), url: localUrl(value.url), evidence_ids: [...value.evidence_ids] };
  });
  check(variants[0].source.commit !== variants[1].source.commit, '同じコミットの正常時と再試験を、変更前後の2版にはできません');
  check(variants[0].url !== variants[1].url, 'A・Bには別々の試用先URLが必要です');
  let conditions = input.conditions;
  if (environment) {
    check(environment.state === 'ready' && environment.identical_images === false && Array.isArray(environment.variants), '試用環境は異なる2イメージのready状態が必要です');
    const lines = variants.map(variant => {
      const matches = environment.variants.filter(item => item.label?.toLowerCase() === variant.label);
      check(matches.length === 1, '試用環境のA・Bの定義を確認してください');
      const deployed = matches[0];
      check(deployed.ready === true && sameRevision(deployed.source, variant.source)
        && localUrl(deployed.url) === variant.url, `${variant.label}の試用環境と計画の版・URLが一致しません`);
      check(/^[a-f0-9]{64}$/.test(deployed.recipe_sha256), '試用環境のレシピSHA-256を確認してください');
      check(typeof deployed.app_image === 'string' && /@sha256:[a-f0-9]{64}$/.test(deployed.app_image), '試用環境のアプリイメージにはダイジェストが必要です');
      for (const evidence of selected.get(variant.label)) {
        const image = evidence.execution.recipe.containers?.services?.find(service => service.name === 'openemr')?.image;
        check(image === deployed.app_image, `${evidence.id}と試用環境のOpenEMRイメージが一致しません`);
      }
      return `${variant.label.toUpperCase()}：レシピSHA-256=${deployed.recipe_sha256}、OpenEMRイメージ=${deployed.app_image}`;
    });
    conditions += '\n\n試用環境の記録との照合：\n' + lines.join('\n')
      + '\n版・URL・イメージは登録された記録と照合した。起動中の内容、およびGitコミットとイメージの同一性は、このツールでは確認していない。';
  }
  text(conditions, '条件');
  const request = { expected_revision: workspace.revision, actor: input.actor, reason: input.reason,
    action: { op: 'add_user_trial_plan', plan: { id: input.id, concern: input.concern, task: input.task, conditions, variants } } };
  check(Buffer.byteLength(JSON.stringify(request)) <= 1024 * 1024, '更新要求が1MiBを超えています');
  return request;
}

function main() {
  const args = process.argv.slice(2);
  if (args.length === 1 && args[0] === '--help') {
    console.log('node tools/openemr-trial-plan.mjs --case CASE_DIR_OR_JSON --input PLAN_INPUT.json --output NEW_CHANGE.json [--environment TRIAL.json]');
    return;
  }
  const options = new Map();
  for (let index = 0; index < args.length; index += 2) {
    const key = args[index], value = args[index + 1];
    check(['--case', '--input', '--output', '--environment'].includes(key) && value && !options.has(key), '引数を確認してください。--helpで使い方を表示します');
    options.set(key, value);
  }
  check(['--case', '--input', '--output'].every(key => options.has(key)), '--case・--input・--outputが必要です');
  const workspace = loadWorkspace(options.get('--case'));
  const environment = options.has('--environment') ? readJson(options.get('--environment')) : null;
  const request = buildRequest(readJson(options.get('--input')), workspace, environment);
  fs.writeFileSync(options.get('--output'), JSON.stringify(request, null, 2) + '\n', { flag: 'wx', mode: 0o600 });
  console.log(`試用計画の登録要求を作成しました（事例の版 ${workspace.revision}）。対象アプリの起動や要求の送信は行っていません。`);
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { main(); } catch (error) { console.error(error.message); process.exitCode = 1; }
}
