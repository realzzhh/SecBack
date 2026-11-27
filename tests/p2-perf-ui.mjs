import {connectCDP} from './cdp.mjs';
const c=await connectCDP();
try {
 const results=await c.run(`(async()=>{
 const state=S,originalSend=send;send=()=>{};const results=[];
 try{for(const count of [100,1000,10000]){
  const empty=()=>({user:'',remark:'',empty:true,hasPass:false});
  S={projects:Array.from({length:count},(_,i)=>({id:i.toString(16).padStart(32,'0'),name:'性能虚构项目 '+i,remark:'备注',slots:[{user:'user-'+i,remark:'',empty:false,hasPass:true},empty(),empty()]})),selected:'0'.repeat(32)};
  query='';$('app').hidden=false;$('login').hidden=true;
  const start=performance.now();renderList();const renderMs=performance.now()-start;
  const selectStart=performance.now();selectProject(S.projects[count-1].id);const selectMs=performance.now()-selectStart;
  query='项目 9';const searchStart=performance.now();renderList();const searchMs=performance.now()-searchStart;
  results.push({count,renderMs,selectMs,searchMs,rows:$('list').children.length});
  await new Promise(r=>requestAnimationFrame(r));
 }}finally{S=state;send=originalSend;query='';render();}
 return results;})()`);
 console.log(JSON.stringify(results,null,2));
}finally{c.close();}
