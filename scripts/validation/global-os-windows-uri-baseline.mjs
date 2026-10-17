// Read exactly the daily URI values touched by tauri-plugin-deep-link. Native
// bytes never leave the reader: reports contain only type, length and SHA-256.
import assert from 'node:assert/strict';
import {createHash,randomUUID} from 'node:crypto';
import {writeFile} from 'node:fs/promises';
import {dirname,join,resolve} from 'node:path';
import {assertWindowsPrivatePath,readWindowsOwnedFixtureBytes,windowsObservation} from './global-os-windows-cases.mjs';

const paths=[String.raw`Software\Classes\nomifun`,String.raw`Software\Classes\nomifun\DefaultIcon`,String.raw`Software\Classes\nomifun\shell\open\command`];
const views=['Registry64','Registry32'];
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
const stateHash=rows=>hash(JSON.stringify(rows));
export const windowsDailyUriSnapshotScript=String.raw`Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using Microsoft.Win32;
using Microsoft.Win32.SafeHandles;
public static class NomiFunDailyUriDigest {
  [DllImport("advapi32.dll",CharSet=CharSet.Unicode)] static extern int RegQueryValueEx(SafeRegistryHandle key,string name,IntPtr reserved,out uint type,byte[] data,ref uint length);
  static object Value(RegistryKey key,string name) {
    if(key==null) return new {name,exists=false,type=(uint?)null,byte_length=(uint?)null,sha256=(string)null};
    for(int attempt=0;attempt<3;attempt++) {
      uint type,length=0;int status=RegQueryValueEx(key.Handle,name,IntPtr.Zero,out type,null,ref length);
      if(status==2) return new {name,exists=false,type=(uint?)null,byte_length=(uint?)null,sha256=(string)null};
      if(status!=0&&status!=234) throw new Win32Exception(status);
      if(length>1048576) throw new InvalidOperationException("URI value exceeds bounded digest budget");
      byte[] bytes=new byte[length];status=RegQueryValueEx(key.Handle,name,IntPtr.Zero,out type,bytes,ref length);
      if(status==234) continue;
      if(status!=0) throw new Win32Exception(status);
      using(SHA256 sha=SHA256.Create()) {
        string digest=BitConverter.ToString(sha.ComputeHash(bytes,0,checked((int)length))).Replace("-","").ToLowerInvariant();
        return new {name,exists=true,type=(uint?)type,byte_length=(uint?)length,sha256=digest};
      }
    }
    throw new InvalidOperationException("URI value changed during bounded acquisition");
  }
  public static object[] Read() {
    var rows=new List<object>();
    foreach(RegistryView view in new[]{RegistryView.Registry64,RegistryView.Registry32})
      using(RegistryKey root=RegistryKey.OpenBaseKey(RegistryHive.CurrentUser,view))
        foreach(string path in new[]{@"Software\Classes\nomifun",@"Software\Classes\nomifun\DefaultIcon",@"Software\Classes\nomifun\shell\open\command"})
          using(RegistryKey key=root.OpenSubKey(path,false)) {
            var values=new List<object>();values.Add(Value(key,""));
            if(path==@"Software\Classes\nomifun") values.Add(Value(key,"URL Protocol"));
            rows.Add(new {hive="CurrentUser",view=view.ToString(),path,key_exists=key!=null,values=values.ToArray()});
          }
    return rows.ToArray();
  }
}
'@
ConvertTo-Json -InputObject ([NomiFunDailyUriDigest]::Read()) -Depth 7 -Compress`;

export function validateWindowsDailyUriSnapshot(snapshot){
  assert.deepEqual(Object.keys(snapshot).sort(),['observed_at','raw_values_recorded','registry_writes','rows','schema','started_at','state_sha256']);
  assert.equal(snapshot.schema,'nomifun.windows-daily-uri-digests.v1');
  assert.equal(snapshot.registry_writes,0);assert.equal(snapshot.raw_values_recorded,false);
  assert.equal(typeof snapshot.started_at,'string');assert.equal(typeof snapshot.observed_at,'string');
  assert.equal(snapshot.rows.length,6);
  snapshot.rows.forEach((row,index)=>{
    assert.deepEqual(Object.keys(row).sort(),['hive','key_exists','path','values','view']);
    assert.equal(row.hive,'CurrentUser');assert.equal(row.view,views[Math.floor(index/3)]);assert.equal(row.path,paths[index%3]);assert.equal(typeof row.key_exists,'boolean');
    const names=index%3===0?['','URL Protocol']:[''];assert.equal(row.values.length,names.length);
    row.values.forEach((value,at)=>{
      assert.deepEqual(Object.keys(value).sort(),['byte_length','exists','name','sha256','type']);
      assert.equal(value.name,names[at]);assert.equal(typeof value.exists,'boolean');
      if(value.exists){assert(row.key_exists);assert(Number.isInteger(value.type)&&value.type>=0&&value.type<=0xffffffff);assert(Number.isInteger(value.byte_length)&&value.byte_length>=0&&value.byte_length<=1048576);assert.match(value.sha256,/^[a-f0-9]{64}$/);}
      else for(const field of ['type','byte_length','sha256'])assert.equal(value[field],null);
    });
  });
  assert.equal(snapshot.state_sha256,stateHash(snapshot.rows));return snapshot;
}

function validateReference(value,stage){
  assert.deepEqual(Object.keys(value).sort(),['observed_at','path','sha256','stage','state_sha256']);
  assert.equal(typeof value.path,'string');assert.match(value.sha256,/^[a-f0-9]{64}$/);assert.match(value.state_sha256,/^[a-f0-9]{64}$/);assert.equal(value.stage,stage);
}

export function windowsDailyUriLaunchObservation({kind,before,after,priorFixtureReport}){
  assert(['fresh','restart','upgrade'].includes(kind));
  validateWindowsDailyUriSnapshot(before.snapshot);validateReference(before.reference,'before');assert.equal(before.reference.state_sha256,before.snapshot.state_sha256);
  if(after){validateWindowsDailyUriSnapshot(after.snapshot);validateReference(after.reference,'after-ready');assert.equal(after.reference.state_sha256,after.snapshot.state_sha256);}
  return{schema:'nomifun.windows-daily-uri-launch-observation.v1',sampling_scope:'this_launch_only',launch_kind:kind,
    automatic_registration_source:'tauri_plugin_deep_link.register_all',this_launch_before:before.reference,after_ready:after?.reference??null,
    changed_during_launch:after?before.snapshot.state_sha256!==after.snapshot.state_sha256:null,
    observed_change_is_exclusive_causality_proof:false,original_pre_first_app_launch_value_proven:false,
    prior_fixture_report:priorFixtureReport?resolve(priorFixtureReport):null,prior_snapshot_is_this_launch_baseline:false,
    registry_restoration_performed:false};
}

export async function captureWindowsDailyUriSnapshot({parent,stage,observe=windowsObservation}){
  assert(['before','after-ready','close-current'].includes(stage));await assertWindowsPrivatePath(parent);
  const started_at=new Date().toISOString();let rows;
  try{rows=await observe(windowsDailyUriSnapshotScript);}catch{throw new Error('Exact daily URI digest acquisition failed; native details and registry values withheld');}
  const snapshot=validateWindowsDailyUriSnapshot({schema:'nomifun.windows-daily-uri-digests.v1',started_at,observed_at:new Date().toISOString(),registry_writes:0,raw_values_recorded:false,rows,state_sha256:stateHash(rows)});
  const path=join(parent,`daily-uri-${stage}-${randomUUID()}.json`),bytes=Buffer.from(JSON.stringify(snapshot,null,2)+'\n');
  await writeFile(path,bytes,{flag:'wx'});
  return{snapshot,reference:{path,sha256:hash(bytes),state_sha256:snapshot.state_sha256,observed_at:snapshot.observed_at,stage}};
}

export async function windowsDailyUriCloseObservation({parent,launchObservation,current}){
  validateWindowsDailyUriSnapshot(current.snapshot);validateReference(current.reference,'close-current');
  const reference=launchObservation?.after_ready;let comparison=null;
  if(reference){
    validateReference(reference,'after-ready');
    assert.equal(resolve(dirname(reference.path)),resolve(parent));assert.match(reference.sha256,/^[a-f0-9]{64}$/);
    const bytes=await readWindowsOwnedFixtureBytes(reference.path,64*1024);assert.equal(hash(bytes),reference.sha256);
    const original=validateWindowsDailyUriSnapshot(JSON.parse(bytes.toString('utf8')));assert.equal(original.state_sha256,reference.state_sha256);
    comparison={launch_after_ready:reference,changed_since_launch_after_ready:original.state_sha256!==current.snapshot.state_sha256};
  }
  return{schema:'nomifun.windows-daily-uri-close-observation.v1',current:current.reference,comparison,
    this_launch_before_reference_recorded:Boolean(launchObservation?.this_launch_before),
    baseline_scope:'Recorded launch sampling only; missing historical observations are not reconstructed',registry_restoration_performed:false};
}
