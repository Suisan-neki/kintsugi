// 画面を使わず、公開したlocalhost APIとCLIで汎用の一件を進める。すべて合成データ。
import assert from 'node:assert/strict';
import {execFileSync,spawn} from 'node:child_process';
import {mkdir,readFile,writeFile,stat,rm} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import net from 'node:net';
const root=resolve(import.meta.dirname,'..'), binary=process.env.REVIEW_BINARY||join(root,'target/debug/mugiurushi');
const output=resolve(process.argv[2]||join(root,'evaluations','backend-'+Date.now()));
await mkdir(output,{recursive:false});
const casePath=join(output,'case'), tokenPath=join(output,'api-token');
execFileSync(binary,['case','init',casePath,'--template','general','--id','general-notification','--title','合成の通知確認','--actor','合成の作成者','--reason','APIの一往復']);
const server=spawn(binary,['api',casePath,'--port','0','--token-file',tokenPath],{stdio:['ignore','pipe','pipe']});
let socket;
try {
 const url=await new Promise((accept,reject)=>{const timeout=setTimeout(()=>reject(Error('API startup timeout')),10000);let text='';server.stdout.on('data',data=>{text+=data;const m=/http:\/\/127\.0\.0\.1:\d+/.exec(text);if(m){clearTimeout(timeout);accept(m[0]);}});server.once('exit',code=>{clearTimeout(timeout);reject(Error('API exited '+code));});});
 const token=await readFile(tokenPath,'utf8');assert.equal((await stat(tokenPath)).mode&0o777,0o600);
 const headers={'Content-Type':'application/json','X-Review-Token':token};
 async function get(path){const r=await fetch(url+path,{headers});assert.equal(r.status,200);return r.json();}
 let state=await get('/api/v1/case');
 async function change(action){const r=await fetch(url+'/api/v1/change',{method:'POST',headers,body:JSON.stringify({expected_revision:state.workspace.revision,actor:'合成の確認者',reason:'API接続の確認',action})});assert.equal(r.status,200,await r.clone().text());state=await r.json();return state;}
 assert.equal((await get('/api/v1/capabilities')).api_version,'v1');
 assert.equal((await fetch(url+'/api/v1/case')).status,403);
 assert.equal((await fetch(url+'/api/v1/change',{method:'POST',headers:{...headers,Origin:'https://example.invalid'},body:'{}'})).status,403);
 await change({op:'add_check',check:{id:'delivered',version:1,text:'通知を読み出した結果が入力と一致する',origin:'合成の条件',source:null,observation:null,rule:'manual_only',dependencies:[],questions:[]}});
 assert.equal(state.checks[0].verdict,'unknown');
 await change({op:'configure_reported_observation',check_id:'delivered',criterion:{assertion:'readback_matches',subject:'notification-1'}});
 function trace(id,outcomes,completed=true){const mode=!completed?'incomplete':outcomes.length===0?'missing':outcomes.length===2?'conflict':outcomes[0];return JSON.parse(execFileSync('python3',[join(root,'examples/general-observations/producer.py'),'--run-id',id,'--mode',mode],{encoding:'utf8'}));}
 function imported(t){const c=state.workspace.checks[0];return {op:'import_reported',condition:{id:c.id,version:c.version,text:c.text,origin:c.origin,kind:'custom'},trace:t};}
 await change(imported(trace('reported-fail',['fail'])));assert.equal(state.checks[0].verdict,'not_satisfied');
 await change({op:'decide',role:'developer',action:'modify',check_ids:['delivered'],change_reference:'合成の改修判断',confidence:null,elapsed_ms:null});
 await change(imported(trace('reported-pass',['pass'])));assert.equal(state.checks[0].verdict,'satisfied');
 const earlier=JSON.stringify(state.workspace.evidence);
 await change(imported(trace('reported-missing',[])));assert.equal(state.checks[0].verdict,'unknown');
 assert.equal(JSON.stringify(state.workspace.evidence.slice(0,2)),earlier);
 await change(imported(trace('reported-conflict',['pass','fail'])));assert.equal(state.checks[0].verdict,'unknown');
 await change(imported(trace('reported-incomplete',['pass'],false)));assert.equal(state.checks[0].verdict,'unknown');
 const beforeInvalid=JSON.stringify(state.workspace);
 const bad=trace('foreign',['pass']);bad.observations[0].correlation.attempt_id='other';
 let r=await fetch(url+'/api/v1/change',{method:'POST',headers,body:JSON.stringify({expected_revision:state.workspace.revision,actor:'合成',reason:'別試行の拒否',action:imported(bad)})});assert.equal(r.status,400);
 assert.deepEqual((await get('/api/v1/case')).workspace,JSON.parse(beforeInvalid));
 r=await fetch(url+'/api/v1/change',{method:'POST',headers,body:JSON.stringify({expected_revision:1,actor:'合成',reason:'古い版の拒否',action:{op:'set_context',key:'runtime',value:'other'}})});assert.equal(r.status,409);
 const compared=await fetch(url+'/api/v1/compare',{method:'POST',headers,body:JSON.stringify({expected_revision:state.workspace.revision,check_id:'delivered',before_id:'evidence-1',after_id:'evidence-3'})});assert.equal(compared.status,200);const comparison=await compared.json();assert.equal(comparison.before.evidence.observed.verdict,'not_satisfied');assert.equal(comparison.after.evidence.observed.verdict,'unknown');
 r=await fetch(url+'/api/v1/compare',{method:'POST',headers,body:JSON.stringify({expected_revision:state.workspace.revision,check_id:'delivered',before_id:'evidence-1',after_id:'evidence-1'})});assert.equal(r.status,400);
 const snapshot={schema:1,fullName:'example/project',commit:'a'.repeat(40),reference:'main',truncated:false,files:{'app.js':'import {save as persist} from "./store.js";\nfunction main(){persist();}','store.js':'export function save(){}'},inventory:[{path:'app.js',size:80,mode:'100644'},{path:'store.js',size:24,mode:'100644'}],omitted:[]};
 const snapshotDirectory=join(casePath,'snapshots',snapshot.fullName);await mkdir(snapshotDirectory,{recursive:true});await writeFile(join(snapshotDirectory,snapshot.commit+'.json'),JSON.stringify(snapshot));
 const analysisResponse=await fetch(url+'/api/v1/understand',{method:'POST',headers,body:JSON.stringify({operation:'analyze',repository:snapshot.fullName,commit:snapshot.commit,path:'app.js',line:2})});assert.equal(analysisResponse.status,200);const analysis=await analysisResponse.json();assert.equal(analysis.guide.calls[0].candidates[0].path,'store.js');
 // 途中のHTTPリクエストがあっても、別接続で事例を読める。
 socket=net.createConnection(new URL(url).port,'127.0.0.1');await new Promise(resolve=>socket.once('connect',resolve));socket.write('GET /api/v1/case HTTP/1.1\r\n');
 const start=Date.now();await get('/api/v1/case');assert.ok(Date.now()-start<2000,'a stalled client blocked other requests');socket.destroy();
 const saved=await get('/api/v1/backup');await writeFile(join(output,'backup.json'),JSON.stringify(saved));
 execFileSync(binary,['case','restore',join(output,'restored'),join(output,'backup.json')]);
 const restored=JSON.parse(execFileSync(binary,['case','show',join(output,'restored')],{encoding:'utf8'}));assert.deepEqual(restored.workspace,state.workspace);assert.equal(restored.checks[0].verdict,'unknown');
 assert.throws(()=>execFileSync(binary,['case','restore',join(output,'restored'),join(output,'backup.json')],{stdio:'pipe'}));
 const cliComparison=JSON.parse(execFileSync(binary,['case','compare',casePath,'--before','evidence-1','--after','evidence-3','--condition','delivered'],{encoding:'utf8'}));assert.deepEqual(cliComparison,comparison);
 const cliBackup=join(output,'cli-backup.json');execFileSync(binary,['case','backup',casePath,'--output',cliBackup]);assert.deepEqual(JSON.parse(await readFile(cliBackup,'utf8')),saved);
 let liveReader=null;
 if(process.argv.includes('--live')) {
  const response=await fetch(url+'/api/v1/understand',{method:'POST',headers,body:JSON.stringify({operation:'read',url:'https://github.com/Suisan-neki/mugiurushi',reference:'63c8dac0789ed712f267a9956f5f012c645aea88'})});
  const body=await response.json();assert.equal(response.status,200,body.error);
  assert.equal(body.source.commit,'63c8dac0789ed712f267a9956f5f012c645aea88');
  liveReader={commit:body.source.commit,inventory:body.source.inventory.length,read_files:body.source.files_read.length};
 }
 const result={schema_version:1,live_reader:liveReader,case_revision:state.workspace.revision,checks:['versioned API','token and origin','generic reported fail/pass/missing/conflict/incomplete','foreign correlation rejected','stale revision rejected','historical decision basis','Rust comparison via API and CLI','static repository analysis API','reads during stalled connection','all-revision backup and restore','existing restore target rejected'],scope:'synthetic inputs; no arbitrary application execution or human evaluation',comparison};
 await writeFile(join(output,'result.json'),JSON.stringify(result,null,2));await writeFile(join(output,'analysis.json'),JSON.stringify(analysis,null,2));console.log(JSON.stringify({output,checks:result.checks,human_evaluation:'not_conducted'}));
} finally {socket?.destroy();server.kill('SIGTERM');await rm(tokenPath,{force:true});}
