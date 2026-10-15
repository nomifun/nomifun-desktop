import { expect, test } from 'bun:test';
import { EventEmitter, once } from 'node:events';
import { Readable } from 'node:stream';
import { createServer } from 'node:net';
import { spawn } from 'node:child_process';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { runCargoArtifacts, runCommand } from './run-macos-dev-runner.mjs';

test('Cargo collector joins stdout EOF with exit, accepts split UTF-8 and the final unterminated report', async () => {
  const child = new EventEmitter();
  let continueOutput;
  const held = new Promise(accept => { continueOutput = accept; });
  const first = Buffer.from(JSON.stringify({ reason: 'compiler-artifact', target: { name: 'nomifun-desktop' }, executable: 'target/中文/nomifun-desktop' }) + '\r\n');
  const split = first.indexOf(Buffer.from('中')) + 1;
  child.stdout = Readable.from((async function* () {
    yield first.subarray(0, split);
    await held;
    yield first.subarray(split);
    yield Buffer.from(JSON.stringify({ reason: 'compiler-artifact', target: { name: 'macos_application_controller' }, executable: 'target/debug/examples/macos_application_controller' }));
  })());
  let settled = false;
  const report = runCargoArtifacts(['build'], ['nomifun-desktop', 'macos_application_controller'], {
    cwd: '/exact-workspace', environment: {}, spawnCargo: () => child,
  }).finally(() => { settled = true; });
  child.emit('exit', 0, null);
  await Promise.resolve();
  expect(settled).toBe(false);
  continueOutput();
  expect(await report).toEqual({
    'nomifun-desktop': resolve('/exact-workspace', 'target/中文/nomifun-desktop'),
    macos_application_controller: resolve('/exact-workspace', 'target/debug/examples/macos_application_controller'),
  });
});

test('Cargo collector preserves stream errors and never guesses a missing executable', async () => {
  const failed = new EventEmitter();
  failed.stdout = Readable.from((async function* () { throw new Error('actual stdout read failed'); })());
  const failure = runCargoArtifacts(['build'], ['nomifun-desktop'], { spawnCargo: () => failed });
  failed.emit('exit', 0, null);
  await expect(failure).rejects.toThrow('actual stdout read failed');
  const missing = new EventEmitter();
  missing.stdout = Readable.from(['null\n', JSON.stringify({ reason: 'compiler-artifact', target: { name: 'unrelated' }, executable: '/unrelated' })]);
  const absent = runCargoArtifacts(['build'], ['nomifun-desktop'], { spawnCargo: () => missing });
  missing.emit('exit', 0, null);
  await expect(absent).rejects.toThrow('Cargo did not report the nomifun-desktop executable');
});

test.skipIf(process.platform === 'win32')('captured command waits for real inherited stdout EOF after the direct child exits', async () => {
  const root = await mkdtemp(join(tmpdir(), 'nomifun-capture-eof-'));
  const socketPath = join(root, 'writer.sock');
  let peer;
  const server = createServer(socket => { peer = socket; });
  try {
    await new Promise((accept, reject) => { server.once('error', reject); server.listen(socketPath, accept); });
    const child = join(root, 'writer.mjs');
    const parent = join(root, 'parent.mjs');
    await writeFile(child, 'import {connect} from "node:net"; const peer=connect(process.argv[2]); peer.on("data",()=>{process.stdout.write(Buffer.concat([Buffer.from("中文").subarray(1),Buffer.from("complete\\\"}")]),()=>{peer.end();});});\n');
    await writeFile(parent, 'import {spawn} from "node:child_process"; process.stdout.write(Buffer.concat([Buffer.from("{\\\"metadata\\\":\\\""),Buffer.from("中文").subarray(0,1)])); const writer=spawn(process.execPath,[process.argv[2],process.argv[3]],{stdio:["ignore",process.stdout,"inherit"]}); writer.unref();\n');
    let settled = false;
    let parentExit;
    const output = runCommand(process.execPath, [parent, child, socketPath], {
      cwd: root, environment: { PATH: '/usr/bin:/bin' }, capture: true,
      spawnCommand: (program, args, options) => {
        const actual = spawn(program, args, options);
        parentExit = once(actual, 'exit');
        return actual;
      },
    }).finally(() => { settled = true; });
    await once(server, 'connection');
    // This is the real direct child's exit. Its inherited stdout is still
    // held by the connected writer until the test explicitly releases it.
    expect(await parentExit).toEqual([0, null]);
    expect(settled).toBe(false);
    peer.write('finish');
    expect(JSON.parse(await output)).toEqual({ metadata: '中文complete' });
  } finally {
    peer?.end('finish');
    await new Promise(accept => server.close(accept));
    await rm(root, { recursive: true, force: true });
  }
}, 5000);
