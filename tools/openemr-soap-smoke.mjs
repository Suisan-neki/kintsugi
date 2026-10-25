import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import {spawnSync,execFileSync} from 'node:child_process';
const root=path.resolve(import.meta.dirname,'..'),bin=process.env.REVIEW_BINARY||path.join(root,'target/debug/mugiurushi');
const recipe=JSON.parse(fs.readFileSync(path.join(root,'examples/openemr-browser/soap-recipe.json')));
const output=path.join(root,'evaluations/ci-openemr-soap');fs.mkdirSync(output,{recursive:true});
const repository=path.join(output,'repository');fs.mkdirSync(repository);
for(const args of [['init',repository],['-C',repository,'remote','add','origin','https://github.com/'+recipe.source.repository+'.git'],['-C',repository,'fetch','--depth','1','origin',recipe.source.commit]])execFileSync('git',args,{stdio:'inherit'});
function cli(args){const r=spawnSync(bin,args,{encoding:'utf8',maxBuffer:64*1024*1024,timeout:600000});assert.equal(r.error,undefined,String(r.error));assert.equal(r.status,0,(r.stderr||'').slice(-12000));return JSON.parse(r.stdout);}
const caseDir=path.join(output,'case'),spec={schema_version:1,revision:1,id:'openemr-soap',title:'架空の患者のSOAPを保存・再表示する',statements:[],context:{},checks:[{id:'openemr',version:1,text:'SOAPの4項目を保存し、サーバーからの再表示とDBの値を照合できる',origin:'Codexが選んだ試験条件',source:recipe.source,rule:'manual_only',dependencies:[],questions:[]}],evidence:[],decisions:[],history:[]};
const specFile=path.join(output,'case-spec.json');fs.writeFileSync(specFile,JSON.stringify(spec));let view=cli(['case','init',caseDir,'--file',specFile]);
const configure=path.join(output,'configure.json');fs.writeFileSync(configure,JSON.stringify({expected_revision:view.workspace.revision,actor:'合成の試験担当',reason:'実行前に観測名と対象を保存',action:{op:'configure_observation',check_id:'openemr',criterion:recipe.criterion}}));view=cli(['case','apply',caseDir,configure]);
const results=[];
for(const [label,fault] of [['normal','normal'],['database-stopped','database_stopped'],['retry','normal'],['database-stopped-recovered','database_stopped']]){
 const r=structuredClone(recipe);r.trial.fault=fault;if(label.endsWith('-recovered'))r.trial.recovery={ready_timeout_ms:180000};
 if(fault!=='normal'){
  const save=r.browser.actions.findIndex(s=>s.id==='save-soap');assert(save>0);
  r.browser.preparation.push(...r.browser.actions.splice(0,save));r.trial.checks=r.trial.checks.filter(c=>c.id.startsWith('soap-'));
 }
 const file=path.join(output,label+'-recipe.json');fs.writeFileSync(file,JSON.stringify(r,null,2));
 view=cli(['case','execute',caseDir,file,'--repository-root',repository,'--mode','local','--actor','合成の試験担当','--reason',label]);
 const e=view.workspace.evidence.at(-1),trace=e.execution,phase=trace.containers,browser=phase.browser;
 fs.writeFileSync(path.join(output,label+'.json'),JSON.stringify({verdict:view.checks[0].verdict,evidence:e},null,2));
 console.log(label,JSON.stringify({run:e.manifest.correlation.run_id,prepared:browser?.prepared,completed:browser?.completed,steps:browser?.steps,http:browser?.responses,reads:phase.database_reads,results:trace.trial.results}));
 assert.equal(browser.prepared,true);assert.deepEqual(phase.leftovers,[]);
 const checks=trace.trial.results;
 if(fault==='normal'){
  assert.equal(browser.completed,true);for(const c of checks)assert.equal(c.observed,'pass',label+' '+c.id);assert.equal(view.checks[0].verdict,'satisfied');
  assert(browser.responses.some(h=>h.watch==='visit-reload'&&h.request_step==='refresh-soap'&&h.status===200),'server reload after SOAP save');
  const read=id=>{const q=phase.database_reads.find(q=>q.id===id);assert.equal(q.performed,true);assert.equal(q.exit_code,0);return q.text.trim().split('\n').map(x=>x.split('\t'));};
  const patient=read('patient'),visit=read('visit'),soap=read('soap-identity');assert.equal(patient.length,1);assert.equal(visit.length,1);assert.equal(soap.length,1);
  assert.equal(soap[0][1],patient[0][2],'SOAP belongs to the synthetic patient');assert.equal(visit[0][1],patient[0][2]);
 }else{
  assert.equal(trace.trial.fault_effective,true);assert.equal(browser.completed,false);assert.notEqual(view.checks[0].verdict,'satisfied');
  for(const c of checks.filter(c=>c.id.endsWith('-row')||c.id.endsWith('-readback')))assert.equal(c.observed,'not_observed',c.id);
  assert(phase.database_reads.every(q=>!q.performed||q.exit_code!==0),'unavailable DB is not an empty row set');
  assert(browser.steps.some(s=>s.id==='save-soap'&&s.phase==='action'),'actually attempted SOAP save after stopping DB');
 }
 if(label.endsWith('-recovered')){
  // 故障中は読めなかった行を、DBを戻した後に読み直す。保存操作はDB停止中に失敗しているため、SOAPの行が増えていないことも確かめる。
  const recovery=trace.trial.recovery;assert.equal(recovery.attempted,true);assert.equal(recovery.effective,true,recovery.applied);
  assert(recovery.services.every(x=>x.ready),JSON.stringify(recovery.services));
  const change=id=>{const c=recovery.changes.find(c=>c.id===id&&c.source==='database_rows');assert(c,id);return c;};
  for(const id of ['patient','visit'])assert.equal(change(id).change,'readable_after_recovery',id),assert.equal(change(id).rows_after_recovery,1,id+' is not duplicated');
  assert.equal(change('soap-identity').rows_after_recovery,0,'the SOAP save made during the outage left no row');
 }
 results.push({label,evidence_id:e.id,run_id:e.manifest.correlation.run_id,verdict:view.checks[0].verdict});
}
const comparison=cli(['case','compare',caseDir,'--before',results[0].evidence_id,'--after',results[2].evidence_id,'--condition','openemr']);
fs.writeFileSync(path.join(output,'comparison.json'),JSON.stringify(comparison,null,2));
fs.writeFileSync(path.join(output,'summary.json'),JSON.stringify({source:recipe.source,results,recovery:'New disposable database and containers; not repair of an existing database.',human_evaluation:'not_conducted'},null,2));
console.log('SOAP fields, explicit server reload, patient identity, DB-stop fault and fresh retry passed.');
