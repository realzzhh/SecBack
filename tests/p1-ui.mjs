// 对编译后的 WebView 运行：CDP_PORT=9222 node tests/p1-ui.mjs。
// 仅拦截协议使用虚构数据；真实写盘回归由核心测试和隔离密库端到端验证负责。
import assert from 'node:assert/strict';
import {connectCDP} from './cdp.mjs';
const {cmd,run,close}=await connectCDP();
try {
const result = await run(`(async () => {
 const originalSend = send, originalState = S;
 const empty = () => ({user:'',remark:'',empty:true,hasPass:false});
 const account = user => ({user,remark:'',empty:false,hasPass:true});
 const p = {id:'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',name:'虚构回归项目',remark:'',slots:[account('A'),empty(),account('C')]};
 const messages = []; const out = {};
 const reset = () => { savingRequest = null; closeEditor(); messages.length = 0; };
 send = m => messages.push(m);
 try {
  $('app').hidden = false; $('login').hidden = true;
  S = {projects:[p],selected:p.id,canUp:false,canDown:false};
  openEditor(p);
  out.sparseSlots = $('m-slots').querySelectorAll('.slot-edit').length === 3;
  $('modal-save').click();
  out.pendingReadPreservesPassword = messages.at(-1).s2.pass === null && messages.at(-1).s2.source === 2;
  out.waitsForSaveResult = !$('overlay').hidden && $('modal-save').disabled;
  reset(); p.slots = [account('A'),account('B'),empty()];
  openEditor(p);
  const oldRequest = messages.find(m => m.slot === 1);
  $('m-slots').querySelector('[data-remove="0"]').click();
  onSecret({...oldRequest,pass:'synthetic-B'});
  out.movedPendingReadUsesSource = $('m-slots').querySelector('[data-slot="0"] [data-f="pass"]').value === 'synthetic-B';
  $('modal-save').click();
  out.movedAccountKeepsSource = messages.at(-1).s0.source === 1 && messages.at(-1).s0.pass === null;
  reset(); openEditor(p);
  const request = messages.find(m => m.slot === 0);
  $('m-slots').querySelector('[data-clear="0"]').click(); onSecret({...request,pass:'synthetic-A'});
  $('modal-save').click();
  out.clearRejectsLateRead = messages.at(-1).s0.pass === '';
  reset(); openEditor(p); const stale = messages.find(m => m.slot === 0);
  closeEditor(); openEditor(p); onSecret({...stale,pass:'stale-secret'});
  out.reopenRejectsStaleRead = $('m-slots').querySelector('[data-slot="0"] [data-f="pass"]').value === '';
  closeEditor(); messages.length = 0;
  document.activeElement?.blur(); document.dispatchEvent(new KeyboardEvent('keydown',{key:'Delete',bubbles:true}));
  out.deleteRequiresConfirmation = !$('confirm').hidden && !messages.some(m => m.t === 'delete');
  S.selected = 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'; $('confirm-ok').click();
  out.deleteCapturesTarget = messages.at(-1).id === p.id;
  S.selected = p.id; secrets[p.id+':0']='old-secret'; applySnapshot({projects:[p],selected:p.id});
  out.snapshotClearsSecrets = Object.keys(secrets).length === 0;
  $('recnew').hidden = false; messages.length = 0; handleBack();
  out.backCancelsRecovery = messages.some(m => m.t === 'recover_cancel') && $('recnew-1').value === '';
 } finally { reset(); send = originalSend; S = originalState; $('confirm').hidden = true; render(); }
 return out;
})()`);
for (const [name, value] of Object.entries(result)) assert.equal(value, true, name);
await cmd('Emulation.setDeviceMetricsOverride',{width:393,height:851,deviceScaleFactor:1,mobile:true});
const layout = await run(`(() => {openEditor(null); for(let i=0;i<2;i++) $('m-slots').querySelector('[data-add]').click(); const body=$('overlay').querySelector('.modal-body'); body.scrollTop=body.scrollHeight; const rect=$('modal-save').getBoundingClientRect(); const out={buttonsVisible:rect.top>=0&&rect.bottom<851, panels:[...$('overlay').querySelectorAll('.form-panel')].every(el=>getComputedStyle(el).borderTopWidth!=='0px')};closeEditor();return out;})()`);
assert.equal(layout.buttonsVisible,true); assert.equal(layout.panels,true);
console.log(JSON.stringify({...result,...layout},null,2)); } finally { close(); }
