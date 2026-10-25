import assert from 'node:assert/strict';
import { test } from 'node:test';
import { apiRequest, ApiError } from '../ui/api.js';

test('queued API replies retain errors and execution jobs keep their existing contract', async () => {
  const original=globalThis.fetch;
  try {
    const calls=[];
    const replies=[
      new Response(JSON.stringify({kind:'request',id:'1',status:'queued'}),{status:202}),
      new Response(JSON.stringify({kind:'request',id:'1',status:'completed',response_status:400,result:{error:'invalid source'}})),
    ];
    globalThis.fetch=async(...args)=>{calls.push(args);const reply=replies.shift();assert.ok(reply);return reply;};
    await assert.rejects(()=>apiRequest('POST','/api/v1/understand',{},'test'),error=>error instanceof ApiError && error.status===400 && error.message==='invalid source');
    assert.equal(calls[0][1].headers.Prefer,'respond-async');
    assert.equal(calls[1][0],'/api/v1/jobs/1');
    globalThis.fetch=async()=>new Response(JSON.stringify({status:'running',id:'execution'}),{status:202});
    assert.deepEqual(await apiRequest('POST','/api/v1/execute',{},'test'),{status:'running',id:'execution'});
    globalThis.fetch=async()=>new Response(JSON.stringify({kind:'request',status:'queued',id:'../case'}),{status:202});
    await assert.rejects(()=>apiRequest('POST','/api/v1/ai',{},'test'),/識別子が不正/);
  } finally {globalThis.fetch=original;}
});
