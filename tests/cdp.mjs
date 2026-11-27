import assert from 'node:assert/strict';
export async function connectCDP(){
 let targets; const deadline=Date.now()+15000;
 while(Date.now()<deadline){try{targets=await(await fetch('http://127.0.0.1:'+(process.env.CDP_PORT||9222)+'/json',{signal:AbortSignal.timeout(2000)})).json();if(targets.some(t=>t.type==='page'))break;}catch{}await new Promise(r=>setTimeout(r,250));}
 const target=targets?.find(t=>t.type==='page');assert(target,'CDP page not found within 15s');
 const ws=new WebSocket(target.webSocketDebuggerUrl);const pending=new Map();let n=0;
 await new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(Error('CDP open timeout')),5000);ws.addEventListener('open',()=>{clearTimeout(timer);resolve();},{once:true});ws.addEventListener('error',()=>{clearTimeout(timer);reject(Error('CDP connection error'));},{once:true});});
 const fail=()=>{for(const p of pending.values()){clearTimeout(p.timer);p.reject(Error('CDP connection closed'));}pending.clear();};
 ws.addEventListener('close',fail);ws.addEventListener('error',fail);
 ws.addEventListener('message',e=>{const m=JSON.parse(e.data),p=pending.get(m.id);if(!p)return;clearTimeout(p.timer);pending.delete(m.id);m.error?p.reject(Error(JSON.stringify(m.error))):p.resolve(m);});
 const cmd=(method,params={})=>new Promise((resolve,reject)=>{const id=++n;const timer=setTimeout(()=>{pending.delete(id);reject(Error('CDP timeout: '+method));},15000);pending.set(id,{resolve,reject,timer});ws.send(JSON.stringify({id,method,params}));});
 const run=async expression=>{const m=await cmd('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true});if(m.result?.exceptionDetails)throw Error(JSON.stringify(m.result.exceptionDetails));assert(m.result?.result,'Missing evaluate result');return m.result.result.value;};
 try {
  const readyBy=Date.now()+15000;let ready=false;
  while(Date.now()<readyBy){if(await run("document.readyState==='complete' && typeof send==='function'")){ready=true;break;}await new Promise(r=>setTimeout(r,100));}
  assert(ready,'SecBack scripts did not become ready within 15s');
 }catch(error){ws.close();fail();throw error;}
 return {cmd,run,close:()=>{ws.close();fail();}};
}
