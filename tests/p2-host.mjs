// Runs only against an explicitly selected disposable vault in a desktop test instance.
import assert from 'node:assert/strict';
import {readFileSync, mkdirSync, rmdirSync} from 'node:fs';
import path from 'node:path';
import {connectCDP} from './cdp.mjs';
const vault=process.env.SECBACK_TEST_VAULT;
assert(vault && process.env.SECBACK_TEST_PASSWORD,'Set SECBACK_TEST_VAULT and SECBACK_TEST_PASSWORD for a disposable test instance');
const c=await connectCDP();
const invoke=m=>c.run(`window.__TAURI__.core.invoke('host_message',{json:${JSON.stringify(JSON.stringify(m))}})`);
async function wait(expr){for(let i=0;i<300;i++){if(await c.run(expr))return;await new Promise(r=>setTimeout(r,100));}throw Error('timeout '+expr);}
const out={},pwd=process.env.SECBACK_TEST_PASSWORD,empty={user:'',pass:'',remark:''};
try {
 await c.run(`(async()=>{window.__p2events=[];window.__p2unlisten=await window.__TAURI__.event.listen('host',e=>window.__p2events.push(typeof e.payload==='string'?JSON.parse(e.payload):e.payload));})()`);
 await invoke({t:'ready'});
 await wait(`window.__p2events.some(e=>e.t==='bootstrap')`);
 const actual=await c.run(`window.__p2events.find(e=>e.t==='bootstrap').vaultPath`);
 assert.equal(path.resolve(actual).toLowerCase(),path.resolve(vault).toLowerCase(),'Refusing to test a different vault');
 await invoke({t:'lock'});
 for(const msg of [{t:'unknown'},{t:'reveal',id:'é'.repeat(16),slot:0},{t:'reveal',id:'0'.repeat(32),slot:256}]) {
  await assert.rejects(invoke(msg));
 }
 out.invalidIpcRejected=true;
 await invoke({t:'recover_set_password',new:'synthetic-new-password'});
 out.recoveryStateGated=await c.run(`$('recnew').hidden&&$('app').hidden`);
 await invoke({t:'login',pwd});await wait(`!$('app').hidden`);
 out.loginPasswordCleared=await c.run(`$('login-pwd').value===''`);
 await invoke({t:'create',name:'P2 E2E sparse',remark:'',s0:{user:'A',pass:'secret-A',remark:''},s1:empty,s2:{user:'C',pass:'secret-C',remark:''},request:'p2-create'});
 await wait(`S.projects.some(p=>p.name==='P2 E2E sparse')`);
 const id=await c.run('S.selected');
 await c.run(`openEditor(projectById(S.selected));$('modal-save').click()`);await wait(`$('overlay').hidden`);
 await invoke({t:'lock'});await invoke({t:'login',pwd});await wait(`!$('app').hidden`);
 out.sparseSurvivesReload=await c.run(`S.projects.find(p=>p.id==='${id}').slots[2].hasPass`);
 await invoke({t:'select',id});
 await invoke({t:'save',id,name:'P2 E2E shifted',remark:'',s0:{user:'C',pass:null,source:2,remark:''},s1:empty,s2:empty,request:'p2-shift'});
 await c.run(`openEditor(projectById(S.selected))`);await wait(`$('m-slots').querySelector('[data-f="pass"]').value==='secret-C'`);
 out.shiftedPasswordCorrect=true;await c.run('closeEditor()');
 const before=readFileSync(vault),block=vault+'.tmp';mkdirSync(block);
 try {
  await c.run(`openEditor(projectById(S.selected));$('m-name').value='P2 FAILED DRAFT';$('modal-save').click()`);
  await wait(`!$('modal-save').disabled`);
  out.failedSaveKeepsDraft=await c.run(`!$('overlay').hidden&&$('m-name').value==='P2 FAILED DRAFT'`);
  out.failedSaveKeepsSnapshot=await c.run(`S.projects.find(p=>p.id==='${id}').name==='P2 E2E shifted'`);
  out.failedSaveKeepsDisk=readFileSync(vault).equals(before);
 }finally{rmdirSync(block);}
 await c.run(`$('modal-save').click()`);await wait(`$('overlay').hidden`);
 out.retrySaveSucceeds=await c.run(`S.projects.find(p=>p.id==='${id}').name==='P2 FAILED DRAFT'`);
 await c.run(`window.__p2events=[]`);await invoke({t:'select',id});
 out.selectionIsSmallEvent=await c.run(`window.__p2events.some(e=>e.t==='selection')&&!window.__p2events.some(e=>e.t==='snapshot')`);
 await c.run(`openEditor(projectById(S.selected));$('sheet').hidden=false;$('reccode-text').textContent='synthetic-code';$('imp-tree').textContent='synthetic-import';$('pwd-old').value='synthetic-password';secrets[S.selected+':0']='synthetic-secret';renderDetail()`);
 await invoke({t:'lock'});await wait(`!$('login').hidden`);
 out.lockClearsSensitiveDom=await c.run(`$('slots').children.length===0&&$('m-slots').children.length===0&&$('reccode-text').textContent===''&&$('imp-tree').textContent===''&&[...document.querySelectorAll('input,textarea')].every(el=>el.value==='')&&[...document.querySelectorAll('.overlay')].every(el=>el.hidden)`);
 for(let i=0;i<5;i++)await invoke({t:'login',pwd:'synthetic-wrong-password'});
 const cooldown=await c.run(`window.__p2events.filter(e=>e.t==='loginError').at(-1).retryAfter`);
 await c.run(`window.__p2events=[]`);await invoke({t:'login',pwd});
 out.hostEnforcesCooldown=cooldown>0&&await c.run(`$('app').hidden&&window.__p2events.some(e=>e.t==='loginError'&&e.retryAfter>0)`);
 for(const [k,v]of Object.entries(out))assert.equal(v,true,k);
 console.log(JSON.stringify(out,null,2));
} finally {
 await c.run(`window.__p2unlisten?.();delete window.__p2unlisten;delete window.__p2events;`).catch(()=>{});
 c.close();
}
