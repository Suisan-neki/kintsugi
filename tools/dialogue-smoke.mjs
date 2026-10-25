// 実Rustサーバーと合成Difyで、図の疑問→継続対話→検証事項→結果の再参照を確認する。
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { spawn, execFileSync } from 'node:child_process';
import { mkdtemp, mkdir, writeFile, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve, join } from 'node:path';
import { chromium } from 'playwright';
import { startSyntheticArchitectureAI } from './system-diagram-fixture.mjs';
const root=resolve(import.meta.dirname,'..'), temp=await mkdtemp(join(tmpdir(),'eel-dialogue-')), data=join(temp,'app');
const snapshot=JSON.parse(await readFile(join(root,'tests/fixtures/reader/system.json'),'utf8'));
snapshot.inventory.push({path:'interface/forms/soap/save.php',mode:'100644'});
snapshot.files['interface/forms/soap/save.php']="<?php\nsqlInsert('form_soap', $value);\n";
await mkdir(join(data,'snapshots',snapshot.fullName),{recursive:true});
await writeFile(join(data,'snapshots',snapshot.fullName,snapshot.commit+'.json'),JSON.stringify(snapshot));
const trialCommit='b'.repeat(40);
await writeFile(join(data,'snapshots',snapshot.fullName,trialCommit+'.json'),JSON.stringify({...snapshot,commit:trialCommit,reference:trialCommit}));
const native=process.env.EEL_DIALOGUE_NATIVE==='1';
const ai=await startSyntheticArchitectureAI(), calls=[], pending=new Map(); let invalidMode='', serial=0;
const dify=createServer(async(req,res)=>{
  try {
    assert.equal(req.headers.authorization,'Bearer synthetic-dify-private-key');
    if(req.method==='POST'&&req.url==='/v1/chat-messages'){
      let text='';for await(const chunk of req)text+=chunk;
      const body=JSON.parse(text), query=JSON.parse(body.query), context=JSON.parse(body.inputs.system_context); calls.push({body,query,context});
      assert.equal(query.audience,body.inputs.audience);
      assert.match(query.writing_policy,/主述・修飾・条件の点検/);
      assert.equal(query.writing_policy_source.commit,'488dd37c0baa67ec65bf155aced9fa1a01d261e2');
      assert.ok(context.architecture.nodes.some(n=>n.id===context.selected_element));
      const ready=Boolean(query.previous_record?.expectation)||query.message.includes('開き直して');
      const record={concern:query.previous_record?.concern||query.message,actor:ready?'私':'',operation:ready?'保存して画面を開き直す':'',expectation:ready?'保存後に画面を開き直しても同じ内容を読める':'',related_object_ids:context.architecture.nodes.filter(n=>['api','database'].includes(n.role)).map(n=>n.id),claims:[{text:'APIが定義されている',refs:[{path:'server/main.py',line:3,end_line:3}]}],unknowns:['保存後の再読込は未検証です。'],test_plan:ready?[{operation:'架空のメモを保存して画面を開き直す',expected:'同じ内容を読める',observe:'保存時の表示と開き直した後の本文'}]:[],reported_observations:[],improvement_ideas:[]};
      if(query.message.includes('SOAP')) record.claims=[{text:'form_soapへの書込関数を呼ぶ記述がある',refs:[{path:'interface/forms/soap/save.php',line:2,end_line:2}]}];
      if(invalidMode==='object')record.related_object_ids=['invented_store'];
      if(invalidMode==='ref')record.claims[0].refs[0].line=999;
      const envelope={request_id:invalidMode==='stale'?'previous-request':query.request_id,record};
      const id=body.conversation_id||`synthetic-conversation-${++serial}`;pending.set(id,JSON.stringify(envelope));
      res.end(JSON.stringify({conversation_id:id,answer:query.results.checks.length?'取得した結果を引き継ぎました。観測がないことだけでは保存の失敗と断定できません。':ready?'前の会話で分かった期待を引き継いで検証案を整理しました。':'保存後に残るか不安なのですね。誰がどんな操作をしたときに確認したいですか？'}));return;
    }
    if(req.method==='GET'&&req.url.startsWith('/v1/conversations/')){
      const url=new URL(req.url,'http://localhost');assert.ok(url.searchParams.get('user'));assert.equal(url.searchParams.get('limit'),'100');
      const id=url.pathname.split('/')[3];res.end(JSON.stringify({data:[{name:'case_record',value:pending.get(id)}],has_more:false,limit:100}));return;
    }
    res.statusCode=404;res.end('{}');
  }catch(error){res.statusCode=500;res.end(JSON.stringify({error:error.message}));}
});
await new Promise(ok=>dify.listen(0,'127.0.0.1',ok));
await writeFile(join(data,'ai.json'),JSON.stringify({base_url:ai.url,model:'synthetic-architecture',api_key:'synthetic-native-private-key'}));
if(!native)await writeFile(join(data,'dify.json'),JSON.stringify({base_url:`http://127.0.0.1:${dify.address().port}/v1`,api_key:'synthetic-dify-private-key'}));
const binary=process.env.REVIEW_BINARY||join(root,'target/debug/mugiurushi');
const env={...process.env};for(const n of ['EEL_AI_BASE_URL','EEL_AI_MODEL','EEL_AI_API_KEY_FILE'])delete env[n];
const child=spawn(binary,['app',data,'--port','0'],{env,stdio:['ignore','pipe','pipe']});
let browser,page;
try{
  const url=await new Promise((ok,no)=>{let out='',error='';const timer=setTimeout(()=>no(Error(error||'startup timeout')),15000);child.stderr.on('data',d=>error+=d);child.stdout.on('data',d=>{out+=d;const m=/http:\/\/127\.0\.0\.1:\d+/.exec(out);if(m){clearTimeout(timer);ok(m[0]);}});child.once('exit',()=>{clearTimeout(timer);no(Error(error));});});
  if(process.env.EEL_DIALOGUE_API_ONLY==='1'){
    const queries=()=>native?ai.state.sent.filter(v=>v.messages[0].content.includes('request_id')).map(v=>({query:JSON.parse(v.messages[1].content.split('今回の入力:\n')[1])})):calls;
    const html=await (await fetch(url)).text(),token=/name="review-token" content="([^"]+)"/.exec(html)[1],headers={'X-Review-Token':token,'Content-Type':'application/json'};
    const post=async(path,data)=>{const r=await fetch(url+'/api/v1/'+path,{method:'POST',headers,body:JSON.stringify(data)});return {status:r.status,body:await r.json()};};
    const view=async()=> (await fetch(url+'/api/v1/case',{headers})).json();
    const change=async(action)=>{const v=await view(),r=await post('change',{expected_revision:v.workspace.revision,actor:'合成の開発者',reason:'対話から検証結果への接続確認',action});assert.equal(r.status,200,JSON.stringify(r.body));return r.body;};
    const generated=await post('system-diagram',{repository:snapshot.fullName,commit:snapshot.commit});assert.equal(generated.status,200);
    const binding={repository:snapshot.fullName,commit:snapshot.commit,revision:generated.body.result.model_revision,object_id:generated.body.result.nodes.find(n=>n.role==='database').id};
    const read=async()=> { const r=await post('system-diagram/dialogue/read',binding); assert.equal(r.status,200,JSON.stringify(r.body)); return r.body; };
    assert.equal((await read()).turn,0);
    const ask=async(question,expected_turn,audience='operator')=>post('system-diagram/dialogue',{binding,question,expected_turn,audience});
    let r=await ask('保存した後に残るか不安',0);assert.equal(r.status,200,JSON.stringify(r.body));assert.equal(r.body.turn,1);
    r=await ask('私が保存して画面を開き直しても同じメモを読めるようにしたい。検証案にして',1);assert.equal(r.status,200,JSON.stringify(r.body));
    assert.equal(r.body.record.actor,'私');assert.equal(r.body.record.test_plan.length,1);if(!native)assert.equal(calls[1].body.conversation_id,'synthetic-conversation-1');assert.ok(queries()[1].query.previous_record.concern);
    const before=await view();assert.equal(before.workspace.checks.length,0);assert.equal(before.workspace.evidence.length,0);
    for(const mode of ['object','ref','stale']){invalidMode=mode;ai.state.dialogueMode=mode;assert.equal((await ask('不正な整理案を拒否する',2)).status,400);assert.equal((await read()).turn,2);}invalidMode='';ai.state.dialogueMode='';
    const callCount=queries().length;assert.equal((await ask('古い画面から送信',1)).status,400);assert.equal(queries().length,callCount);
    const saved=await read(),statementId='dialogue-test',source={repository:binding.repository,commit:binding.commit,path:'server/main.py',line:3,end_line:3};
    await change({op:'add_statement',statement:{id:statementId,kind:'unanswered',role:'operator',text:saved.record.concern,origin:JSON.stringify({kind:'repository_concern',...binding,model_revision:binding.revision,dialogue_id:saved.thread_id,dialogue_turn:2}),source,check_ids:[]}});
    await change({op:'add_check',check:{id:'dialogue-check',version:1,text:'私が保存後に開き直しても同じ内容を読める',origin:'repository-concern:'+statementId,source,observation:null,rule:'manual_only',dependencies:[],questions:saved.record.unknowns.map((text,n)=>({id:'unknown-'+n,text,answer:null,origin:null}))}});
    let v=await view();assert.equal(v.workspace.statements[0].confirmations.length,0);assert.equal(v.workspace.checks[0].questions[0].answer,null);assert.equal(v.workspace.evidence.length,0);
    await change({op:'configure_reported_observation',check_id:'dialogue-check',criterion:{assertion:'readback_matches',subject:'notification-1'}});
    const trace=JSON.parse(execFileSync('python3',[join(root,'examples/general-observations/producer.py'),'--run-id','dialogue-api-missing','--mode','missing'],{encoding:'utf8'}));
    v=await view();const c=v.workspace.checks[0];trace.source=c.source;await change({op:'import_reported',condition:{id:c.id,version:c.version,text:c.text,origin:c.origin,kind:'custom'},trace});
    r=await ask('この結果から何が分かる？',2,'developer');assert.equal(r.status,200,JSON.stringify(r.body));assert.equal(queries().at(-1).query.results.checks[0].result.verdict,'unknown');assert.equal(queries().at(-1).query.audience,'技術者');assert.equal(r.body.turn,3);
    assert.equal((await post('system-diagram/dialogue/read',{...binding,object_id:null})).body.turn,0);
    assert.equal((await post('system-diagram/dialogue/read',{...binding,revision:'f'.repeat(64)})).status,400);
    assert.equal((await fetch(url+'/dify.json',{headers})).status,404);assert.ok(!JSON.stringify(await read()).includes('private-key'));
    r=await ask('SOAPの保存処理は？',3,'developer');assert.equal(r.status,200,JSON.stringify(r.body));assert.equal(r.body.claims[0].refs[0].path,'interface/forms/soap/save.php');assert.equal(r.body.claims[0].references_in_context,true);
    if(!native){assert.equal(calls.at(-1).body.conversation_id,'');assert.match(calls.at(-1).context.code_excerpts,/form_soap/);assert.ok(calls.at(-1).query.previous_record);}
    console.log(JSON.stringify({checks:['Dify blocking chat and conversation variables protocol','same conversation ID and previous record','reject invented object, invalid refs and stale request','optimistic concurrency','no automatic checks, evidence or agreement','human-edited expectation and unknowns retained','reported missing observation reaches same conversation','audience changes without changing record','selection and diagram isolation','credentials stay on server'],backend:native?'native':'synthetic_dify',browser:'not_run',actual_dify_api:'not_connected',actual_target_execution:'not_conducted'}));
  }else{
  browser=await chromium.launch({headless:true,executablePath:process.env.REVIEW_BROWSER_EXECUTABLE||undefined,args:['--no-sandbox']});
  page=await browser.newPage({viewport:{width:1440,height:1000}});page.setDefaultTimeout(10000);const errors=[];page.on('pageerror',e=>errors.push(e.message));
  await page.goto(url);const token=await page.locator('meta[name=review-token]').getAttribute('content'), headers={'X-Review-Token':token,'Content-Type':'application/json'};
  const post=async(path,data)=>{const r=await page.request.post(url+'/api/v1/'+path,{headers,data});return {status:r.status(),body:await r.json()};};
  const view=async()=> (await page.request.get(url+'/api/v1/case',{headers})).json();
  const change=async(action)=>{const v=await view();const r=await post('change',{expected_revision:v.workspace.revision,actor:'合成の開発者',reason:'対話から検証結果へ接続する試験',action});assert.equal(r.status,200,JSON.stringify(r.body));return r.body;};
  await page.getByRole('button',{name:/非技術者/}).click();await page.locator('#repository-url').fill('https://github.com/'+snapshot.fullName);
  await page.getByText('ブランチ・コミットを指定する',{exact:true}).click();await page.locator('#repository-ref').fill(snapshot.commit);await page.getByRole('button',{name:'構成図を作成する',exact:true}).click();
  await page.locator('#system-screen:not([hidden])').waitFor();
  const model=(await post('system-diagram',{repository:snapshot.fullName,commit:snapshot.commit})).body.result;
  const dbId=model.nodes.find(n=>n.role==='database').id;
  await page.locator(`[data-node="${dbId}"]`).click();
  const ask=async(text,count)=>{await page.locator('#concern-text').fill(text);await page.locator('#ask-concern-ai').click();await page.locator('.dialogue-history summary',{hasText:`${count}往復`}).waitFor();};
  await ask('保存しても残らないのでは、と不安です',1);
  assert.equal((await view()).workspace.checks.length,0);
  await ask('使うのは私です。保存して画面を開き直しても同じメモを読めるようにしたい。検証案にして',2);
  assert.equal(calls[1].body.conversation_id,calls[0].body.conversation_id||'synthetic-conversation-1');
  assert.equal(calls[1].query.previous_record.concern,'保存しても残らないのでは、と不安です');
  assert.equal(await page.locator('[data-discussion="true"]').count(),2);
  const binding={repository:snapshot.fullName,commit:snapshot.commit,revision:new URL(page.url()).searchParams.get('diagram'),object_id:dbId};
  const read=async()=> { const r=await post('system-diagram/dialogue/read',binding); assert.equal(r.status,200,JSON.stringify(r.body)); return r.body; };
  const before=await read();assert.equal(before.turn,2);assert.equal(before.record_status,'ai_draft');assert.ok(!JSON.stringify(before).includes('private-key'));
  for(const mode of ['object','ref','stale']){invalidMode=mode;const bad=await post('system-diagram/dialogue',{binding,expected_turn:2,audience:'operator',question:'変な根拠の拒否を確認'});assert.equal(bad.status,400);assert.equal((await read()).turn,2);}invalidMode='';
  const conflicted=await post('system-diagram/dialogue',{binding,expected_turn:1,audience:'operator',question:'古い画面から送信'});assert.equal(conflicted.status,400);
  await page.reload();await page.locator('.dialogue-history summary',{hasText:'2往復'}).waitFor();
  await page.getByRole('button',{name:/立場：非技術者/}).click();await page.getByRole('button',{name:/^技術者/}).click();await page.locator('.dialogue-history summary',{hasText:'2往復'}).waitFor();
  assert.equal((await read()).turn,2);
  await page.getByLabel('検証する動作',{exact:true}).fill('私が保存後に開き直しても同じ内容を読める');
  await page.getByRole('button',{name:'この内容を検証する',exact:true}).click();await page.locator('#trial:not([hidden])').waitFor();
  let v=await view(),check=v.workspace.checks[0];assert.equal(check.rule,'manual_only');assert.equal(v.workspace.evidence.length,0);assert.equal(check.questions[0].text,'保存後の再読込は未検証です。');
  assert.equal(v.workspace.statements[0].confirmations.length,0);assert.equal(JSON.parse(v.workspace.statements[0].origin).dialogue_turn,2);
  assert.equal(await page.getByLabel('検証する動作',{exact:true}).inputValue(),check.id);
  assert.equal(await page.getByLabel('検証する動作',{exact:true}).locator('option',{hasText:check.text}).count(),1);
  await page.getByRole('link',{name:/検証する動作を決める/}).click();
  await page.locator(`[data-check="${check.id}"]`).waitFor();
  await page.getByRole('button',{name:'この動作を検証する',exact:true}).click();
  await page.locator('#trial:not([hidden])').waitFor();
  // 外部プログラムが出した合成の報告を、同じ検証事項の結果として取り込む。
  await change({op:'configure_reported_observation',check_id:check.id,criterion:{assertion:'readback_matches',subject:'notification-1'}});
  const trace=JSON.parse(execFileSync('python3',[join(root,'examples/general-observations/producer.py'),'--run-id','dialogue-reported-missing','--mode','missing'],{encoding:'utf8'}));
  v=await view();check=v.workspace.checks[0];trace.source=check.source;await change({op:'import_reported',condition:{id:check.id,version:check.version,text:check.text,origin:check.origin,kind:'custom'},trace});
  await page.getByRole('link',{name:'構成図と疑問に戻る',exact:true}).click();await page.locator('.dialogue-history summary',{hasText:'2往復'}).waitFor();
  await ask('この結果から何が分かる？',3);const last=calls.at(-1);assert.equal(last.query.results.checks[0].result.verdict,'unknown');assert.equal(last.query.results.checks[0].result.evidence[0].run_id,'dialogue-reported-missing');
  assert.match(await page.locator('.dialogue-reply').last().innerText(),/失敗と断定できません/);
  // 同じ期待は複製せず、同じ会話で編集した確認項目も版へ反映する。
  await page.getByRole('button',{name:'この内容で再検証する',exact:true}).click();await page.locator('#trial:not([hidden])').waitFor();
  v=await view();assert.equal(v.workspace.checks.length,1);assert.equal(v.workspace.checks[0].version,check.version);
  for(const expectation of ['開き直した後の本文が保存前の本文と一致する','二度開き直した後も保存前の本文と一致する']){
    await page.getByRole('link',{name:'構成図と疑問に戻る',exact:true}).click();await page.locator('.dialogue-history summary',{hasText:'3往復'}).waitFor();
    await page.getByLabel('検証する動作',{exact:true}).fill(expectation);
    await page.getByRole('button',{name:'この内容で再検証する',exact:true}).click();await page.locator('#trial:not([hidden])').waitFor();
    v=await view();assert.equal(v.workspace.checks.length,1);assert.equal(v.workspace.checks[0].text,expectation);assert.equal(v.workspace.checks[0].version,++check.version);
  }
  await page.getByRole('link',{name:'構成図と疑問に戻る',exact:true}).click();await page.locator('.dialogue-history summary',{hasText:'3往復'}).waitFor();
  assert.equal((await post('system-diagram/dialogue/read',{...binding,object_id:null})).body.turn,0);
  assert.equal((await post('system-diagram/dialogue/read',{...binding,revision:'f'.repeat(64)})).status,400);
  // 試す版を変え、元の図へ戻って再び検証へ進んでも、図と試験のコミットを混ぜない。
  await change({op:'bind_source',check_id:check.id,source:{...check.source,commit:trialCommit}});
  await page.reload();await page.locator('.dialogue-history summary',{hasText:'3往復'}).waitFor();
  const trialLink=page.getByRole('link',{name:'検証の設定へ進む',exact:true});
  const trialUrl=new URL(await trialLink.getAttribute('href'),url);
  assert.equal(trialUrl.searchParams.get('ref'),trialCommit);
  assert.equal(trialUrl.searchParams.get('system_ref'),snapshot.commit);
  await trialLink.click();await page.locator('#trial:not([hidden])').waitFor();
  await page.reload();await page.locator('#trial:not([hidden])').waitFor();
  assert.equal(new URL(await page.getByRole('link',{name:'構成図と疑問に戻る',exact:true}).getAttribute('href'),url).searchParams.get('ref'),snapshot.commit);
  await page.getByRole('link',{name:'構成図と疑問に戻る',exact:true}).click();await page.locator('.dialogue-history summary',{hasText:'3往復'}).waitFor();
  assert.equal((await read()).turn,3);
  assert.equal(new URL(await page.getByRole('link',{name:'検証結果を確認する',exact:true}).getAttribute('href'),url).searchParams.get('system_ref'),snapshot.commit);
  await mkdir(join(root,'test-results'),{recursive:true});await page.screenshot({path:join(root,'test-results/dialogue-handoff.png'),fullPage:true});
  assert.deepEqual(errors,[]);console.log(JSON.stringify({checks:['Dify chat and variables protocol','multi-turn structured record','same diagram highlights','reload and role switch','reject invented nodes, refs and stale variable','conflict preserves history','human-edited expectation and unknowns handoff','no automatic evidence or agreement','reported result returns to original conversation','diagram and selection isolation'],actual_dify_api:'not_connected',actual_target_execution:'not_conducted'}));
  }
}finally{child.kill('SIGTERM');await browser?.close();await ai.close();await new Promise(ok=>dify.close(ok));await rm(temp,{recursive:true,force:true});}
