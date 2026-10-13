// Tauri kills its cargo runner on a Rust watch restart. Keep the native app a
// LaunchServices App supervised by the surviving development launcher. Each
// generation needs authenticated cleanup and the actual App's kernel exit.
import { createServer, connect } from 'node:net';
import { mkdtemp, chmod, rm } from 'node:fs/promises';
import { basename, join, resolve, relative, isAbsolute } from 'node:path';
import { launchMacosApplication, MacosApplicationLaunchCancelled } from './macos-application-launch.mjs';

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
  if (!socket.destroyed && !socket.writableEnded) socket.write(`${JSON.stringify(value)}\n`);
};

export function validateDevelopmentLaunch(request, root) {
  const cache = join(resolve(root), 'target', 'macos-dev-app');
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
  const controllerProgram = resolve(request.controllerProgram ?? '');
  const controllerWithin = relative(cache, controllerProgram);
  if (!controllerWithin || controllerWithin.startsWith('..') || isAbsolute(controllerWithin)
    || basename(controllerProgram) !== 'macos_application_controller') {
    throw new Error('macOS development launch requires its owned compiled LaunchServices controller');
  }
  return { program, controllerProgram, args: request.args, environment: request.environment, cwd };
}

export async function createMacosDevSupervisor({
  root, createLifetime, launchApplication = launchMacosApplication,
  validateLaunch = request => validateDevelopmentLaunch(request, root),
  shutdownTimeoutMs = 120_000, createControlServer = createServer,
}) {
  const directory = await mkdtemp('/tmp/nomifun-dev-host-');
  await chmod(directory, 0o700);
  const socketPath = join(directory, 'control.sock');
  const connections = new Map();
  let stopping = false;
  let queue = Promise.resolve();
  let failure;

  const server = createControlServer(socket => {
    let acquired = false;
    let applicationStarted = false;
    let leaseStopping = false;
    let resolveLaunch;
    let resolveStop;
    const launched = new Promise(accept => { resolveLaunch = accept; });
    const stopRequested = new Promise(accept => { resolveStop = accept; });
    const halted = () => stopping || leaseStopping || socket.destroyed || socket.readableEnded;
    const requestStop = () => {
      leaseStopping = true;
      resolveStop();
      // Before App launch there is no native exit proof to await. Once launch
      // can have started, retain this lease until the independent App result.
      if (!applicationStarted) socket.destroy();
    };
    connections.set(socket, requestStop);
    socket.on('error', () => socket.destroy());
    socket.once('end', requestStop);
    socket.once('close', () => {
      connections.delete(socket);
      requestStop();
    });
    readFrames(socket, request => {
      if (!acquired && request.command === 'acquire') {
        acquired = true;
        const previous = queue;
        queue = (async () => {
          await previous;
          if (failure) throw failure;
          if (halted()) return;
          const ownLifetime = async () => {
            const lifetime = await createLifetime();
            let shutdown;
            const owner = {
              socketPath: lifetime.socketPath,
              stop() {
                if (!shutdown) {
                  shutdown = Promise.resolve().then(() => lifetime.stop()).catch(error => {
                    failure ??= error;
                    throw error;
                  });
                  shutdown.catch(() => {});
                }
                return shutdown;
              },
            };
            // Bind this trigger to its own generation before any adapter
            // await. A late EOF must never stop a later generation instead.
            stopRequested.then(() => owner.stop()).catch(() => {});
            return owner;
          };
          let owner = await ownLifetime();
          let result;
          try {
            if (!halted()) {
              send(socket, { event: 'ready' });
              const request = await Promise.race([
                launched, stopRequested.then(() => null),
              ]);
              if (request && !halted()) {
                const launch = validateLaunch(request);
                while (!halted()) {
                  if (failure) throw failure;
                  const current = owner;
                  applicationStarted = true;
                  let application;
                  try {
                    application = await launchApplication({ ...launch,
                      environment: { ...launch.environment, NOMIFUN_DEV_LIFETIME_SOCKET: current.socketPath },
                      applicationScope: 'desktop_development',
                    }, { isLaunchAllowed: () => !halted() && !failure });
                  } catch (error) {
                    if (error instanceof MacosApplicationLaunchCancelled && halted()) break;
                    throw error;
                  }
                  if (application.evidenceDirectory) {
                    console.error(`[dev] native application logs: ${application.evidenceDirectory}`);
                  }
                  // A stop failure is observed immediately by the lifetime
                  // owner but never abandons a late handle or actual App exit.
                  result = await application.exit;
                  if (result?.code !== 0 || result.signal !== null || result.cleanupVerified !== true
                    || typeof result.restartRequested !== 'boolean') {
                    failure ??= new Error('macOS development app exited abnormally; native cleanup is unconfirmed and new launches are blocked');
                  }
                  await current.stop();
                  if (failure) throw failure;
                  // Intermediate authenticated restart is not a terminal
                  // runner event. The same control lease keeps the queue until
                  // its final App exits, or an observed stop suppresses restart.
                  if (!result.restartRequested || halted()) break;
                  owner = null;
                  owner = await ownLifetime();
                }
              }
            }
          } catch (error) {
            // Preserve a launch/exit failure before a later teardown failure
            // can replace it while unwinding this generation.
            failure ??= error;
            throw error;
          } finally {
            await owner?.stop();
          }
          if (result) send(socket, { event: 'exit', ...result });
          socket.end();
        })().catch(error => {
          // Cleanup failure closes admission for this launcher. Keep the app's
          // authority; never replace a timeout with forced termination.
          failure ??= error;
          send(socket, { event: 'error', message: failure.message });
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
    if (stopping) requestStop();
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
      for (const requestStop of connections.values()) requestStop();
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
