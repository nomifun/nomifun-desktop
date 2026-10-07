import { spawn } from 'node:child_process';
import { performance } from 'node:perf_hooks';

// Keep native UI observation separate from an independently supervised child.
// A deadline/forced signal is a failure even if the child eventually exits 0.
export async function superviseNativeChild(command, args, {
  deadlineMs, terminateGraceMs = 5000, onState = () => {}, ...spawnOptions
}) {
  if (!Number.isFinite(deadlineMs) || deadlineMs <= 0
      || !Number.isFinite(terminateGraceMs) || terminateGraceMs <= 0) {
    throw new Error('A positive native deadline and termination grace are required');
  }
  const started = performance.now();
  const child = spawn(command, args, { ...spawnOptions, shell: false });
  let expired = false;
  let terminateSent = false;
  let forceKillSent = false;
  let forceTimer;
  let observerError;
  const deadlineAt = Date.now() + deadlineMs;
  const emit = state => {
    try { onState({ pid: child.pid, deadlineAt, ...state }); }
    catch (error) { observerError ??= error; }
  };
  const stop = () => {
    if (child.exitCode !== null || child.signalCode !== null || !child.pid) return;
    terminateSent = child.kill('SIGTERM');
    forceTimer ??= setTimeout(() => {
      if (child.exitCode === null && child.signalCode === null) {
        forceKillSent = child.kill('SIGKILL');
      }
    }, terminateGraceMs);
  };
  const timer = setTimeout(() => {
    expired = true;
    emit({ running: false, expired: true });
    stop();
  }, deadlineMs);
  try {
    const outcome = await new Promise((resolve, reject) => {
      child.once('error', reject);
      child.once('close', (exitCode, signal) => resolve({ exitCode, signal }));
      child.once('spawn', () => {
        emit({ running: true, expired: false });
        if (observerError) stop();
      });
    });
    const elapsedMs = performance.now() - started;
    const result = { ...outcome, pid: child.pid, elapsedMs,
      expired: expired || elapsedMs > deadlineMs, terminateSent, forceKillSent,
      observationFailed: Boolean(observerError) };
    emit({ running: false, ...result });
    if (observerError) throw observerError;
    return result;
  } finally {
    clearTimeout(timer);
    clearTimeout(forceTimer);
  }
}
