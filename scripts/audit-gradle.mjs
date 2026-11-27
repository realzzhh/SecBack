// Audit the versions Gradle actually resolved, including transitive modules and build plugins.
import fs from 'node:fs/promises';
const [input,output,...additional]=process.argv.slice(2);
if(!input||!output)throw Error('Usage: node scripts/audit-gradle.mjs coordinates.json report.json [additional-coordinates.json ...]');
const graphs=await Promise.all([input,...additional].map(async file=>JSON.parse(await fs.readFile(file,'utf8'))));
if(graphs.some(g=>g.schema!==1||!g.scopes?.length))throw Error('Missing resolved Gradle scopes');
const data={scopes:graphs.flatMap((g,i)=>g.scopes.map(s=>({...s,project:(i?'buildSrc':'android')+s.project})))};
const packages=[...new Map(data.scopes.flatMap(s=>s.packages).map(p=>[p.name+'@'+p.version,p])).values()].sort((a,b)=>(a.name+'@'+a.version).localeCompare(b.name+'@'+b.version));
if(!packages.length)throw Error('Empty Gradle dependency graph');
async function request(url,body){
 for(let attempt=0;attempt<3;attempt++){
  try{const r=await fetch(url,{...(body?{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify(body)}:{}),signal:AbortSignal.timeout(60000)});if(!r.ok)throw Error('HTTP '+r.status);return await r.json();}
  catch(e){if(attempt===2)throw e;await new Promise(r=>setTimeout(r,1000*(attempt+1)));}
 }
}
const findings=[];
for(let start=0;start<packages.length;start+=100){
 const batch=packages.slice(start,start+100);
 const result=await request('https://api.osv.dev/v1/querybatch',{queries:batch.map(p=>({package:{ecosystem:'Maven',name:p.name},version:p.version}))});
 if(result.results?.length!==batch.length)throw Error('Incomplete OSV response');
 for(let i=0;i<batch.length;i++)for(const v of result.results[i].vulns||[])findings.push({package:batch[i],id:v.id});
}
const advisories={};
for(const id of new Set(findings.map(f=>f.id)))advisories[id]=await request('https://api.osv.dev/v1/vulns/'+encodeURIComponent(id));
const active=findings.filter(f=>!advisories[f.id].withdrawn);
const report={schema:1,checkedAt:new Date().toISOString(),source:'https://api.osv.dev/v1/querybatch',packageCount:packages.length,scopes:data.scopes.map(s=>({project:s.project,configuration:s.configuration,packageCount:s.packages.length})),packages,findings:active,advisories};
await fs.writeFile(output,JSON.stringify(report,null,2)+'\n');
console.log(JSON.stringify({packageCount:packages.length,scopeCount:data.scopes.length,findings:active},null,2));
if(active.length)process.exitCode=1;
