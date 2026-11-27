// Build native code first, then copy exactly that library and package; never package concurrently.
import fs from 'node:fs/promises';
import path from 'node:path';
import {spawn} from 'node:child_process';
import {fileURLToPath} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const project=path.resolve(process.env.SECBACK_ANDROID_PROJECT||path.join(root,'src-tauri/gen/android'));
async function run(file,args,cwd=root){return new Promise((resolve,reject)=>{let output='';const p=spawn(file,args,{cwd,env:process.env,windowsHide:true});p.stdout.on('data',b=>{output+=b;process.stdout.write(b)});p.stderr.on('data',b=>{output+=b;process.stderr.write(b)});p.on('error',reject);p.on('close',code=>resolve({code,output}));});}
let result=await run(process.execPath,['scripts/check-version.mjs']);if(result.code)throw Error('Version check failed');
// Isolated Gradle projects must package the authoritative resources, including current icons.
const resources=path.join(root,'src-tauri/gen/android/app/src/main/res');
if(project!==path.join(root,'src-tauri/gen/android'))await fs.cp(resources,path.join(project,'app/src/main/res'),{recursive:true});
result=await run(process.execPath,['node_modules/@tauri-apps/cli/tauri.js','android','build','--target','aarch64','--apk']);
const nativeOutput=result.output.replace(/\x1b\[[0-9;]*m/g,'');
if(result.code && !(/Finished.*release/.test(nativeOutput)&&/symbolic link/.test(nativeOutput)))throw Error('Native Android build failed');
const target=process.env.CARGO_TARGET_DIR?path.resolve(root,process.env.CARGO_TARGET_DIR):path.join(root,'src-tauri/target');
const native=path.join(target,'aarch64-linux-android/release/libsecback_lib.so');
const jni=path.join(project,'app/src/main/jniLibs/arm64-v8a');await fs.mkdir(jni,{recursive:true});
const dest=path.join(jni,'libsecback_lib.so');
// Tauri may have made a symlink; removing this one link prevents copying onto the source itself.
if(await fs.lstat(dest).then(s=>s.isSymbolicLink()).catch(()=>false))await fs.unlink(dest);
await fs.copyFile(native,dest);
const args=['--no-daemon','--no-configuration-cache','assembleArm64Release','-Ptauri.abiFilters=arm64','-Psecback.sourceRoot='+root,'-x','rustBuildArm64Release','-x','rustBuildUniversalRelease','-x','rustBuildArmRelease'];
if(process.env.SECBACK_GRADLE_OFFLINE==='1')args.push('--offline');
if(process.env.SECBACK_GRADLE_INIT)args.push('-I',path.resolve(process.env.SECBACK_GRADLE_INIT));
args.push('--max-workers=1','-Pkotlin.compiler.execution.strategy=in-process');
const gradle=process.env.SECBACK_GRADLE_BIN;
result=process.platform==='win32'?await run('cmd.exe',['/d','/c',gradle||'gradlew.bat',...args],project):await run(gradle||'./gradlew',args,project);
if(result.code)throw Error('APK packaging failed');
const apk=path.join(project,'app/build/outputs/apk/arm64/release/app-arm64-release.apk');
// PowerShell/.NET verifies the actual APK entry and public signer before delivery.
result=await run('powershell.exe',['-NoProfile','-File',path.join(root,'scripts/verify-release.ps1'),'-Apk',apk,'-Native',native]);
if(result.code)throw Error('Release verification failed');
const delivery=path.join(root,'delivery/android');await fs.mkdir(delivery,{recursive:true});
const {version}=JSON.parse(await fs.readFile(path.join(root,'package.json'),'utf8'));
const delivered=path.join(delivery,`SecBack-${version}-Android-arm64.apk`);
await fs.copyFile(apk,delivered);await fs.copyFile(apk+'.manifest.json',delivered+'.manifest.json');
console.log(`Android arm64 release verified: ${delivered}`);
