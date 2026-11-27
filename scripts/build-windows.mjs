// Build the Windows executable and give every delivered artifact its product version.
import fs from 'node:fs/promises';
import path from 'node:path';
import {spawn} from 'node:child_process';
import {fileURLToPath} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
if(process.platform!=='win32')throw Error('Windows packaging must run on Windows');
const args=process.argv.slice(2);
if(args.some(a=>a!=='--debug'))throw Error('Usage: node scripts/build-windows.mjs [--debug]');
const debug=args.includes('--debug');
async function run(args){await new Promise((resolve,reject)=>{const p=spawn(process.execPath,args,{cwd:root,env:process.env,stdio:'inherit',windowsHide:true});p.on('error',reject);p.on('close',code=>code===0?resolve():reject(Error(`Build failed (${code})`)));});}
await run(['scripts/check-version.mjs']);
await run(['node_modules/@tauri-apps/cli/tauri.js','build','--no-bundle',...(debug?['--debug']:[])]);
const {version}=JSON.parse(await fs.readFile(path.join(root,'package.json'),'utf8'));
const target=path.resolve(root,process.env.CARGO_TARGET_DIR||'src-tauri/target');
const delivery=path.join(root,'delivery');await fs.mkdir(delivery,{recursive:true});
const filename=`SecBack-${version}-Windows-${process.arch}${debug?'-debug':''}.exe`;
const delivered=path.join(delivery,filename);
await fs.copyFile(path.join(target,debug?'debug':'release','secback.exe'),delivered);
console.log(`Windows executable delivered: ${delivered}`);
