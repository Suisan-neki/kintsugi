// 架空の画面で準備・保存・HTTP・途中失敗を確かめる。実医療OSSの代替結果にはしない。
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {spawn} from 'node:child_process';
import {mkdtemp,readFile,writeFile,mkdir,stat,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {resolve,join} from 'node:path';
import {chromium} from 'playwright';
import {createHash} from 'node:crypto';
const root=resolve(import.meta.dirname,'..');
const temp=await mkdtemp(join(tmpdir(),'eel-browser-'));
const launched=await chromium.launch({headless:true,executablePath:process.env.EEL_BROWSER_EXECUTABLE||undefined,args:['--no-sandbox',...JSON.parse(process.env.EEL_BROWSER_EXTRA_ARGS||'[]')]});const version=launched.version();await launched.close();
let database=true,stored='',requests=[];
const server=createServer(async(req,res)=>{requests.push(req.url);
 if(req.url==='/save'){let body='';for await(const chunk of req)body+=chunk;if(database)stored=body;res.setHeader('Content-Type','application/json');res.end(JSON.stringify({message:'保存しました'}));return;}
 if(req.url==='/frame'){res.setHeader('Content-Type','multipart/related; boundary=eel');res.end(Buffer.concat([Buffer.from('--eel\r\nContent-Type: application/octet-stream\r\n\r\n'),Buffer.from([0,1,2,255]),Buffer.from('\r\n--eel--\r\n')]));return;}
 if(req.url==='/stored'){res.end(stored);return;}
 if(req.url==='/dismiss'||req.url==='/dismiss-stuck'){
  const stuck=req.url==='/dismiss-stuck';
  res.setHeader('Content-Type','text/html; charset=utf-8');
  // 最初のクリックが表示遷移中に届いて無視される場合を明示的に作る。
  // 2回目で実際に案内を閉じるか、閉じられない状態を維持する。
  res.end(`<!doctype html><html><div id="notice" role="dialog" style="position:fixed;inset:0;background:white;z-index:10"><button id="dismiss">後で</button></div><p id="attempts"></p><button id="proceed">次の操作</button><p id="result"></p><script>
   let attempts=0,transitionFinished=false;
   document.querySelector('#dismiss').onclick=()=>{
    attempts++;document.querySelector('#attempts').textContent=String(attempts);
    if(attempts===1){setTimeout(()=>transitionFinished=true,100);return;}
    if(transitionFinished&&!${stuck})document.querySelector('#notice').hidden=true;
   };
   document.querySelector('#proceed').onclick=async()=>{await fetch('/save',{method:'POST',body:'dismissed'});document.querySelector('#result').textContent='次の操作を完了した';};
  </script></html>`);return;
 }
 if(req.url!=='/'){res.writeHead(404);res.end();return;}
 res.setHeader('Content-Type','text/html; charset=utf-8');res.end(`<!doctype html><html lang="ja"><label>利用者<input id="user"></label><label>パスワード<input type="password" id="password"></label><button id="login" onclick="document.querySelector('#ready').hidden=false">ログイン</button><p id="ready" hidden>準備完了</p><button id="expand" onclick="document.querySelector('#note').hidden=!document.querySelector('#note').hidden">詳細</button><div id="note" hidden>表示する説明</div><label>試験値<input id="value"></label><button id="save">保存</button><p id="result"></p><canvas id="pixels" width="64" height="64"></canvas><div style="position:relative;display:inline-block"><canvas id="blank" width="64" height="64"></canvas><div style="position:absolute;inset:0;background:linear-gradient(black,white)">合成の重なり</div></div><script>const c=document.querySelector("#pixels").getContext("2d");const gradient=c.createLinearGradient(0,0,64,64);gradient.addColorStop(0,"black");gradient.addColorStop(1,"white");c.fillStyle=gradient;c.fillRect(0,0,64,64);document.querySelector('#save').onclick=async()=>{const r=await fetch('/save',{method:'POST',body:document.querySelector('#value').value});const body=await r.json();const node=document.querySelector('#result');node.textContent=body.message;node.dataset.done='true';fetch('/frame');fetch('http://external.invalid/blocked').catch(()=>{});};</script></html>`);
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));const origin=`http://127.0.0.1:${server.address().port}`;
const label=text=>({locator:{by:'label',text}}),css=selector=>({locator:{by:'css',selector}});
const input=(id,text,key)=>({id,action:'fill',target:label(text),value:{source:'secret',key}});
const spec={service:'web',tls:false,start_path:'/',expected_version:version,
 preparation:[input('user','利用者','user'),input('password','パスワード','password'),{id:'login',action:'click',target:css('#login')},{id:'ready',action:'read_text',target:css('#ready')},{id:'expand',action:'expand',target:css('#expand'),content:css('#note')},{id:'already-expanded',action:'expand',target:css('#expand'),content:css('#note')}],
 actions:[{id:'value',action:'fill',target:label('試験値'),value:{source:'literal',value:'{{test_value}}'}},{id:'save',action:'click',target:css('#save')},{id:'message',action:'read_text',target:css('#result[data-done=true]')},{id:'canvas',action:'read_canvas',target:css('#pixels'),min_levels:64,min_pixels:100}],
 responses:[{id:'save',path:'/save',method:'POST',contains:'保存しました'},{id:'frame',path:'/frame',method:'GET',payload:'dicom_frame'}],database:[]};
const secretsPath=join(temp,'secrets.json');await writeFile(secretsPath,JSON.stringify({user:'synthetic-operator-secret',password:'synthetic-password-secret'}),{mode:0o600});
const live=new Set();
async function waitFile(file,child){for(let i=0;i<1000;i++){if(await stat(file).then(()=>true,()=>false))return JSON.parse(await readFile(file,'utf8'));if(child.exitCode!==null)throw Error('runner stopped without '+file);await new Promise(r=>setTimeout(r,20));}throw Error('runner timed out');}
async function run(name,config,fault=false,timeout_ms=10000){const dir=join(temp,name);await mkdir(dir);const run_id='synthetic-'+name;
 await writeFile(join(dir,'request.json'),JSON.stringify({schema_version:1,spec:config,run_id,test_value:run_id,origin,origins:[origin],timeout_ms}));
 const child=spawn(process.execPath,[join(root,'tools/browser-runner.mjs'),join(dir,'request.json')],{env:{...process.env,EEL_PLAYWRIGHT_MODULE:join(root,'node_modules/playwright/index.mjs'),EEL_PNGJS_MODULE:join(root,'node_modules/pngjs/lib/png.js'),EEL_BROWSER_SECRETS_FILE:secretsPath},stdio:'pipe'});live.add(child);let stderr='';child.stderr.on('data',d=>stderr+=d);child.stdout.resume();
 const exit=new Promise(r=>child.once('exit',r));
 const ready=await waitFile(join(dir,'ready.json'),child);
 if(ready.prepared){assert.equal(requests.filter(p=>p==='/save').length,0);if(fault)database=false;await writeFile(join(dir,'continue.json'),JSON.stringify({run_id,proceed:true}));}
 const result=await waitFile(join(dir,'result.json'),child);assert.equal(await exit,0,stderr);live.delete(child);
 const json=JSON.stringify(result);assert.ok(!json.includes('synthetic-password-secret'));assert.ok(!json.includes('synthetic-operator-secret'));
 return result;
}
try {
 const success=await run('normal',spec);assert.equal(success.completed,true,JSON.stringify(success));assert.equal(stored,'synthetic-normal');assert.equal(success.responses[0].status,200);assert.equal(success.responses[0].body_match,true);assert.equal(success.responses[0].request_step,'save');assert.equal(success.steps.find(s=>s.id==='message').text,'保存しました');assert.ok(success.blocked_requests>0);assert.equal(success.steps.find(s=>s.id==='password').screenshot,null);assert.equal(success.steps.find(s=>s.id==='canvas').canvas.variation,true);assert.equal(success.responses.find(r=>r.watch==='frame').payload_sha256,createHash('sha256').update(Buffer.from([0,1,2,255])).digest('hex'));
 requests=[];stored='';const fault=await run('db-stopped',spec,true);assert.equal(fault.completed,true,JSON.stringify(fault));assert.equal(stored,'');assert.equal(fault.steps.find(s=>s.id==='message').text,'保存しました');
 requests=[];const blank=await run('blank-canvas',{...spec,actions:[{id:'blank',action:'read_canvas',target:css('#blank'),min_levels:64,min_pixels:100}]});assert.equal(blank.steps.find(s=>s.id==='blank').completed,true);assert.equal(blank.steps.find(s=>s.id==='blank').canvas.variation,false);
 requests=[];const broken=await run('missing-element',{...spec,preparation:[],actions:[{id:'missing',action:'read_text',target:css('#does-not-exist')},spec.actions[1]]},false,5000);assert.equal(broken.completed,false);assert.equal(broken.steps.length,1);assert.equal(broken.steps.at(-1).id,'missing');assert.equal(broken.steps.at(-1).completed,false);assert.equal(requests.filter(p=>p==='/save').length,0);
 assert.equal(broken.steps.at(-1).screenshot,'browser/action-missing.png');assert.match(broken.steps.at(-1).detail,/操作が未完了となった後/);
 const failedPhoto=await readFile(join(temp,'missing-element/action-missing.png'));assert(failedPhoto.subarray(0,8).equals(Buffer.from([137,80,78,71,13,10,26,10])));
 requests=[];const preparationFailure=await run('missing-preparation',{...spec,preparation:[spec.preparation[0],{id:'missing',action:'read_text',target:css('#does-not-exist')}],actions:spec.actions},false,5000);
 assert.equal(preparationFailure.prepared,false);assert.equal(preparationFailure.completed,false);assert.equal(preparationFailure.steps.at(-1).screenshot,'browser/preparation-missing.png');assert(preparationFailure.steps.every(s=>s.phase==='preparation'));assert.equal(requests.filter(p=>p==='/save').length,0);
 requests=[];const secretFailure=await run('missing-secret',{...spec,preparation:[{id:'password',action:'fill',target:css('#does-not-exist'),value:{source:'secret',key:'password'}}]},false,5000);
 assert.equal(secretFailure.prepared,false);assert.equal(secretFailure.steps.at(-1).completed,false);assert.equal(secretFailure.steps.at(-1).screenshot,null);
 requests=[];const wrongVersion=await run('wrong-version',{...spec,expected_version:'0.0.0'});assert.equal(wrongVersion.prepared,false);assert.equal(wrongVersion.completed,false);assert.equal(wrongVersion.steps.length,0);
 const dismissStep={id:'dismiss-notice',action:'dismiss',target:css('#dismiss'),content:css('#notice')};
 const dismissSpec={...spec,start_path:'/dismiss',preparation:[dismissStep,{id:'dismiss-attempts',action:'read_text',target:css('#attempts'),contains:'2'}],actions:[{id:'proceed',action:'click',target:css('#proceed')},{id:'proceeded',action:'read_text',target:css('#result'),contains:'次の操作を完了した'}],responses:[]};
 requests=[];const dismissed=await run('dismiss-transition',dismissSpec);assert.equal(dismissed.prepared,true,JSON.stringify(dismissed));assert.equal(dismissed.completed,true);assert.match(dismissed.steps[0].detail,/クリックを2回実行した.*非表示を確認した/);assert.equal(dismissed.steps[1].text,'2');assert.equal(requests.filter(p=>p==='/save').length,1);
 requests=[];const stuck=await run('dismiss-stuck',{...dismissSpec,start_path:'/dismiss-stuck'});assert.equal(stuck.prepared,false);assert.equal(stuck.completed,false);assert.equal(stuck.steps.length,1);assert.match(stuck.steps[0].detail,/クリックを3回実行した/);assert(!stuck.steps[0].detail.includes('非表示を確認した'));assert.equal(stuck.steps[0].screenshot,'browser/preparation-dismiss-notice.png');assert.equal(requests.filter(p=>p==='/save').length,0);
 requests=[];const unauthorized=await run('dismiss-action',{...dismissSpec,preparation:[],actions:[dismissStep]});assert.equal(unauthorized.completed,false);assert.match(unauthorized.steps[0].detail,/準備中だけ/);assert.equal(requests.filter(p=>p==='/save').length,0);
 console.log(JSON.stringify({version,checks:['preparation precedes fault','successful display with absent stored data','action HTTP attribution','secrets redacted','external request blocked','partial step retained','failure photo after action deadline','preparation failure does not start actions','secret fill failure has no photo','version mismatch rejected','canvas pixels exclude a gradient HTML overlay','dismissal confirms hidden after opening transition','only dismissal retries at most three times','failed dismissal prevents target actions','dismissal rejected outside preparation']}));
} finally {for(const child of live)child.kill('SIGKILL');await new Promise(r=>server.close(r));await rm(temp,{recursive:true,force:true});}
