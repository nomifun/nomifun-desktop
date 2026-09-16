#!/usr/bin/env node
// Real AppKit/WKWebView prerequisite tests. Never flips product availability.
// Exit 1 = conformance failure; 77 = permission required; 2 = harness failure.
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { readFile, writeFile, mkdir, mkdtemp, rename, rm } from 'node:fs/promises';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

try {
if (process.platform !== 'darwin') { console.error('macOS WKWebView required; this is not a passing native test.'); process.exit(2); }
const root=resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const args=process.argv.slice(2);
function option(name, fallback) { const index=args.indexOf(name); return index<0 ? fallback : args[index+1]; }
const probe=option('--probe','input'), transport=option('--transport','appkit');
if (!['input','storage','permission'].includes(probe) || !['appkit','pid'].includes(transport)) throw new Error('Use --probe input|storage|permission and --transport appkit|pid');
const output=resolve(option('--output',resolve(root,'dist/browser-macos-native-probe')));
const identity=option('--identity','-');
const app=resolve(output,'NomiBrowserNativeProbe.app');
await mkdir(output,{recursive:true});
const report=resolve(output,`${probe}-${transport}-${Date.now()}.json`);
const environment=Object.fromEntries(['PATH','HOME','TMPDIR','LANG','DEVELOPER_DIR'].filter(key=>process.env[key]).map(key=>[key,process.env[key]]));
const run=(command, argv)=>new Promise((res,rej)=>{
  const child=spawn(command,argv,{cwd:root,env:environment,stdio:'inherit'});
  child.once('error',rej);child.once('exit',code=>code===0?res():rej(new Error(`${command} exited ${code}`)));
});
const architecture=process.arch==='arm64'?'arm64':process.arch==='x64'?'x86_64':null;
if(!architecture) throw new Error('Unsupported native Mac architecture');
// Reuse the exact signed bundle while validating a newly granted TCC permission.
if(!args.includes('--reuse-signed-app')) {
const staging=await mkdtemp(resolve(output,'.native-probe-build-'));
const stagedApp=resolve(staging,'NomiBrowserNativeProbe.app');
const previous=resolve(staging,'previous.app');
let movedPrevious=false;
let preserveStaging=false;
try {
  await mkdir(resolve(stagedApp,'Contents/MacOS'),{recursive:true});
  await run('swiftc',['-target',`${architecture}-apple-macos14.0`,'-parse-as-library','-framework','AppKit','-framework','WebKit',resolve(root,'apps/desktop/examples/support/macos/native_browser_probe.swift'),'-o',resolve(stagedApp,'Contents/MacOS/NomiBrowserNativeProbe')]);
  await writeFile(resolve(stagedApp,'Contents/Info.plist'),`<?xml version="1.0"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>com.nomifun.browser-native-probe</string><key>CFBundleName</key><string>Nomi Browser Native Probe</string><key>CFBundleExecutable</key><string>NomiBrowserNativeProbe</string><key>CFBundlePackageType</key><string>APPL</string><key>NSHighResolutionCapable</key><true/><key>LSMinimumSystemVersion</key><string>14.0</string></dict></plist>`);
  await run('codesign',['--force','--sign',identity,stagedApp]);
  await run('codesign',['--verify','--strict',stagedApp]);
  // Never leave an authorized app temporarily unsigned during an in-place build.
  try { await rename(app,previous); movedPrevious=true; } catch(error) { if(error.code!=='ENOENT') throw error; }
  try { await rename(stagedApp,app); }
  catch(error) {
    if(movedPrevious) {
      try { await rename(previous,app); }
      catch(restoreError) { preserveStaging=true; throw new Error(`App replacement failed; previous signed app retained at ${previous}: ${restoreError.message}`); }
    }
    throw error;
  }
} finally { if(!preserveStaging) await rm(staging,{recursive:true,force:true}); }

}
await run('codesign',['--verify','--strict',app]);
if(args.includes('--build-only')) { console.log(JSON.stringify({status:'built_not_run',app})); process.exit(0); }
const html=await readFile(resolve(root,'apps/desktop/examples/fixtures/browser_workspace.html'));
const server=createServer((request,response)=>{
  if(request.url!=='/browser_workspace.html'){response.writeHead(404);response.end();return;}
  response.writeHead(200,{'Content-Type':'text/html; charset=utf-8','Cache-Control':'no-store'});response.end(html);
});
await new Promise((res,rej)=>{server.once('error',rej);server.listen(0,'127.0.0.1',res)});
try {
  const nativeArgs=['--probe',probe,'--transport',transport,'--fixture-url',`http://127.0.0.1:${server.address().port}/browser_workspace.html`,'--report',report];
  // Requesting OS authorization is explicit, never a side effect of a normal check.
  if(args.includes('--request-event-access')) nativeArgs.push('--request-event-access');
  await run('open',['-W','-n',app,'--args',...nativeArgs]);
  const result=JSON.parse(await readFile(report,'utf8'));
  console.log(JSON.stringify({report,app,...result},null,2));
  process.exitCode=Number.isInteger(result.exitCode)?result.exitCode:2;
} finally { await new Promise(resolve=>server.close(resolve)); }

} catch (error) {
  console.error(`Native probe could not finish: ${error.message}`);
  process.exitCode=2;
}
