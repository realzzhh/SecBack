import assert from 'node:assert/strict';
import {connectCDP} from './cdp.mjs';
const {cmd,run,close}=await connectCDP();
try {
 const out=await run(`(async()=>{
 const originalSend=send, originalS=S;const messages=[],out={};send=m=>messages.push(m);
 const wait=ms=>new Promise(r=>setTimeout(r,ms));
 const empty=()=>({user:'',remark:'',empty:true,hasPass:false});
 const p={id:'a'.repeat(32),name:'虚构项目',remark:'',slots:[{user:'A',remark:'',empty:false,hasPass:true},empty(),empty()]};
 try{
  $('app').hidden=false;$('login').hidden=true;S={projects:[p],selected:p.id,canUp:false,canDown:false};query='';render();
  openEditor(p);const pass=$('m-slots').querySelector('[data-f="pass"]');out.passwordMasked=pass.type==='password';
  const show=$('m-slots').querySelector('[data-visibility]');show.click();out.passwordCanShow=pass.type==='text';show.click();
  $('m-name').value='未保存';$('m-name').dispatchEvent(new Event('input',{bubbles:true}));$('modal-cancel').click();out.dirtyNeedsChoice=!$('discard').hidden&&!$('overlay').hidden;
  $('discard-cancel').click();out.keepDraft=$('m-name').value==='未保存'&&!$('overlay').hidden;
  requestCloseEditor();$('discard-ok').click();out.discardClears=$('overlay').hidden&&$('m-name').value==='';
  query='不存在';renderList();out.searchEmpty=!$('list-empty').hidden&&$('list-empty').textContent.includes('没有匹配');query='';renderList();
  out.listAccessible=$('list').getAttribute('role')==='listbox'&&!!$('list').querySelector('[role=option][tabindex="0"]');
  const first=$('list').firstElementChild;selectProject(p.id);out.selectionKeepsRows=$('list').firstElementChild===first;
  let backHandler, exits=0;await registerBackButton({app:{onBackButtonPress:async handler=>{backHandler=handler;},exit:async code=>{if(code===0)exits++;}}});
  await backHandler();out.nativeBackReturnsToList=!document.body.classList.contains('showing-detail')&&exits===0;
  await backHandler();out.nativeBackExitsFromList=exits===1;
  $('app').hidden=true;showLogin();firstRun=false;loginPending=false;loginDeadline=0;$('login-pwd').value='synthetic';messages.length=0;doLogin();doLogin();out.loginSingleFlight=messages.filter(m=>m.t==='login').length===1;
  loginPending=false;loginDeadline=Date.now()+5000;doLogin();out.enterCannotBypassCooldown=messages.filter(m=>m.t==='login').length===1;loginDeadline=0;loginPending=false;
  $('app').hidden=false;$('login').hidden=true;
  $('ipwd').hidden=false;await wait(30);$('ipwd').hidden=true;
  showImportPreview({fileName:'synthetic.properties',entries:[{id:p.id,name:'外部',localName:'本机',metadataConflict:true,localRemark:'旧',incomingRemark:'新',action:'merge',slots:[]}],summary:{}});
  await wait(100);out.importTransitionStaysOpen=!$('imp').hidden&&impData!==null;
  const meta=$('imp-tree').querySelector('[data-metadata]');meta.checked=true;meta.dispatchEvent(new Event('change',{bubbles:true}));out.metadataChoice=messages.at(-1).useIncomingMetadata[0]===p.id&&messages.at(-1).t==='import_plan';
  $('imp-ok').click();out.importWaits=!$('imp').hidden&&$('imp-ok').disabled;closeImport();$('imp-ok').disabled=false;$('imp-tree').inert=false;
  await wait(30);$('pwd').hidden=false;await wait(30);const controls=[...$('pwd').querySelectorAll('button,input')].filter(el=>!el.disabled&&el.getClientRects().length);controls.at(-1).focus();document.dispatchEvent(new KeyboardEvent('keydown',{key:'Tab',bubbles:true,cancelable:true}));out.focusTrapped=$('pwd').contains(document.activeElement);
  $('pwd-cancel').click();await wait(30);out.backgroundReleased=!$('app').inert;
  messages.length=0;query='';renderList();const item=$('list').firstElementChild;
  const touch=(type,y)=>{const ev=new Event(type,{bubbles:true,cancelable:true});Object.defineProperty(ev,'touches',{value:type==='touchstart'?[{clientY:y}]:[]});item.dispatchEvent(ev);};
  touch('touchstart',100);await wait(1050);touch('touchcancel',100);out.touchCancelDoesNotCommit=!messages.some(m=>['reorder','select'].includes(m.t));
  query='query';touch('touchstart',100);await wait(1050);out.filteredDragDisabled=!item.classList.contains('dragging');touch('touchcancel',100);query='';
  out.zoomAllowed=!document.querySelector('meta[name=viewport]').content.includes('user-scalable=no');
 }finally{savingRequest=null;closeEditor();$('discard').hidden=true;overlayEls.forEach(el=>el.hidden=true);send=originalSend;S=originalS;loginPending=false;loginDeadline=0;query='';render();}
 return out;})()`);
 for(const [k,v] of Object.entries(out))assert.equal(v,true,k);
 for(const [width,height] of [[393,851],[393,420],[800,600]]){
  await cmd('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:1,mobile:true});
  await run('new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))');
  const layout=await run(`(()=>{
   $('app').hidden=false;openEditor(null);for(let i=0;i<2;i++)$('m-slots').querySelector('[data-add]').click();const body=$('overlay').querySelector('.modal-body');body.scrollTop=body.scrollHeight;const r=$('modal-save').getBoundingClientRect();const editor=r.top>=0&&r.bottom<=innerHeight;closeEditor();
   $('pwd').hidden=false;const m=$('pwd').querySelector('.modal'),foot=m.querySelector('.modal-foot');m.querySelector('.modal-body').scrollTop=9999;const f=foot.getBoundingClientRect();const other=f.top>=0&&f.bottom<=innerHeight+1&&getComputedStyle(foot).position!=='sticky';$('pwd').hidden=true;
   showImportPreview({fileName:'synthetic.properties',entries:[],summary:{}});
   const inside=id=>{const b=$(id).getBoundingClientRect();return b.left>=0&&b.right<=innerWidth+1&&b.top>=0&&b.bottom<=innerHeight+1;};
   const importActions=!$('imp-x')&&['imp-cancel','imp-ok'].every(inside);$('imp').hidden=true;impData=null;
   return {editor,other,importActions,more:innerWidth<=700||getComputedStyle($('btn-more')).display!=='none'||getComputedStyle($('desktop-chrome')).display!=='none'};
  })()`);
  for(const [k,v]of Object.entries(layout))assert.equal(v,true,width+'x'+height+': '+k);
 }
 console.log(JSON.stringify(out,null,2));console.log('mobile/narrow keyboard/wide layouts: passed');
}finally{close();}
