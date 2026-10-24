// Synthetic records test the bridge, not OpenEMR or participant behavior.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { buildRequest, loadWorkspace } from './openemr-trial-plan.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const recipe = JSON.parse(fs.readFileSync(path.join(root, 'examples/openemr-browser/soap-recipe.json')));
function fixtures() {
  const before = { id: 'evidence-before', source: recipe.source, execution: { recipe: structuredClone(recipe) } };
  const after = structuredClone(before);
  after.id = 'evidence-after';
  after.source = { ...after.source, repository: 'synthetic-fork/openemr-research', commit: 'b'.repeat(40) };
  after.execution.recipe.source = after.source;
  after.execution.recipe.containers.services.find(service => service.name === 'openemr').image = 'synthetic/openemr@sha256:' + 'b'.repeat(64);
  const workspace = { revision: 3, evidence: [before, after], user_trial_plans: [] };
  const input = { id: 'soap-save-1', actor: '合成の準備担当', reason: '入力形式の自動試験',
    concern: '仮想事例。保存完了が分からない。', task: '架空のSOAPを保存し、判断した根拠を記録する。',
    conditions: '自動試験の合成値。人による試用は未実施。', variants: [
      { label: 'a', url: 'https://127.0.0.1:34567/interface/login/login.php', evidence_ids: [before.id] },
      { label: 'b', url: 'https://127.0.0.1:34568/interface/login/login.php', evidence_ids: [after.id] },
    ] };
  const environment = { state: 'ready', identical_images: false, variants: input.variants.map((variant, index) => ({
    label: variant.label.toUpperCase(), source: workspace.evidence[index].source, ready: true, url: variant.url,
    recipe_sha256: 'c'.repeat(64), app_image: workspace.evidence[index].execution.recipe.containers.services.find(service => service.name === 'openemr').image,
  })) };
  return { input, workspace, environment };
}

test('existing per-version browser evidence supplies source, fork and exact selected IDs', () => {
  const { input, workspace, environment } = fixtures();
  const before = JSON.stringify({ input, workspace, environment });
  const result = buildRequest(input, workspace, environment);
  assert.equal(result.expected_revision, 3);
  assert.equal(result.action.op, 'add_user_trial_plan');
  assert.deepEqual(result.action.plan.variants.map(variant => variant.source), workspace.evidence.map(item => item.source));
  assert.deepEqual(result.action.plan.variants.map(variant => variant.evidence_ids), [['evidence-before'], ['evidence-after']]);
  assert.match(result.action.plan.conditions, /同一性は、このツールでは確認していない/);
  assert(!JSON.stringify(result).includes('verification_passed'));
  assert(!JSON.stringify(result).includes('credentials'));
  assert.equal(JSON.stringify({ input, workspace, environment }), before, 'request preparation must not change records');
});

test('normal and retry of one commit cannot count as before and after', () => {
  const { input, workspace } = fixtures();
  workspace.evidence[1].source = workspace.evidence[0].source;
  workspace.evidence[1].execution.recipe.source = workspace.evidence[0].source;
  assert.throws(() => buildRequest(input, workspace), /同じコミット/);
});

test('unknown, missing, mixed-version or non-browser evidence is rejected', () => {
  for (const change of [
    ({ input }) => { input.variants[1].evidence_ids = ['not-existing']; },
    ({ input }) => { input.variants[1].evidence_ids = []; },
    ({ input }) => { input.variants[1].evidence_ids.push('evidence-before'); },
    ({ workspace }) => { workspace.evidence[1].execution.recipe.runtime = 'node'; },
    ({ workspace }) => { workspace.evidence[1].execution.recipe.source = workspace.evidence[0].source; },
  ]) {
    const fixture = fixtures();
    change(fixture);
    assert.throws(() => buildRequest(fixture.input, fixture.workspace));
  }
});

test('public URLs, credentials, query values and equivalent A/B URLs are rejected', () => {
  for (const url of ['https://example.com/', 'http://localhost.evil/', 'http://0.0.0.0/', 'file:///tmp/test',
    'https://operator:secret@127.0.0.1/', 'https://127.0.0.1/?site=default', 'https://127.0.0.1/#token',
    'https://127.0.0.1:34567/interface/login/login.php']) {
    const { input, workspace } = fixtures();
    input.variants[1].url = url;
    assert.throws(() => buildRequest(input, workspace), url);
  }
  const { input, workspace } = fixtures();
  input.variants[0].url = 'http://[::1]:34567/';
  assert.equal(buildRequest(input, workspace).action.plan.variants[0].url, input.variants[0].url);
});

test('environment must be ready and match source, URL and tested image', () => {
  for (const change of [
    fixture => { fixture.environment.state = 'removed'; },
    fixture => { fixture.environment.identical_images = true; },
    fixture => { fixture.environment.variants[1].source = fixture.workspace.evidence[0].source; },
    fixture => { fixture.environment.variants[1].url = 'http://127.0.0.1:2222/'; },
    fixture => { fixture.environment.variants[1].app_image = 'synthetic/openemr@sha256:' + 'd'.repeat(64); },
    fixture => { fixture.environment.variants[1].ready = false; },
  ]) {
    const fixture = fixtures();
    change(fixture);
    assert.throws(() => buildRequest(fixture.input, fixture.workspace, fixture.environment));
  }
});

test('case revision loading and CLI use actual latest snapshot and never overwrite output', t => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'mugiurushi-trial-plan-'));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  const revisions = path.join(directory, 'case/revisions');
  fs.mkdirSync(revisions, { recursive: true });
  const { input, workspace } = fixtures();
  fs.writeFileSync(path.join(revisions, '00000000000000000002.json'), JSON.stringify({ ...workspace, revision: 2 }));
  fs.writeFileSync(path.join(revisions, '00000000000000000003.json'), JSON.stringify(workspace));
  fs.writeFileSync(path.join(revisions, '00000000000000000004.pending'), '{}');
  assert.equal(loadWorkspace(path.dirname(revisions)).revision, 3);
  const inputPath = path.join(directory, 'input.json'), output = path.join(directory, 'change.json');
  fs.writeFileSync(inputPath, JSON.stringify(input));
  const args = [path.join(root, 'tools/openemr-trial-plan.mjs'), '--case', path.dirname(revisions), '--input', inputPath, '--output', output];
  const run = () => spawnSync(process.execPath, args, { encoding: 'utf8' });
  assert.equal(run().status, 0);
  assert.equal(JSON.parse(fs.readFileSync(output)).expected_revision, 3);
  const saved = fs.readFileSync(output, 'utf8');
  assert.notEqual(run().status, 0);
  assert.equal(fs.readFileSync(output, 'utf8'), saved);
  fs.writeFileSync(path.join(directory, 'case/restore.pending'), '');
  assert.throws(() => loadWorkspace(path.dirname(revisions)), /復元が完了/);
});
