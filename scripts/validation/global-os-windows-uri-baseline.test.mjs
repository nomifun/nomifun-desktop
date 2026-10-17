import{test,expect}from'bun:test';
import{createHash}from'node:crypto';
import{validateWindowsDailyUriSnapshot,windowsDailyUriLaunchObservation,windowsDailyUriCloseObservation,windowsDailyUriSnapshotScript}from'./global-os-windows-uri-baseline.mjs';
const hash=value=>createHash('sha256').update(value).digest('hex');
function sample(stage='before'){
  const rows=[];for(const view of ['Registry64','Registry32'])for(const path of [String.raw`Software\Classes\nomifun`,String.raw`Software\Classes\nomifun\DefaultIcon`,String.raw`Software\Classes\nomifun\shell\open\command`])rows.push({hive:'CurrentUser',view,path,key_exists:true,values:(path===String.raw`Software\Classes\nomifun`?['','URL Protocol']:['']).map(name=>({name,exists:true,type:1,byte_length:2,sha256:hash(Buffer.from([0,0]))}))});
  const snapshot={schema:'nomifun.windows-daily-uri-digests.v1',started_at:'2026-10-17T07:30:00Z',observed_at:'2026-10-17T07:30:01Z',registry_writes:0,raw_values_recorded:false,rows,state_sha256:hash(JSON.stringify(rows))};
  return{snapshot,reference:{path:`C:\\private\\daily-uri-${stage}-unique.json`,sha256:hash(JSON.stringify(snapshot)),state_sha256:snapshot.state_sha256,observed_at:snapshot.observed_at,stage}};
}
test('daily URI snapshots contain only exact HKCU type/length/digests and never command bytes',()=>{
  const before=sample();expect(validateWindowsDailyUriSnapshot(before.snapshot)).toBe(before.snapshot);
  for(const mutate of [x=>{x.raw='secret';},x=>{x.rows[0].raw='secret';},x=>{x.rows[0].values[0].value='secret';},x=>{x.rows[0].hive='LocalMachine';},x=>{x.rows[2].path=String.raw`Software\Classes\other`;},x=>{x.rows[0].values[0].byte_length=1048577;},x=>{x.rows[0].values[0].type=-1;},x=>{x.rows[0].values[0].sha256='bad';}]){const x=sample().snapshot;mutate(x);expect(()=>validateWindowsDailyUriSnapshot(x)).toThrow();}
  expect(windowsDailyUriSnapshotScript).toContain('root.OpenSubKey(path,false)');expect(windowsDailyUriSnapshotScript).toContain('RegQueryValueEx');
  for(const word of ['CreateSubKey','SetValue','DeleteSubKey','GetValue('])expect(windowsDailyUriSnapshotScript).not.toContain(word);
});
test('restart and upgrade baseline always refer to this launch and do not fabricate original values',()=>{
  const before=sample(),after=sample('after-ready');after.snapshot.observed_at='2026-10-17T07:31:00Z';
  for(const kind of ['fresh','restart','upgrade']){
    const report=windowsDailyUriLaunchObservation({kind,before,after,priorFixtureReport:'C:\\prior.json'});
    expect(report.this_launch_before).toBe(before.reference);expect(report.changed_during_launch).toBe(false);expect(report.original_pre_first_app_launch_value_proven).toBe(false);expect(report.prior_snapshot_is_this_launch_baseline).toBe(false);expect(report.registry_restoration_performed).toBe(false);
  }
  after.snapshot.rows[2].values[0].sha256=hash('different command bytes');after.snapshot.state_sha256=hash(JSON.stringify(after.snapshot.rows));
  after.reference.state_sha256=after.snapshot.state_sha256;
  expect(windowsDailyUriLaunchObservation({kind:'upgrade',before,after}).changed_during_launch).toBe(true);
  expect(windowsDailyUriLaunchObservation({kind:'restart',before}).after_ready).toBeNull();
});
test('closing an old fixture without a captured baseline remains observation only',async()=>{
  const result=await windowsDailyUriCloseObservation({parent:'C:\\private',current:sample('close-current')});
  expect(result.comparison).toBeNull();expect(result.this_launch_before_reference_recorded).toBe(false);expect(result.registry_restoration_performed).toBe(false);
});
