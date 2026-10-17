#!/usr/bin/env node
// Exercise the actual Rust C ABI from a foreign C main calling libc exit.
// It intentionally runs as the ordinary user on its own temporary files;
// this is not an administrator or signed-App acceptance substitute.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { chmod, mkdir, mkdtemp, rm, stat, writeFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';

const run = (command,args,options={}) => {
  const result=spawnSync(command,args,{encoding:'utf8',timeout:30000,maxBuffer:16*1024*1024,...options});
  if (result.error) throw result.error;
  return result;
};

export async function verifyReadonlyForeignExit({staticLibrary,reportDirectory}) {
  assert.equal(process.platform,'darwin','This fixture needs the local Apple linker and Swift system runtime');
  assert.notEqual(process.getuid(),0,'Run this fixture as the ordinary login user');
  assert((await stat(staticLibrary)).isFile(),'Pass the exact Cargo-produced static library');
  await mkdir(reportDirectory,{recursive:true,mode:0o700});
  const fixture=await mkdtemp(join(reportDirectory,'foreign-exit-fixture-'));
  const harness=join(fixture,'foreign-main');
  const cSource=join(fixture,'foreign-main.c');
  const protectedDirectory=join(fixture,'no-access');
  const report={schema:'nomifun.readonly-foreign-exit.v1',driver_uid:process.getuid(),static_library:resolve(staticLibrary),scope:'Actual C ABI and libc exit; no root/helper registration or native App acceptance',checks:[],status:'running'};
  try {
    await writeFile(cSource,'#include <stdlib.h>\n#include <string.h>\n#include <unistd.h>\n#include <signal.h>\nextern int nomifun_system_readonly_worker(void);\nint main(int argc, char **argv) { if (argc == 2 && strcmp(argv[1], "--broken-output") == 0) { int fds[2]; if (pipe(fds) != 0) return 71; close(fds[0]); if (dup2(fds[1], STDOUT_FILENO) < 0) return 71; close(fds[1]); signal(SIGPIPE, SIG_IGN); } exit(nomifun_system_readonly_worker()); }\n');
    const sdk=run('xcrun',['--sdk','macosx','--show-sdk-path']);assert.equal(sdk.status,0,sdk.stderr);
    const architecture=process.arch==='arm64'?'arm64':'x86_64';
    const compiled=run('xcrun',['clang','-arch',architecture,'-mmacosx-version-min=14.0',cSource,resolve(staticLibrary),`-L${sdk.stdout.trim()}/usr/lib/swift`,'-framework','Foundation','-framework','ServiceManagement','-framework','Security','-framework','AppKit','-liconv','-lresolv','-Wl,-dead_strip','-o',harness]);
    assert.equal(compiled.status,0,compiled.stderr);
    const source=join(fixture,'source.txt');const body=Buffer.from('owned foreign-exit fixture 中文\n');await writeFile(source,body,{mode:0o600});
    const invoke=(request,args=[])=>run(harness,args,{input:JSON.stringify(request)});
    const decode=(result)=>{assert.equal(result.status,0,result.stderr);assert(result.stdout.length>0,'C exit lost the Rust stdout reply');return JSON.parse(result.stdout);};
    let result=invoke({kind:'source',path:source,max_bytes:16384});let reply=decode(result);
    assert.equal(reply.ok,true);assert.deepEqual(Buffer.from(reply.acquisition.source.bytes,'base64'),body);
    assert.equal(reply.acquisition.source.sha256,createHash('sha256').update(body).digest('hex'));
    report.checks.push({id:'foreign-exit-complete-source',status:'passed',exit_code:result.status,stdout_bytes:Buffer.byteLength(result.stdout)});
    result=invoke({kind:'source',path:join(fixture,'genuinely-missing.txt'),max_bytes:16384});reply=decode(result);
    assert.equal(reply.ok,true);assert.equal(reply.acquisition.source,null);
    report.checks.push({id:'foreign-exit-true-absence',status:'passed',stdout_bytes:Buffer.byteLength(result.stdout)});
    result=invoke({kind:'source',path:fixture,max_bytes:16384});reply=decode(result);
    assert.equal(reply.ok,false);assert.equal(reply.error.code,'INVALID_PAYLOAD');assert.equal(reply.acquisition,undefined);
    report.checks.push({id:'foreign-exit-metadata-error',status:'passed',code:reply.error.code});
    await mkdir(protectedDirectory,{mode:0o700});const denied=join(protectedDirectory,'AGENTS.md');await writeFile(denied,'private fixture rule\n',{mode:0o600});await chmod(protectedDirectory,0o000);
    result=invoke({kind:'source',path:denied,max_bytes:16384});reply=decode(result);
    assert.equal(reply.ok,false);assert.equal(reply.error.code,'SYSTEM_ACCESS_DENIED');assert.equal(reply.error.kind,'PermissionDenied');assert.equal(reply.acquisition,undefined);
    report.checks.push({id:'foreign-exit-denied-remains-error',status:'passed',code:reply.error.code,kind:reply.error.kind,raw_os_error:reply.error.raw_os_error});
    await chmod(protectedDirectory,0o700);
    result=invoke({kind:'source',path:source,max_bytes:16384},['--broken-output']);assert.equal(result.status,74,'A failed flush must not return success');
    report.checks.push({id:'foreign-exit-flush-failure',status:'passed',exit_code:result.status});
    report.status='passed';
  } catch (error) { report.status='failed';report.failure={name:error.name,message:error.message};throw error; }
  finally {
    await chmod(protectedDirectory,0o700).catch(()=>{});
    await writeFile(join(reportDirectory,'readonly-foreign-exit-result.json'),`${JSON.stringify(report,null,2)}\n`,{mode:0o600});
    await rm(fixture,{recursive:true,force:true});
  }
  return report;
}

if (process.argv[1]&&resolve(process.argv[1])===fileURLToPath(import.meta.url)) {
  const args=process.argv.slice(2);const option=(name)=>{const index=args.indexOf(name);assert(index>=0&&args[index+1],`${name} requires a value`);return args[index+1];};
  try {console.log(JSON.stringify(await verifyReadonlyForeignExit({staticLibrary:option('--static-library'),reportDirectory:option('--report-dir')})));}
  catch(error){console.error(`READONLY_FOREIGN_EXIT_FAIL ${error.message}`);process.exitCode=1;}
}
