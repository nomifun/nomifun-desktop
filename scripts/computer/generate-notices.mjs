import fs from 'node:fs/promises';
import path from 'node:path';
import {createHash} from 'node:crypto';
import {fileURLToPath} from 'node:url';
// Maintenance only: consume actual locked Cargo metadata. Missing archive
// notices come from the exact public repository commit in .cargo_vcs_info.
// This never builds, installs or starts the native worker.
const root=fileURLToPath(new URL('../../',import.meta.url));
const [metadataPath,platform,...targets]=process.argv.slice(2);
if(!metadataPath||!['windows','linux','macos'].includes(platform)||!targets.length
 ||targets.some(target=>!/^[a-z0-9_]+(?:-[a-z0-9_]+){2,}$/.test(target)))throw Error('metadata JSON, platform and exact target triple(s) required');
const metadata=JSON.parse(await fs.readFile(metadataPath,'utf8'));
const directory=path.join(root,'crates/backend/nomifun-runtime/notices');
const sha=bytes=>createHash('sha256').update(bytes).digest('hex');
const requests=new Map();
async function remote(url){if(!requests.has(url))requests.set(url,(async()=>{const response=await fetch(url);if(response.status===404)return null;if(!response.ok)throw Error(`${response.status} ${url}`);return Buffer.from(await response.arrayBuffer());})());return requests.get(url);}
const records=[],texts=new Map();
for(const pkg of metadata.packages.sort((a,b)=>a.name.localeCompare(b.name)||a.version.localeCompare(b.version))){
 if(pkg.name==='nomifun-computer-worker')continue;
 const packageRoot=path.dirname(pkg.manifest_path),files=[];
 for(const entry of await fs.readdir(packageRoot,{withFileTypes:true}))if(entry.isFile()&&/^(?:licen[cs]e|copying|notice|copyright)(?:[._-].*)?$/i.test(entry.name))files.push(path.join(packageRoot,entry.name));
 if(pkg.license_file&&!files.includes(pkg.license_file))files.push(pkg.license_file);
 const licenses=[];
 for(const file of files){const bytes=await fs.readFile(file);if(bytes.includes(0))throw Error('nontext license '+file);licenses.push({filename:path.basename(file),bytes,source:pkg.source?`https://crates.io/api/v1/crates/${pkg.name}/${pkg.version}/download`:'https://github.com/trycua/cua/tree/4d959bbfa57f0c89f310330e849500b222084194/libs/cua-driver/rust'});}
 if(!licenses.length&&pkg.source){
   const vcs=JSON.parse(await fs.readFile(path.join(packageRoot,'.cargo_vcs_info.json'),'utf8'));
   if(!/^https:\/\/github.com\/[^/]+\/[^/]+\/?$/.test(pkg.repository??'')||!vcs.git.sha1)throw Error('No exact upstream license source '+pkg.name);
   const repo=pkg.repository.replace(/\/$/,'').replace(/\.git$/,'').replace('https://github.com/','');
   for(const filename of ['LICENSE','LICENSE.md','LICENSE.txt','LICENSE-MIT','COPYING']){
     const url=`https://raw.githubusercontent.com/${repo}/${vcs.git.sha1}/${filename}`,bytes=await remote(url);
     if(bytes){licenses.push({filename,bytes,source:url});break;}
   }
 }
 if(!licenses.length&&!pkg.source)licenses.push({filename:'LICENSE.md',bytes:await fs.readFile(path.join(root,'vendor/cua-driver/LICENSE.md')),source:'https://github.com/trycua/cua/blob/4d959bbfa57f0c89f310330e849500b222084194/LICENSE.md'});
 if(!licenses.length)throw Error('License not found '+pkg.name);
 const record={name:pkg.name,version:pkg.version,license:pkg.license,authors:pkg.authors,repository:pkg.repository,source:pkg.source?`https://crates.io/crates/${pkg.name}/${pkg.version}`:'https://github.com/trycua/cua/tree/4d959bbfa57f0c89f310330e849500b222084194/libs/cua-driver/rust',licenses:[]};
 for(const item of licenses){const hash=sha(item.bytes);record.licenses.push({filename:item.filename,source:item.source,sha256:hash});if(!texts.has(hash))texts.set(hash,{...item,packages:[]});texts.get(hash).packages.push(`${pkg.name} ${pkg.version}`);}
 records.push(record);
}
const lockSha=sha(await fs.readFile(path.join(root,'apps/computer-worker/Cargo.lock')));
const prefix=`nomifun-computer-worker-${platform}`;
const inventory={schema_version:1,platform,worker_lock_sha256:lockSha,upstream_commit:'4d959bbfa57f0c89f310330e849500b222084194',metadata_scope:'Cargo metadata --locked --offline --filter-platform; includes resolved build dependencies. This is source/license provenance, not native or package acceptance.',packages:records,targets};
await fs.writeFile(path.join(directory,prefix+'-dependencies.json'),JSON.stringify(inventory,null,2)+'\n');
let body=`NomiFun Computer worker third-party notices (${platform})\n\nGenerated from the locked independent worker Cargo metadata.\nCargo.lock SHA-256: ${lockSha}\nThe adjacent ${prefix}-dependencies.json records every package, declared SPDX expression, authors, upstream source and license hashes.\nPackages with MPL-2.0 source are available from their exact public upstream sources and crates.io archives linked in that inventory.\nThese notices preserve upstream license texts, including optional/build dependencies; they do not assert every listed package is linked into an installed binary.\n\n`;
for(const [hash,item]of [...texts].sort(([a],[b])=>a.localeCompare(b))){body+=`======================================================================\nPackages: ${item.packages.join(', ')}\nLicense file: ${item.filename}\nSource: ${item.source}\nSHA-256: ${hash}\n----------------------------------------------------------------------\n${item.bytes.toString('utf8')}\n\n`;}
await fs.writeFile(path.join(directory,prefix+'-THIRD_PARTY_NOTICES.txt'),body);
console.log(JSON.stringify({platform,packages:records.length,distinct_license_texts:texts.size,lock_sha256:lockSha}));
