// Only generates a script for a separately owned canonical Agent namespace.
// Importing this module never compiles, spawns, creates a share or opens a file.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {fileURLToPath} from 'node:url';
import {win32} from 'node:path';
const source=readFileSync(fileURLToPath(new URL('./global-os-windows-smb-disposition-probe.cs',import.meta.url)),'utf8');
const uuid=/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const preservedNonce='34d53b78-f4d3-4128-b8a6-cb7e6b509895';
const literal=value=>`'${value.replaceAll("'","''")}'`;
const pathParts=path=>{assert(typeof path==='string'&&/^[A-Za-z]:\\/.test(path)&&!/[\x00-\x1f]/.test(path));assert.equal(win32.normalize(path),path);assert(path.split('\\').slice(1).every(part=>part&&part!=='.'&&part!=='..'));return path;};
const overlapping=(a,b)=>{a=a.toLowerCase();b=b.toLowerCase();return a===b||a.startsWith(b+'\\')||b.startsWith(a+'\\');};
export function windowsSmbDispositionControlTarget({nonce,physicalRoot,shareName,preservedPhysicalRoot,preservedShareName}){
  assert(uuid.test(nonce)&&nonce!==preservedNonce,'A new UUID-v4 control namespace is required');
  pathParts(physicalRoot);pathParts(preservedPhysicalRoot);
  assert.equal(shareName,`NomiFun_GlobalOS_Disposition_${nonce}`);
  assert.equal(preservedShareName,`NomiFun_GlobalOS_${preservedNonce}`);
  assert.equal(win32.basename(physicalRoot),'share-root');
  assert.equal(win32.basename(win32.dirname(physicalRoot)),`smb-disposition-cases-${nonce}`);
  const fixtureName=win32.basename(win32.dirname(win32.dirname(physicalRoot)));
  assert(fixtureName.startsWith('nomifun-global-os-product-')&&uuid.test(fixtureName.slice('nomifun-global-os-product-'.length)));
  assert.equal(win32.basename(preservedPhysicalRoot),'share-root');
  assert.equal(win32.basename(win32.dirname(preservedPhysicalRoot)),`unc-cases-${preservedNonce}`);
  assert.notEqual(win32.dirname(win32.dirname(physicalRoot)).toLowerCase(),win32.dirname(win32.dirname(preservedPhysicalRoot)).toLowerCase(),'Use a new physical fixture, not the preserved pending fixture');
  assert(!overlapping(physicalRoot,preservedPhysicalRoot),'The control and pending physical namespaces must not overlap');
  return{nonce,physicalRoot,shareName,uncRoot:`\\\\localhost\\${shareName}`,markerName:`identity-${nonce}.txt`,markerText:`NOMIFUN_SMB_DISPOSITION_CONTROL_${nonce}\n`,caseCount:16};
}
export function windowsSmbDispositionProbeScript(input){
  const target=windowsSmbDispositionControlTarget(input);
  const script=`Add-Type -TypeDefinition @'\n${source}\n'@\n[NomiSmbDispositionControl]::Run(${literal(target.nonce)},${literal(target.physicalRoot)},${literal(target.uncRoot)},${literal(target.markerText)})|ConvertTo-Json -Depth 12 -Compress`;
  assert(Buffer.byteLength(Buffer.from(script,'utf16le').toString('base64'),'ascii')<29000,'The fixed WinPS encoded command must fit the real Windows command-line budget');
  return script;
}
