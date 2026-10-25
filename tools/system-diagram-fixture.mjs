// 自動試験だけで使う合成のAI応答。実モデルの品質確認とは区別する。
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
export async function startSyntheticArchitectureAI() {
  const graph = JSON.parse(await readFile(new URL('../tests/fixtures/reader/system-ai.json', import.meta.url), 'utf8'));
  const state = { calls: 0, questions: 0, mode: 'valid', revision: 1, sent: [] };
  const server = createServer(async (req, res) => {
    let bytes = ''; for await (const chunk of req) bytes += chunk;
    const body = JSON.parse(bytes); state.sent.push(body);
    if (state.mode === 'unavailable') { res.writeHead(503); res.end('synthetic outage'); return; }
    if (body.messages[0].content.includes('EEL_CONNECTION_TEST')) {
      res.writeHead(200, { 'Content-Type': 'application/json' }); res.end(JSON.stringify({ model: 'synthetic-architecture', choices: [{ message: { content: '{"connected":true}' } }] })); return;
    }
    if (!body.messages[0].content.includes('主述・修飾・条件の点検')) throw Error('本家yomiyasuの規則が生成要求にありません');
    if (body.messages[0].content.includes('JSONは {"answer"')) {
      const input = JSON.parse(body.messages[1].content);
      const record = JSON.parse(input.record).record;
      const response = { answer: record.expectation ? '前の会話で確認した人・操作・期待を引き継ぎ、未実施の検証案を整理しました。保存表示と保存先の状態は別に確認します。' : '保存の失敗を利用者が確認できるか、期待する動作と実際の結果を分けて確かめます。どんな動作を期待していますか？' };
      res.writeHead(200, { 'Content-Type': 'application/json' }); res.end(JSON.stringify({model:'synthetic-dialogue',choices:[{message:{content:JSON.stringify(response)}}]})); return;
    }
    if (body.messages[0].content.includes('request_id')) {
      state.questions++;
      const query = JSON.parse(body.messages[1].content.split('今回の入力:\n')[1]);
      const context = JSON.parse(body.messages[1].content.split('今回の入力:\n')[0].replace('調査資料:\n', '').trim());
      const ready = query.message.includes('開き直して') || query.previous_record?.expectation;
      const record = {concern:query.previous_record?.concern || query.message, actor:ready?'私':'', operation:ready?'保存して画面を開き直す':'', expectation:ready?'保存後に画面を開き直しても同じ内容を読める':'',related_object_ids:context.architecture.nodes.filter(n=>['api','database'].includes(n.role)).map(n=>n.id),claims:[{text:'記録管理のAPIが定義されています。',refs:[{path:'server/main.py',line:3,end_line:3}]}],unknowns:['再読込時の表示は未検証です。'],test_plan:ready?[{operation:'架空のメモを保存して画面を開き直す',expected:'同じ内容を読める',observe:'保存時の表示と開き直した後の本文'}]:[],reported_observations:[],improvement_ideas:[]};
      if(query.message.includes('SOAP')) record.claims=[{text:'form_soapへの書込関数を呼ぶ記述がある',refs:[{path:'interface/forms/soap/save.php',line:2,end_line:2}]}];
      if (state.mode === 'bad-dialogue' || state.dialogueMode === 'object') record.related_object_ids = ['invented_database'];
      if (state.dialogueMode === 'ref') record.claims[0].refs[0].line = 999;
      res.writeHead(200, { 'Content-Type': 'application/json' }); res.end(JSON.stringify({model:'synthetic-dialogue',choices:[{message:{content:JSON.stringify({request_id:state.dialogueMode === 'stale' ? 'previous-request' : query.request_id,record})}}]})); return;
    }
    const question = body.messages[0].content.includes('test_candidates');
    if (question) state.questions++; else state.calls++;
    const result = structuredClone(graph);
    if (state.revision > 1) { result.summary.workflow += ' これは再生成した説明です。'; result.summary.operator_workflow += ' これは再生成した説明です。'; }
    if (state.dense) {
      for (let i = result.nodes.length; i < 19; i++) {
        result.nodes.push({ ...result.nodes[2], id: `detail_${i}`, label: `記録の確認処理${i}` });
        result.edges.push({ ...result.edges[0], from: i === 7 ? 'records_api' : `detail_${i - 1}`, to: `detail_${i}`, label: '確認した記録を渡す' });
      }
    }
    if (body.messages[1].content.includes('repository: example/other')) {
      result.nodes.push({ ...result.nodes[2], id: 'worker', label: '添付ファイルの変換処理', role: 'application', path: 'compose.yaml', refs: [{ path: 'compose.yaml', line: 1, end_line: 1 }] });
      result.edges.push({ ...result.edges[0], from: 'worker', to: 'records_api', label: 'APIの起動を待つ', relation_type: 'startup', evidence: 'code', refs: [{ path: 'compose.yaml', line: 1, end_line: 1 }] });
    }
    if (state.mode === 'invalid' || state.mode === 'repair' && state.calls === 1) result.nodes[1].refs = [{ path: 'unseen.py', line: 1, end_line: 1 }];
    const response = question ? { answer: '保存の失敗を利用者が確認できるか、期待する動作と実際の結果を分けて確かめます。', claims: [{ text: '記録管理のAPIが定義されています。', refs: [{ path: 'server/main.py', line: 3, end_line: 3 }] }], unknowns: ['通信が途切れたときの表示は未検証です。'], test_candidates: [] } : result;
    res.writeHead(200, { 'Content-Type': 'application/json' }); res.end(JSON.stringify({ model: 'synthetic-architecture', choices: [{ message: { content: JSON.stringify(response) } }] }));
  });
  await new Promise(accept => server.listen(0, '127.0.0.1', accept));
  return { url: `http://127.0.0.1:${server.address().port}/v1`, state, close: () => new Promise(accept => server.close(accept)) };
}
