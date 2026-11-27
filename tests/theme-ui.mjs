// Compiled-host regression for icons, secure actions, login state and theme contrast.
import assert from 'node:assert/strict';
import {connectCDP} from './cdp.mjs';
const {cmd,run,close}=await connectCDP();
try {
 const results=[];
 for(const [width,height] of [[393,851],[393,420],[1000,680]]){
  await cmd('Emulation.setDeviceMetricsOverride',{width,height,deviceScaleFactor:1,mobile:true});
  for(const theme of ['dark','light']){
   const checks=await run(`(async()=>{
    const original={send,S,secrets,firstRun,query,wantDetail,theme:document.documentElement.dataset.theme};const messages=[],out={};send=m=>messages.push(m);
    const shown=e=>getComputedStyle(e).display!=='none';
    const inside=(child,parent)=>{const a=child.getBoundingClientRect(),b=parent.getBoundingClientRect();return a.left>=b.left-1&&a.right<=b.right+1&&a.top>=b.top-1&&a.bottom<=b.bottom+1;};
    const contrast=e=>{const c=getComputedStyle(e);const lum=color=>{const v=color.match(/[\\d.]+/g).slice(0,3).map(Number).map(x=>{x/=255;return x<=.04045?x/12.92:((x+.055)/1.055)**2.4;});return v[0]*.2126+v[1]*.7152+v[2]*.0722;};const a=lum(c.color),b=lum(c.backgroundColor);return (Math.max(a,b)+.05)/(Math.min(a,b)+.05);};
    try{
     document.documentElement.dataset.theme=${JSON.stringify(theme)};firstRun=false;loginPending=false;loginDeadline=0;showLogin();await new Promise(r=>setTimeout(r,300));
     const login=$('login-pwd'),eye=$('login-visible');login.value='synthetic';
     out.loginIconInside=inside(eye,login)&&eye.getBoundingClientRect().width>=44;
     out.loginDefaultClosed=login.type==='password'&&shown(eye.querySelector('.eye-closed'))&&!shown(eye.querySelector('.eye-open'));
     eye.querySelector('svg').dispatchEvent(new MouseEvent('click',{bubbles:true}));
     out.loginCanShow=login.type==='text'&&shown(eye.querySelector('.eye-open'))&&eye.getAttribute('aria-label')==='隐藏主密码';
     firstRun=true;showLogin();out.reopenMasks=login.type==='password'&&eye.getAttribute('aria-pressed')==='false';
     out.createKeepsIcon=!!$('login-go').querySelector('svg')&&$('login-go').textContent==='创建密库';
     eye.click();out.confirmFollows=$('login-confirm').type==='text';eye.click();
     firstRun=false;showLogin();login.value='synthetic';messages.length=0;$('login-go').querySelector('svg').dispatchEvent(new MouseEvent('click',{bubbles:true}));out.enterForwards=messages.length===1&&messages[0].t==='login'&&messages[0].pwd==='synthetic';loginPending=false;$('login-go').disabled=false;
     out.primaryContrast=contrast($('login-go'))>=4.5;
     $('login').hidden=true;$('app').hidden=false;const account={user:'synthetic-user',remark:'synthetic remark',empty:false,hasPass:true};S={projects:[{id:'d'.repeat(32),name:'图标回归',remark:'虚构项目',slots:[account,{empty:true},{empty:true}]}],selected:'d'.repeat(32)};secrets={};query='';wantDetail=true;render();
     const copyUser=document.querySelector('[data-act="copy-user"]'),copyPass=document.querySelector('[data-act="copy-pass"]');messages.length=0;copyUser.querySelector('svg').dispatchEvent(new MouseEvent('click',{bubbles:true}));copyPass.querySelector('svg').dispatchEvent(new MouseEvent('click',{bubbles:true}));
     out.copyRoutes=messages.length===2&&messages[0].t==='copy'&&messages[0].field==='user'&&messages[1].field==='pass'&&!messages.some(m=>m.t==='reveal');
     out.detailHasOnlyIcons=[copyUser,copyPass,document.querySelector('[data-act="toggle-pass"]')].every(e=>!!e.querySelector('svg')&&e.textContent.trim()===''&&!!e.getAttribute('aria-label'));
     messages.length=0;document.querySelector('[data-act="toggle-pass"] svg').dispatchEvent(new MouseEvent('click',{bubbles:true}));const request=messages.find(m=>m.t==='reveal');onSecret({...request,pass:'synthetic-secret'});let toggle=document.querySelector('[data-act="toggle-pass"]');out.detailOpen=toggle.getAttribute('aria-pressed')==='true'&&shown(toggle.querySelector('.eye-open'));
     toggle.querySelector('svg').dispatchEvent(new MouseEvent('click',{bubbles:true}));toggle=document.querySelector('[data-act="toggle-pass"]');out.detailClosed=toggle.getAttribute('aria-pressed')==='false'&&!shown(toggle.querySelector('.eye-open'))&&shown(toggle.querySelector('.eye-closed'));
     out.detailTouch=innerWidth>700||[...document.querySelectorAll('.icon-action')].every(e=>e.getBoundingClientRect().width>=44&&e.getBoundingClientRect().height>=44);
     out.textIcons=['btn-edit','btn-delete','modal-save','modal-cancel','confirm-ok','confirm-cancel'].every(id=>!!$(id).querySelector('svg')&&!!$(id).textContent.trim());
     out.detailContrast=contrast($('btn-edit'))>=4.5&&contrast($('btn-delete'))>=4.5;
     openEditor(null);await Promise.all($('overlay').querySelector('.modal').getAnimations().map(a=>a.finished));await new Promise(r=>setTimeout(r,100));out.formContrast=contrast($('modal-save'))>=4.5&&contrast($('modal-cancel'))>=4.5;
     out.saveIconSurvives=!!$('modal-save').querySelector('svg');
    }finally{savingRequest=null;editorDirty=false;closeEditor();send=original.send;S=original.S;secrets=original.secrets;firstRun=original.firstRun;query=original.query;wantDetail=original.wantDetail;loginPending=false;loginDeadline=0;document.documentElement.dataset.theme=original.theme;render();}
    return out;
   })()`);
   for(const [key,value]of Object.entries(checks))assert.equal(value,true,`${width}x${height} ${theme}: ${key}`);
   results.push({width,height,theme,checks:Object.keys(checks).length});
  }
 }
 console.log(JSON.stringify(results,null,2));
}finally{close();}
