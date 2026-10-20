import{test,expect}from'bun:test';
import{createHash}from'node:crypto';
import{resolve}from'node:path';
import{assertWindowsPriorAppExited,windowsRendererReadinessTransient}from'./global-os-windows-fixture.mjs';


test('renderer readiness waits only for exact Node or observed Bun transport transients',()=>{
  for(const code of ['ECONNREFUSED','ECONNRESET','UND_ERR_SOCKET'])expect(windowsRendererReadinessTransient({name:'TypeError',cause:{code}})).toBe(true);
  expect(windowsRendererReadinessTransient({name:'TypeError',code:'ConnectionRefused'})).toBe(true);
  for(const name of ['TimeoutError','AbortError'])expect(windowsRendererReadinessTransient({name})).toBe(true);
  for(const error of [new TypeError('renderer broke'),new SyntaxError('invalid CDP JSON'),new Error('The original owner JWT was rejected after native restart'),{name:'TypeError',code:'EPERM'},{name:'TypeError',cause:{code:'EACCES'}},{name:'Error',code:'ConnectionRefused'},undefined])expect(windowsRendererReadinessTransient(error)).toBe(false);
});
test('restart distinguishes a reused PID from the original native App generation without inventing an exit code',()=>{
  const source={app_pid:100,actual_app_identity:{native_process:{pid:100,creation_filetime:'1234'}}};
  const reused={pid:100,state:'running',creation_filetime:'5678',exit_code:''};
  expect(assertWindowsPriorAppExited(source,reused)).toEqual({...reused,pid_reused:true,expected_prior_creation_filetime:'1234',prior_app_running:false});
  for(const state of ['absent','exited']){const observed={pid:100,state};expect(assertWindowsPriorAppExited(source,observed)).toBe(observed);}
  expect(()=>assertWindowsPriorAppExited(source,{...reused,creation_filetime:'1234'})).toThrow();
  expect(()=>assertWindowsPriorAppExited(source,{...reused,pid:101})).toThrow();
  expect(()=>assertWindowsPriorAppExited({app_pid:100},reused)).toThrow();
  expect(()=>assertWindowsPriorAppExited({...source,actual_app_identity:{native_process:{pid:100,creation_filetime:''}}},reused)).toThrow();
});
