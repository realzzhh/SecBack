import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import {connectCDP} from './cdp.mjs';
const {cmd,run,close}=await connectCDP();
const pause=ms=>new Promise(r=>setTimeout(r,ms));
const results=[];
try {
 await run(`window.polishOriginal={send,S,secrets,query,wantDetail,platform:document.documentElement.dataset.platform,theme:document.documentElement.dataset.theme,stored:localStorage.getItem('secback.theme')};window.polishMessages=[];send=m=>polishMessages.push(m);`);
 for(const [width,height] of [[860,560],[1000,680],[1440,900],[393,851],[393,420]]){
  await cmd('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:1,mobile:width<701});
  for(const theme of ['dark','light']){
   await run(`(()=>{document.documentElement.dataset.platform=${JSON.stringify(width<701?'android':'windows')};document.documentElement.dataset.theme=${JSON.stringify(theme)};document.body.classList.remove('showing-detail');query='';const account={user:'demo@example.test',remark:'账户备注 '.repeat(600),hasPass:true,empty:false};S={projects:[{id:'a'.repeat(32),name:'桌面详情滚动验证',remark:'虚构项目',slots:[{...account},{...account},{...account}]}],selected:'a'.repeat(32)};secrets={};wantDetail=true;showMain();render();})();`);
   // Repeated synthetic cards stress overflow even on a tall phone; no host data is changed.
   await run(`for(let i=0;i<3;i++)$('slots').append(...[...$('slots').children].slice(0,3).map(el=>el.cloneNode(true)));`);
   await pause(100);
   const before=await run(`(()=>{const r=document.querySelector('.pane-actions').getBoundingClientRect(),head=document.querySelector('.pane-head').getBoundingClientRect();return {top:r.top,bottom:r.bottom,head:head.top};})()`);
   assert(before.top>0&&Math.abs(before.bottom-height)<=1,`actions at bottom ${width} ${theme} ${JSON.stringify(before)}`);
   const content=await run(`(()=>{const r=document.querySelector('.pane-content').getBoundingClientRect();return {x:(r.left+r.right)/2,y:(r.top+r.bottom)/2};})()`);
   for(let i=0;i<3;i++)await cmd('Input.dispatchMouseEvent',{type:'mouseWheel',x:content.x,y:content.y,deltaX:0,deltaY:3000});
   await pause(150);
   const after=await run(`(()=>{const r=document.querySelector('.pane-actions').getBoundingClientRect();return {top:r.top,bottom:r.bottom,scroll:document.querySelector('.pane-content').scrollTop,root:document.scrollingElement.scrollTop,head:document.querySelector('.pane-head').getBoundingClientRect().top};})()`);
   assert(after.scroll>0&&after.root===0&&Math.abs(after.top-before.top)<=1&&Math.abs(after.head-before.head)<=1,`wheel isolation ${width} ${theme} ${JSON.stringify(after)}`);
   const check=await run(`(async()=>{
    const out={},inside=e=>{const r=e.getBoundingClientRect();return r.top>=0&&r.left>=0&&r.bottom<=innerHeight+1&&r.right<=innerWidth+1;};
    const entry=i=>({id:i.toString(16).padStart(32,'0'),name:'外部虚构项目 '+i,action:'add',slots:[{user:'demo'+i,hasPass:true,srcSlot:0}]});
    showImportPreview({fileName:'synthetic.properties',summary:{added:50,slotsAdded:50},entries:Array.from({length:50},(_,i)=>entry(i))});
    await new Promise(r=>setTimeout(r,180));document.querySelector('#imp .modal-body').scrollTop=99999;
    out.importControls=!$('imp-x')&&!!$('imp-ok').querySelector('svg')&&inside($('imp-ok'))&&inside($('imp-cancel'));
    out.modalBlocksMenu=$('desktop-chrome').inert;
    polishMessages.length=0;$('imp-ok').querySelector('svg').dispatchEvent(new MouseEvent('click',{bubbles:true}));
    out.importRoute=polishMessages.some(m=>m.t==='import_commit')&&$('imp-ok').disabled;
    $('imp-ok').disabled=false;closeImport();await new Promise(r=>setTimeout(r,20));out.cancelClears=impData===null&&$('imp-tree').childElementCount===0&&polishMessages.at(-1).t==='import_cancel';
    out.menuVisibility=(getComputedStyle($('desktop-chrome')).display!=='none')===(innerWidth>700);
    if(innerWidth>700){
     $('menu-file').querySelector('svg').dispatchEvent(new MouseEvent('click',{bubbles:true}));out.menuBounds=inside($('menu-panel-file'));
     const key=(el,key)=>el.dispatchEvent(new KeyboardEvent('keydown',{key,bubbles:true,cancelable:true}));
     key($('menu-file'),'ArrowDown');out.menuFirst=document.activeElement.dataset.menuAction==='new';
     key(document.activeElement,'End');out.menuLast=document.activeElement.dataset.menuAction==='quit';
     key(document.activeElement,'ArrowRight');out.menuNext=desktopMenuOpen==='view'&&document.activeElement.dataset.menuAction==='theme-dark';
     const target=document.querySelector('[data-menu-action="theme-light"]');polishMessages.length=0;target.click();out.themeRoute=document.documentElement.dataset.theme==='light'&&polishMessages.some(m=>m.t==='theme_sync'&&!m.dark);
     out.themeCheck=document.querySelector('[data-menu-action="theme-light"]').getAttribute('aria-checked')==='true';
     $('menu-file').click();key($('menu-file'),'Escape');out.escapeRestores=desktopMenuOpen===null&&document.activeElement.id==='menu-file';
     $('menu-file').click();document.body.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true}));out.outsideCloses=desktopMenuOpen===null;
     showLogin();$('menu-file').click();out.lockedActions=[...document.querySelectorAll('#menu-panel-file [data-menu-action]')].every(e=>e.disabled===(e.dataset.menuAction!=='quit'));
     key($('menu-file'),'ArrowDown');out.lockedFocus=document.activeElement.dataset.menuAction==='quit';closeDesktopMenu();showMain();render();
    }
    return out;
   })()`);
   for(const [name,passed]of Object.entries(check))assert.equal(passed,true,`${width} ${theme}: ${name}`);
   results.push({width,height,theme,checks:Object.keys(check).length+2});
  }
 }
 console.log(JSON.stringify(results,null,2));
 if(process.env.UI_RESULTS)await fs.writeFile(process.env.UI_RESULTS,JSON.stringify(results,null,2));
}finally{
 await run(`(()=>{savingRequest=null;editorDirty=false;closeEditor();overlayEls.forEach(el=>el.hidden=true);closeDesktopMenu();const o=polishOriginal;send=o.send;S=o.S;secrets=o.secrets;query=o.query;wantDetail=o.wantDetail;document.documentElement.dataset.platform=o.platform;document.documentElement.dataset.theme=o.theme;if(o.stored===null)localStorage.removeItem('secback.theme');else localStorage.setItem('secback.theme',o.stored);render();delete window.polishOriginal;delete window.polishMessages;})();`);
 close();
}
