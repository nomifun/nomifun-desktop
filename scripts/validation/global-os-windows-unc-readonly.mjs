// The canonical UNC factory creates/removes its own share. This hook only
// opens its exact root with metadata access and never writes to that share.
import assert from 'node:assert/strict';
import {execFile} from 'node:child_process';
import {createHash,randomUUID} from 'node:crypto';
import {lstat,mkdir,readdir,readFile} from 'node:fs/promises';
import {dirname,join,resolve,win32} from 'node:path';
import {fileURLToPath} from 'node:url';
import {promisify} from 'node:util';
import {homedir} from 'node:os';
const execute=promisify(execFile),repository=resolve(dirname(fileURLToPath(import.meta.url)),'../..');
const digest=bytes=>createHash('sha256').update(bytes).digest('hex');
let prepared;
export function windowsUncReadonlyTarget({uncRoot,extendedUncRoot,physicalRoot}){
  const nonce=/^\\\\localhost\\NomiFun_GlobalOS_([0-9a-f-]{36})$/.exec(uncRoot)?.[1];assert(nonce,'The probe accepts only the canonical fixture-generated localhost share');
  assert.equal(extendedUncRoot,`\\\\?\\UNC\\localhost\\NomiFun_GlobalOS_${nonce}`);assert.equal(win32.basename(physicalRoot),'share-root');assert.equal(win32.basename(win32.dirname(physicalRoot)),`unc-cases-${nonce}`);assert.match(win32.basename(win32.dirname(win32.dirname(physicalRoot))),/^nomifun-global-os-product-[0-9a-f-]{36}$/);assert(win32.isAbsolute(physicalRoot));return{nonce,root:extendedUncRoot};
}
async function prepare(){
  assert.equal(process.platform,'win32');const dependencies=join(repository,'build.noindex','debug','deps'),source=join(repository,'scripts','validation','global-os-windows-unc-readonly.rs');
  const libraries=await Promise.all((await readdir(dependencies)).filter(name=>/^libnomi_native_files-[0-9a-f]+\.rlib$/.test(name)).map(async name=>{const path=join(dependencies,name);return{path,stat:await lstat(path)};}));
  libraries.sort((a,b)=>b.stat.mtimeMs-a.stat.mtimeMs);const library=libraries[0];assert(library&&library.stat.isFile()&&!library.stat.isSymbolicLink(),'A real Windows nomi-native-files build is required before this read-only probe');
  const output=join(repository,'build.noindex',`unc-readonly-probe-${randomUUID()}`);await mkdir(output);const executable=join(output,'unc-readonly-probe.exe');
  const registry=join(process.env.CARGO_HOME??join(homedir(),'.cargo'),'registry','src'),nativeLibraries=[];
  for(const index of await readdir(registry)){for(const name of await readdir(join(registry,index))){if(/^windows_x86_64_msvc-[0-9.]+$/.test(name))nativeLibraries.push(join(registry,index,name,'lib'));}}
  assert(nativeLibraries.length>0,'The real Windows target import libraries must already be installed');
  await execute('rustc',['--edition','2024',source,'--extern',`nomi_native_files=${library.path}`,'-L',`dependency=${dependencies}`,...nativeLibraries.flatMap(path=>['-L',`native=${path}`]),'-o',executable],{windowsHide:true,timeout:30000,maxBuffer:256*1024});
  return{executable,source_sha256:digest(await readFile(source)),native_library:library.path,native_library_sha256:digest(await readFile(library.path)),executable_sha256:digest(await readFile(executable))};
}
export async function probeWindowsUncDirectoryIdentity(input){
  const target=windowsUncReadonlyTarget(input),artifact=await(prepared??=prepare());
  const result=await execute(artifact.executable,[target.root],{windowsHide:true,timeout:10000,maxBuffer:256*1024});assert.equal(result.stderr.trim(),'');const value=JSON.parse(result.stdout);assert.equal(value.schema,'nomifun.windows-unc-readonly-native.v1');assert.equal(value.filesystem_mutations,0);assert.equal(value.target,`${target.root}\\sample.txt`);
  return{schema:'nomifun.windows-unc-readonly-probe.v1',nonce:target.nonce,artifact,result:value,share_mutations:0,file_content_reads:0,scope:'Exact fixture share-root metadata and handle identity only; no permission/race guard changed'};
}
