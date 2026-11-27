import {connectCDP} from './cdp.mjs';
import assert from 'node:assert/strict';
const vault=process.env.SECBACK_TEST_VAULT, pwd=process.env.SECBACK_TEST_PASSWORD, file=process.env.SECBACK_IMPORT_FIXTURE;
assert(vault && pwd && file,'Set disposable SECBACK_TEST_VAULT, SECBACK_TEST_PASSWORD, SECBACK_IMPORT_FIXTURE');
const c=await connectCDP(),r={};
const invoke=m=>c.run(`window.__TAURI__.core.invoke('host_message',{json:${JSON.stringify(JSON.stringify(m))}})`);
async function wait(expr){for(let i=0;i<200;i++){if(await c.run(expr))return;await new Promise(r=>setTimeout(r,100));}throw Error('Timeout '+expr);}
try {
 await c.run(`(async()=>{window.__importBootstrap=null;window.__importUnlisten=await window.__TAURI__.event.listen('host',e=>{const m=typeof e.payload==='string'?JSON.parse(e.payload):e.payload;if(m.t==='bootstrap')window.__importBootstrap=m;});})()`);
 await invoke({t:'ready'});await wait('window.__importBootstrap');
 const {resolve}=await import('node:path');
 assert.equal(resolve(await c.run('window.__importBootstrap.vaultPath')).toLowerCase(),resolve(vault).toLowerCase());
 await invoke({t:'login',pwd});
 await wait(`!$('app').hidden`);
 const count=await c.run('S.projects.length');
 await invoke({t:'import_path',path:file});await wait(`!$('ipwd').hidden`);
 await invoke({t:'import_unlock',pwd:'wrong'});await wait(`$('ipwd-err').textContent.includes('密码')`);r.wrongPasswordRetry=true;
 await invoke({t:'import_unlock',pwd:'synthetic-import-password'});await wait(`!$('imp').hidden`);
 r.preview=await c.run(`$('imp-tree').textContent.includes('Android-URI-import-test')`);
 await c.run(`$('imp-cancel').click()`);r.cancelKeepsCount=(await c.run('S.projects.length'))===count;
 await invoke({t:'import_path',path:file});await wait(`!$('ipwd').hidden`);
 await invoke({t:'import_unlock',pwd:'synthetic-import-password'});await wait(`!$('imp').hidden`);
 await c.run(`$('imp-ok').click()`);await wait(`S.projects.some(p=>p.name==='Android-URI-import-test')`);r.committed=true;
 await invoke({t:'lock'});await invoke({t:'login',pwd});await wait(`!$('app').hidden`);
 r.persisted=await c.run(`S.projects.some(p=>p.name==='Android-URI-import-test')`);
 for(const [k,v]of Object.entries(r))assert.equal(v,true,k);
 console.log(JSON.stringify(r,null,2));
} finally {await c.run('window.__importUnlisten?.();delete window.__importUnlisten;delete window.__importBootstrap;').catch(()=>{});c.close();}
