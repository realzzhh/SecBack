// Regenerate every desktop and Android launcher icon from the editable SVG sources.
import {spawnSync} from 'node:child_process';
import fs from 'node:fs';
import {fileURLToPath} from 'node:url';
const root=fileURLToPath(new URL('../',import.meta.url));
const result=spawnSync(process.execPath,['node_modules/@tauri-apps/cli/tauri.js','icon','src-tauri/icons/icon-source.json','--output','src-tauri/icons'],{cwd:root,stdio:'inherit',windowsHide:true});
if(result.error)throw result.error;if(result.status!==0)process.exit(result.status||1);
fs.copyFileSync(new URL('../src-tauri/icons/icon.png',import.meta.url),new URL('../src-tauri/icons/source-icon.png',import.meta.url));
