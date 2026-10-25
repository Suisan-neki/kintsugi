// 基盤に組み込むブラウザ操作。対象リポジトリのスクリプトは読み込まない。
import fs from 'node:fs/promises';
import path from 'node:path';
import {pathToFileURL} from 'node:url';
import {createHash} from 'node:crypto';
const request=JSON.parse(await fs.readFile(process.argv[2],'utf8'));
const dir=path.dirname(process.argv[2]), spec=request.spec;
const report={schema_version:1,run_id:request.run_id,version:'',prepared:false,completed:false,steps:[],responses:[],blocked_requests:0,response_limit:false,detail:'',launch_args:['--use-gl=angle','--use-angle=swiftshader','--enable-unsafe-swiftshader']};
if(process.env.EEL_BROWSER_EXTRA_ARGS){const args=JSON.parse(process.env.EEL_BROWSER_EXTRA_ARGS);if(!Array.isArray(args)||args.length>16||args.some(s=>typeof s!=='string'||s.includes('\0')||['--no-sandbox','--disable-setuid-sandbox','--disable-web-security'].includes(s.split('=')[0])))throw Error('ブラウザーの追加引数が不正');report.launch_args.push(...args);}
let browser, page, secrets={}, masks=[], requests=new Map(), active=null, pending=new Set(), deadline=Date.now()+request.timeout_ms;
// 停止要求は操作待ちの間にもブラウザを閉じ、Playwrightが起動した子を回収する。
const closeBrowser=async()=>{if(browser)await browser.close().catch(()=>{});};
process.once('SIGTERM',()=>{void closeBrowser();});
const stopPoll=setInterval(()=>{void fs.stat(path.join(dir,'stop')).then(closeBrowser,()=>{});},50);stopPoll.unref();
const redact=text=>{let s=String(text);for(const value of Object.values(secrets).filter(v=>typeof v==='string'&&v.length)){s=s.split(value).join('[秘匿]');}return s.slice(0,8192);};
const replace=s=>s.replaceAll('{{test_value}}',request.test_value);
const save=async(name,data)=>{const temp=path.join(dir,name+'.tmp');await fs.writeFile(temp,JSON.stringify(data),{mode:0o600,flag:'wx'});await fs.rename(temp,path.join(dir,name));};
function target(t){let scope=page;for(const frame of t.frames||[])scope=scope.frameLocator(frame);const l=t.locator;return l.by==='label'?scope.getByLabel(l.text,{exact:true}):l.by==='role'?scope.getByRole(l.role,{name:l.name,exact:true}):scope.locator(l.selector);}
function remaining(){const n=deadline-Date.now();if(n<=0)throw Error('ブラウザ試験が待機上限に達した');return Math.min(n,15000);}
async function dismiss(step,record){
 // Bootstrap等の表示アニメーション中には閉じる要求が反映されない場合がある。
 // 再試行するのは宣言された準備用の閉じる操作だけ。保存等のclickには適用しない。
 const content=target(step.content),until=Date.now()+remaining();
 const time=()=>{const n=Math.min(remaining(),until-Date.now());if(n<=0)throw Error('案内を閉じる操作が待機上限に達した');return n;};
 await content.waitFor({state:'visible',timeout:time()});
 for(let attempt=1;attempt<=3;attempt++){
  if(attempt>1&&!await content.isVisible()){record.detail+='。案内の非表示を確認した';return;}
  record.detail=`案内を閉じるクリックの試行${attempt}回目`;
  await target(step.target).click({timeout:time()});
  record.detail=`案内を閉じるクリックを${attempt}回実行した`;
  try{await content.waitFor({state:'hidden',timeout:Math.min(750,time())});record.detail+='。案内の非表示を確認した';return;}
  catch(error){if(error.name!=='TimeoutError'||attempt===3)throw error;}
 }
}
async function screenshot(step,r,timeout,failed=false){
 if(step.action==='fill'&&step.value.source==='secret')return;
 try{
  const name=`${r.phase}-${step.id}.png`;
  await page.screenshot({path:path.join(dir,name),timeout,mask:[...masks,...page.frames().map(f=>f.locator('input[type=password]'))],animations:'disabled'});
  r.screenshot='browser/'+name;
  if(failed)r.detail+='。操作が未完了となった後の画面写真。保存の成功を示す記録とは別';
 }catch(error){r.detail+='。画面写真は未取得: '+redact(error.message);}
}
function singleFrame(kind,bytes){
 if(!kind.toLowerCase().startsWith('multipart/related'))throw Error('multipart/relatedではない');
 const boundary=kind.split(';').slice(1).map(x=>x.trim()).find(x=>/^boundary=/i.test(x))?.split('=').slice(1).join('=').replace(/^"|"$/g,'');
 if(!boundary||boundary.length>200||/[\r\n]/.test(boundary))throw Error('boundaryが不正');
 if(bytes.subarray(0,2).equals(Buffer.from('\r\n')))bytes=bytes.subarray(2);
 const opening=Buffer.from(`--${boundary}\r\n`);if(!bytes.subarray(0,opening.length).equals(opening))throw Error('multipart開始が不明');
 bytes=bytes.subarray(opening.length);const split=bytes.indexOf('\r\n\r\n');if(split<0||split>8192||!bytes.subarray(0,split).toString().toLowerCase().split('\r\n').some(x=>x.startsWith('content-type: application/octet-stream')))throw Error('frameのヘッダーが不正');
 const body=bytes.subarray(split+4),end=Buffer.from(`\r\n--${boundary}--`),index=body.indexOf(end);
 if(index<0||!['','\r\n'].includes(body.subarray(index+end.length).toString())||body.subarray(0,index).includes(Buffer.from(`\r\n--${boundary}\r\n`)))throw Error('単一frameの全部を読み出せない');
 return body.subarray(0,index);
}
async function steps(list,phase){
 for(const step of list){active=step.id;const r={id:step.id,phase,completed:false,text:null,screenshot:null,detail:''};report.steps.push(r);
  try{const options={timeout:remaining()};
   if(step.action==='goto'){const url=new URL(replace(step.path),request.origin);if(url.origin!==request.origin)throw Error('操作先のオリジンが不一致');await page.goto(url.href,{...options,waitUntil:'domcontentloaded'});}
   else if(step.action==='fill'){const value=step.value.source==='secret'?secrets[step.value.key]:replace(step.value.value);if(typeof value!=='string')throw Error('試験用の認証情報が未設定');const locator=target(step.target);if(step.value.source==='secret')masks.push(locator);await locator.fill(value,options);}
   else if(step.action==='click'){await target(step.target).click(options);}
   else if(step.action==='expand'){const content=target(step.content);await target(step.target).waitFor({...options,state:'visible'});if(!await content.isVisible())await target(step.target).click(options);await content.waitFor({...options,state:'visible'});}
   else if(step.action==='dismiss'){if(phase!=='preparation')throw Error('案内を閉じる操作は準備中だけ利用できる');await dismiss(step,r);}
   else if(step.action==='select'){await target(step.target).selectOption(replace(step.value),options);}
   else if(step.action==='read_text'){const base=target(step.target);const locator=step.contains?base.filter({hasText:replace(step.contains)}):base;await locator.waitFor({...options,state:'visible'});const text=await locator.innerText(options);if(text.length>8192)throw Error('表示の文字列が上限を超えた');r.text=redact(text);}
   else if(step.action==='read_canvas'){
    const {PNG}=await import(pathToFileURL(process.env.EEL_PNGJS_MODULE).href);
    const locator=target(step.target);await locator.waitFor({...options,state:'visible'});
    if(await locator.evaluate(e=>e.tagName)!=='CANVAS')throw Error('観測先がcanvasではない');
    const until=Date.now()+Math.max(1,Math.min(12000,remaining()-1000));let photo;
    do{
     const attribute='data-eel-canvas-probe',marker=createHash('sha256').update(request.run_id+step.id).digest('hex').slice(0,16),previous=await locator.getAttribute(attribute);
     await locator.evaluate((e,{attribute,marker})=>e.setAttribute(attribute,marker),{attribute,marker});
     try{photo=await locator.screenshot({timeout:remaining(),animations:'disabled',style:`* { visibility: hidden !important; } canvas[${attribute}="${marker}"] { visibility: visible !important; }`});}
     finally{await locator.evaluate((e,{attribute,previous})=>previous==null?e.removeAttribute(attribute):e.setAttribute(attribute,previous),{attribute,previous});}
     if(photo.length>4*1024*1024)throw Error('canvasの画面写真が上限を超えた');
     const png=PNG.sync.read(photo);if(png.width*png.height>2000000)throw Error('canvasの画素数が上限を超えた');
     const levels=new Set();let non_black_pixels=0;
     for(let i=0;i<png.data.length;i+=4){if(!png.data[i+3])continue;const y=Math.round(0.2126*png.data[i]+0.7152*png.data[i+1]+0.0722*png.data[i+2]);levels.add(y);if(y>2)non_black_pixels++;}
     r.canvas={measurement:'isolated_canvas_screenshot',width:png.width,height:png.height,levels:levels.size,non_black_pixels,rgba_sha256:createHash('sha256').update(png.data).digest('hex'),variation:levels.size>=step.min_levels&&non_black_pixels>=step.min_pixels};
     if(r.canvas.variation||Date.now()>=until)break;await new Promise(resolve=>setTimeout(resolve,300));
    }while(true);
    const name=`${phase}-${step.id}.png`;await fs.writeFile(path.join(dir,name),photo);r.screenshot='browser/'+name;
   }
   else throw Error('未対応の操作');
   r.completed=true;r.detail=(r.detail?r.detail+'。':'')+'操作を完了した。保存の成功を示す記録とは別';
   if(step.action!=='read_canvas')await screenshot(step,r,Math.max(1,Math.min(15000,deadline-Date.now())));
  }catch(error){
   r.detail=(r.detail?r.detail+'。':'')+redact(error.message);report.detail=`${phase}の手順 ${step.id} を完了できなかった`;
   // 操作の待機上限を使い切っていても、一度だけ短い上限で停止後の表示を残す。
   // 追加の操作や再試行はせず、写真の有無で未完了という結果を変えない。
   await screenshot(step,r,1000,true);return false;
  }
 }
 active=null;return true;
}
try{
 if(process.env.EEL_BROWSER_SECRETS_FILE)secrets=JSON.parse(await fs.readFile(process.env.EEL_BROWSER_SECRETS_FILE,'utf8'));
 const {chromium}=await import(pathToFileURL(process.env.EEL_PLAYWRIGHT_MODULE).href);
 browser=await chromium.launch({headless:true,chromiumSandbox:true,executablePath:process.env.EEL_BROWSER_EXECUTABLE||undefined,args:report.launch_args});report.version=browser.version();
 if(report.version!==spec.expected_version)throw Error('ブラウザの版が試験設定と異なる');
 const context=await browser.newContext({viewport:{width:1440,height:1000},ignoreHTTPSErrors:spec.tls,serviceWorkers:'block',acceptDownloads:false});
 await context.route('**/*',async route=>{let allowed=false;try{allowed=request.origins.includes(new URL(route.request().url()).origin);}catch{}if(allowed)await route.continue();else{report.blocked_requests++;await route.abort('blockedbyclient');}});
 await context.routeWebSocket('**/*',ws=>ws.close());
 page=await context.newPage();page.on('dialog',d=>d.accept());page.on('download',d=>d.cancel());
 page.on('request',r=>requests.set(r,active));
 page.on('requestfinished',r=>requests.delete(r));page.on('requestfailed',r=>requests.delete(r));
 page.on('response',r=>{
  const p=(async()=>{const request_step=requests.get(r.request());if(!spec.actions.some(s=>s.id===request_step))return;const u=new URL(r.url());if(!request.origins.includes(u.origin))return;for(const w of spec.responses){if(w.path!==u.pathname||w.method!==r.request().method())continue;if(report.responses.length>=200){report.response_limit=true;continue;}const record={watch:w.id,method:r.request().method(),path:u.pathname,status:r.status(),body_match:null,request_step,payload_sha256:null};report.responses.push(record);if(w.contains!=null||w.payload){try{const body=await r.body();if(body.length<=1024*1024){if(w.contains!=null)record.body_match=body.toString('utf8').includes(replace(w.contains));if(w.payload){const payload=w.payload==='dicom_frame'?singleFrame(r.headers()['content-type']||'',body):body;record.payload_sha256=createHash('sha256').update(payload).digest('hex');}}}catch{}}}
  })().catch(()=>{});pending.add(p);p.finally(()=>pending.delete(p));
 });
 await page.goto(new URL(spec.start_path,request.origin).href,{timeout:remaining(),waitUntil:'domcontentloaded'});
 report.prepared=await steps(spec.preparation,'preparation');await save('ready.json',report);
 if(report.prepared){while(true){if(await fs.stat(path.join(dir,'stop')).then(()=>true,()=>false))throw Error('停止を要求された');if(await fs.stat(path.join(dir,'continue.json')).then(()=>true,()=>false)){const control=JSON.parse(await fs.readFile(path.join(dir,'continue.json'),'utf8'));if(control.run_id!==report.run_id||control.proceed!==true)throw Error('継続する試行が一致しない');break;}remaining();await new Promise(r=>setTimeout(r,30));}
  // 準備中の通信と、故障後の操作で観測した通信を混ぜない。
  await Promise.all([...pending]);report.responses=[];requests.clear();report.completed=await steps(spec.actions,'action');
 }
 await Promise.race([Promise.all([...pending]),new Promise(r=>setTimeout(r,Math.min(2000,Math.max(1,deadline-Date.now()))))]);
}catch(error){report.detail=redact(error.message);}finally{clearInterval(stopPoll);await closeBrowser();if(!report.detail)report.detail=report.completed?'宣言した操作を完了した。画面・HTTP・DBの記録を別々に確認する':'操作を完了していない';
 if(!await fs.stat(path.join(dir,'ready.json')).then(()=>true,()=>false))await save('ready.json',report);
 await save('result.json',report);
}
