// Tauri kills its cargo runner on a Rust watch restart. Keep the native app a
// direct child of the surviving development launcher, so every generation can
// finish ExitCoordinator cleanup before the next one starts.
import { createServer, connect } from 'node:net';
import { mkdtemp, chmod, rm } from 'node:fs/promises';
import { join, resolve, relative, isAbsolute } from 'node:path';
import { spawn } from 'node:child_process';

const MAX_REQUEST_BYTES = 1024 * 1024;

function readFrames(socket, receive) {
  let buffered = '';
  socket.setEncoding('utf8');
  socket.on('data', data => {
    buffered += data;
    if (Buffer.byteLength(buffered) > MAX_REQUEST_BYTES) {
      socket.destroy(new Error('macOS development request exceeded its size limit'));
      return;
    }
    let newline;
    while ((newline = buffered.indexOf('\n')) >= 0) {
      const frame = buffered.slice(0, newline);
      buffered = buffered.slice(newline + 1);
      try { receive(JSON.parse(frame)); }
      catch (error) { socket.destroy(error); return; }
    }
  });
}

const send = (socket, value) => {
  if (!socket.destroyed) socket.write(`${JSON.stringify(value)}\n`);
};

export function validateDevelopmentLaunch(request, root) {
  const cache = join(resolve(root), 'target', 'macos-dev-browser');
  const program = resolve(request.program ?? '');
  const within = relative(cache, program);
  if (!within || within.startsWith('..') || isAbsolute(within)
    || !program.endsWith(join('.app', 'Contents', 'MacOS', 'nomifun-desktop'))) {
    throw new Error('macOS development launch must use the owned complete app bundle');
  }
  if (!Array.isArray(request.args) || !request.args.every(arg => typeof arg === 'string')
    || !request.environment || typeof request.environment !== 'object'
    || !Object.values(request.environment).every(value => typeof value === 'string')) {
    throw new Error('macOS development launch arguments are invalid');
  }
  const cwd = resolve(request.cwd ?? root);
  const local = relative(resolve(root), cwd);
  if (local.startsWith('..') || isAbsolute(local)) {
    throw new Error('macOS development launch directory must be in this workspace');
  }
  return { program, args: request.args, environment: request.environment, cwd };
}

export async function createMacosDevSupervisor({
  root, createLifetime, spawnProgram = spawn,
  validateLaunch = request => validateDevelopmentLaunch(request, root),
  shutdownTimeoutMs = 120_000,
}) {
  const directory = await mkdtemp('/tmp/nomifun-dev-host-');
  await chmod(directory, 0o700);
  const socketPath = join(directory, 'control.sock');
  const connections = new Set();
  let stopping = false;
  let queue = Promise.resolve();
  let failure;

  const server = createServer(socket => {
    connections.add(socket);
    let acquired = false;
    let resolveLaunch;
    let resolveClosed;
    const launched = new Promise(accept => { resolveLaunch = accept; });
    const closed = new Promise(accept => { resolveClosed = accept; });
    socket.on('error', () => socket.destroy());
    socket.once('close', () => {
      connections.delete(socket);
      resolveClosed();
    });
    readFrames(socket, request => {
      if (!acquired && request.command === 'acquire') {
        acquired = true;
        const previous = queue;
        queue = (async () => {
          await previous;
          if (failure) throw failure;
          if (stopping || socket.destroyed) return;
          const lifetime = await createLifetime();
          try {
            send(socket, { event: 'ready' });
            const request = await Promise.race([launched, closed.then(() => null)]);
            if (!request || stopping || socket.destroyed) return;
            const launch = validateLaunch(request);
            const child = spawnProgram(launch.program, launch.args, {
              cwd: launch.cwd,
              env: { ...launch.environment, NOMIFUN_DEV_LIFETIME_SOCKET: lifetime.socketPath },
              stdio: 'inherit',
              // Keep terminal Ctrl-C on run-dev. Otherwise the terminal also
              // signals CEF's child processes before native cleanup can run.
              detached: true,
            });
            const exited = new Promise((accept, reject) => {
              child.once('exit', (code, signal) => accept({ code, signal }));
              child.once('error', reject);
            });
            // Disconnect means the cargo runner was killed by Tauri's watcher.
            // Request ordinary native shutdown, then wait our own direct child.
            const result = await Promise.race([
              exited,
              closed.then(async () => { await lifetime.stop(); return exited; }),
            ]);
            if (result.code !== 0 || result.signal) {
              failure = new Error('macOS development app exited abnormally; Browser helper cleanup is unconfirmed and new launches are blocked');
            }
            send(socket, { event: 'exit', ...result });
            socket.end();
          } finally {
            await lifetime.stop();
          }
        })().catch(error => {
          // Cleanup failure closes admission for this launcher. Keep the app's
          // authority; never replace a timeout with forced termination.
          failure = error;
          send(socket, { event: 'error', message: error.message });
          socket.end();
        });
      } else if (acquired && request.command === 'launch') {
        if (!resolveLaunch) throw new Error('duplicate macOS development launch');
        resolveLaunch(request);
        resolveLaunch = null;
      } else {
        throw new Error('invalid macOS development control request');
      }
    });
    if (stopping) socket.destroy();
  });
  try {
    await new Promise((accept, reject) => {
      server.once('error', reject);
      server.listen(socketPath, accept);
    });
  } catch (error) {
    await rm(directory, { recursive: true, force: true });
    throw error;
  }
  let shutdown;
  return {
    socketPath,
    stop() {
      if (shutdown) return shutdown;
      stopping = true;
      const closed = new Promise(accept => server.close(accept));
      for (const socket of connections) socket.destroy();
      shutdown = new Promise((accept, reject) => {
        const timeout = setTimeout(() => reject(new Error(
          'macOS development app did not finish verified shutdown within 120 seconds',
        )), shutdownTimeoutMs);
        Promise.all([closed, queue]).then(async () => {
          clearTimeout(timeout);
          if (failure) throw failure;
          await rm(directory, { recursive: true, force: true });
          accept();
        }).catch(error => { clearTimeout(timeout); reject(error); });
      });
      return shutdown;
    },
  };
}

export async function acquireMacosDevGeneration(socketPath) {
  if (!socketPath) throw new Error('start macOS development with bun run dev');
  const socket = connect(socketPath);
  let resolveReady, rejectReady, resolveExit, rejectExit;
  const ready = new Promise((accept, reject) => { resolveReady = accept; rejectReady = reject; });
  const exit = new Promise((accept, reject) => { resolveExit = accept; rejectExit = reject; });
  // A killed runner abandons these waiters; ordinary failures remain observed.
  exit.catch(() => {});
  let finished = false;
  socket.on('error', error => { rejectReady(error); rejectExit(error); });
  socket.on('close', () => {
    if (!finished) {
      const error = new Error('macOS development supervisor stopped');
      rejectReady(error); rejectExit(error);
    }
  });
  readFrames(socket, result => {
    if (result.event === 'ready') resolveReady();
    else if (result.event === 'exit') { finished = true; resolveExit(result); }
    else if (result.event === 'error') {
      const error = new Error(result.message);
      rejectReady(error); rejectExit(error);
    } else throw new Error('invalid macOS development supervisor response');
  });
  socket.once('connect', () => send(socket, { command: 'acquire' }));
  await ready;
  return {
    launch(request) { send(socket, { command: 'launch', ...request }); return exit; },
    close() { socket.destroy(); },
  };
}
