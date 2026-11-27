// Compiled WebView regression: CDP_PORT=9223 node tests/mobile-scroll-ui.mjs.
// All project data and host messages are synthetic and restored after the test.
import assert from 'node:assert/strict';
import {connectCDP} from './cdp.mjs';
const {cmd,run,close}=await connectCDP();
const pause=ms=>new Promise(resolve=>setTimeout(resolve,ms));
const touch=(type,x,y)=>cmd('Input.dispatchTouchEvent',{type,touchPoints:type==='touchEnd'||type==='touchCancel'?[]:[{x,y}]});
try {
 await cmd('Emulation.setTouchEmulationEnabled',{enabled:true});
 await run(`window.scrollTest={send,S,query};window.scrollMessages=[];send=m=>scrollMessages.push(m);$('login').hidden=true;$('app').hidden=false;`);
 for (const [width,height] of [[393,851],[393,420],[800,600]]) {
  await cmd('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:1,mobile:true});
  await run(`(()=>{document.body.classList.remove('showing-detail');query='';const empty=()=>({user:'',remark:'',empty:true,hasPass:false});S={projects:Array.from({length:32},(_,i)=>({id:i.toString(16).padStart(32,'0'),name:'虚构滚动回归 '+i,remark:'虚构备注',slots:[{user:'Synthetic '+i,remark:'备注 '.repeat(300),empty:false,hasPass:true},empty(),empty()]})),selected:null};render();$('list').scrollTop=0;})()`);
  await pause(200);
  const bounds=await run(`(()=>{const r=$('list').getBoundingClientRect();return {x:r.left+60,top:r.top,bottom:r.bottom,height:innerHeight};})()`);
  // Real CDP long press and held touch, including both list edges.
  for (const direction of ['down','up']) {
   await run(`$('list').scrollTop=${direction==='down'?'0':"$('list').scrollHeight"};scrollMessages.length=0;`);
   await pause(150);
   const row=await run(`(()=>{const rows=[...$('list').children].filter(el=>{const r=el.getBoundingClientRect();return r.top>=${bounds.top}+1&&r.bottom<=${bounds.bottom}-1});const r=rows[${direction==='down'?'0':'rows.length-1'}].getBoundingClientRect();return {x:r.left+60,y:r.top+r.height/2,id:rows[${direction==='down'?'0':'rows.length-1'}].dataset.id};})()`);
   await touch('touchStart',row.x,row.y);await pause(1150);
   assert.equal(await run(`$('list').classList.contains('sorting')`),true,'long press starts sorting');
   const initial=await run(`({scroll:$('list').scrollHeight,client:$('list').clientHeight})`);
   const endY=direction==='down'?Math.min(height-5,bounds.bottom+25):Math.max(2,bounds.top-25);
   for(let step=1;step<=8;step++){await touch('touchMove',row.x,row.y+(endY-row.y)*step/8);await pause(30);}
   await pause(6500);
   for(let n=0;n<3;n++){
    await touch('touchMove',row.x,endY);await pause(250);
    const state=await run(`(()=>{const list=$('list'),drag=list.querySelector('.dragging'),r=drag.getBoundingClientRect(),b=list.getBoundingClientRect();return {height:list.scrollHeight,scroll:list.scrollTop,client:list.clientHeight,dragTop:r.top,dragBottom:r.bottom,top:b.top,bottom:b.bottom,root:document.scrollingElement.scrollTop,app:$('app').getBoundingClientRect().bottom,bar:$('tabbar').getBoundingClientRect().bottom};})()`);
    assert(Math.abs(state.height-initial.scroll)<=1,`${direction}: scrollHeight remains bounded ${JSON.stringify(state)}`);
    assert(state.scroll>=0&&state.scroll<=initial.scroll-initial.client+1,'scrollTop bounded');
    assert.equal(state.root,0,'document stays stationary');
    if(width<=700)assert(Math.abs(state.bar-height)<=1,'list bottom bar stays in viewport');
    if(direction==='down'){assert(state.dragBottom<=state.bottom+1,'drag stops at last row');assert(Math.abs(state.scroll-(initial.scroll-initial.client))<=1,'can auto-scroll to bottom');}
    else {assert(state.dragTop>=state.top-1,'drag stops at first row');assert.equal(state.scroll,0,'can auto-scroll to top');}
   }
   await touch('touchEnd');await pause(450);
   const message=await run(`scrollMessages.find(m=>m.t==='reorder')`);
   assert.equal(message?.id,row.id);assert.equal(message?.to,direction==='down'?31:0);
   assert.equal(await run(`$('list').querySelectorAll('.dragging,.shifting').length`),0,'drag cleanup');
  }
  // Repeated scroll gestures cannot move either navigation bar off screen.
  for(let n=0;n<4;n++){await touch('touchStart',bounds.x,bounds.bottom-30);await touch('touchMove',bounds.x,bounds.top+25);await touch('touchEnd');await pause(80);}
  await run(`selectProject(S.projects[0].id);`);await pause(150);
  if(width<=700){
   for(let n=0;n<5;n++){
    await run(`document.querySelector('.pane-content').scrollTop=999999;document.querySelector('.detail').scrollTop=999999;window.scrollTo(0,999999);`);
    const rect=await run(`(()=>{const b=$('btn-edit').getBoundingClientRect(),d=$('btn-delete').getBoundingClientRect(),c=document.querySelector('.pane-content');return {top:b.top,bottom:b.bottom,deleteBottom:d.bottom,scroll:c.scrollTop,max:c.scrollHeight-c.clientHeight,root:document.scrollingElement.scrollTop};})()`);
    assert(rect.top>=0&&rect.bottom<=height&&rect.deleteBottom<=height,'detail actions stay visible');assert.equal(rect.root,0);assert(Math.abs(rect.scroll-rect.max)<=1,'last detail content remains reachable');
   }
  }
  const editor=await run(`(async()=>{openEditor(S.projects[0]);await Promise.all($('overlay').querySelector('.modal').getAnimations().map(a=>a.finished));await new Promise(r=>setTimeout(r,100));const p=$('m-slots').querySelector('[data-f="pass"]'),b=p.parentElement.querySelector('[data-visibility]'),lock=p.parentElement.querySelector(':scope > svg');p.value='synthetic-password';const r=p.getBoundingClientRect(),i=lock.getBoundingClientRect(),e=b.getBoundingClientRect();const out={centered:Math.abs((i.top+i.bottom-r.top-r.bottom)/2)<1,eyeInside:e.left>=r.left&&e.right<=r.right&&e.top>=r.top&&e.bottom<=r.bottom,touchTarget:e.width>=43.99&&e.height>=43.99,masked:p.type==='password'};b.click();out.show=p.type==='text'&&b.getAttribute('aria-label')==='隐藏密码'&&b.getAttribute('aria-pressed')==='true';b.click();out.hide=p.type==='password'&&b.getAttribute('aria-label')==='显示密码'&&b.getAttribute('aria-pressed')==='false';b.click();Object.defineProperty(document,'hidden',{configurable:true,value:true});document.dispatchEvent(new Event('visibilitychange'));delete document.hidden;out.backgroundMasks=p.type==='password'&&b.getAttribute('aria-pressed')==='false'&&b.getAttribute('aria-label')==='显示密码';const body=$('overlay').querySelector('.modal-body');body.scrollTop=999999;const save=$('modal-save').getBoundingClientRect();out.saveVisible=save.top>=0&&save.bottom<=innerHeight;editorDirty=false;closeEditor();return out;})()`);
  for(const [key,value]of Object.entries(editor))assert.equal(value,true,`${width}x${height}: ${key}`);
  console.log(`${width}x${height}: drag boundaries, auto-scroll, bars, password icons passed`);
 }
} finally {
 try {await touch('touchCancel').catch(()=>{});await run(`if(window.scrollTest){savingRequest=null;editorDirty=false;closeEditor();send=scrollTest.send;S=scrollTest.S;query=scrollTest.query;document.body.classList.remove('showing-detail');render();delete window.scrollTest;delete window.scrollMessages;}`);}finally{close();}
}
