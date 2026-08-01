/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * In-house bridge: transport-agnostic pub/sub + request/response RPC.
 *
 * Replaces the bridge surface of the former third-party platform package.
 * The wire protocol is preserved byte-for-byte so the Rust backend
 * (`WebSocketMessage{name,data}`), the mobile client, and the desktop
 * adapters all keep working unchanged:
 *
 *  - invoke(name, data):
 *      emits event `"subscribe-" + name` with body `{ id, data }`,
 *      then awaits a one-shot `"subscribe.callback-" + name + id`.
 *  - subscribe(name, handler):
 *      listens on `"subscribe-" + name`, runs the handler, then emits
 *      `"subscribe.callback-" + name + body.id` with the result.
 *
 * The id is concatenated to the name WITHOUT a delimiter — this matches the
 * original library and the consumers that hand-build the callback name
 * (mobile bridge.ts, useDirectorySelection.tsx). Do not change it.
 */
import EventEmitter from 'eventemitter3';

type AnyFn = (...args: any[]) => any;

interface Adapter {
  emit(name: string, data: any, ...args: any[]): void;
  on(emitter: { emit(name: string, ...args: any[]): void }): void;
}

// Internal event hub. The adapter plugs the real transport into `outbound`
// (how events leave this process) and feeds inbound events back via the
// emitter handed to `adapter.on`.
const hub = new EventEmitter();

// Default outbound: loopback (same-process). The adapter overrides this.
let outbound: (name: string, data: any, ...args: any[]) => void = (name, data, ...args) => {
  hub.emit(name, data, ...args);
};

/** Register a listener; returns a disposer. */
const on = (name: string, callback: AnyFn): (() => void) => {
  hub.on(name, callback);
  return () => hub.off(name, callback);
};

const off = (name: string, callback: AnyFn): void => {
  // Best-effort: matches the original loose contract. Listeners created via
  // `on()` return their own disposer; this removes direct hub listeners.
  hub.off(name, callback);
};

const emit = (name: string, data: any, ...args: any[]): void => {
  outbound(name, data, ...args);
};

// Monotonic, collision-resistant id. Format is opaque; only uniqueness as a
// string matters (consumers concatenate it onto the callback event name).
let idCounter = 0;
const nextId = (prefix: string): string => `${prefix}_${idCounter++}_${randomSuffix()}`;
const randomSuffix = (): string => {
  // Avoid Math.random/Date in hot equality paths is unnecessary here; this is
  // a fresh id each call. Use performance-cheap entropy.
  let s = '';
  for (let i = 0; i < 6; i++) s += ((Math.random() * 36) | 0).toString(36);
  return s;
};

const subscribe = <Data = any, Result = any>(name: string, callback: (data: Data) => Promise<Result>): (() => void) => {
  return on('subscribe-' + name, (body: { id: string; data: Data }, ...rest: any[]) => {
    Promise.resolve(callback(body.data)).then((result) => {
      emit('subscribe.callback-' + name + body.id, result, ...rest);
    });
  });
};

const invoke = <Data = any>(name: string, data?: any): Promise<Data> => {
  const id = nextId(name);
  return new Promise<Data>((resolve) => {
    // Register the one-shot callback listener BEFORE emitting the request, so a
    // synchronous responder (loopback / same-process provider) cannot deliver
    // the result before we are listening. With an async transport the order is
    // immaterial; this is correct in both cases.
    const dispose = on('subscribe.callback-' + name + id, (result: Data) => {
      resolve(result);
      dispose();
    });
    emit('subscribe-' + name, { id, data });
  });
};

/**
 * Install the real transport. `emit` ships an outbound event; `on` is handed
 * an emitter whose `.emit(name, data)` feeds inbound events into the hub.
 */
const adapter = (config: Adapter): void => {
  outbound = (name, data, ...args) => config.emit(name, data, ...args);
  config.on({
    emit: (name: string, ...args: any[]) => {
      hub.emit(name, ...args);
    },
  });
};

export { adapter, on, off, emit, subscribe, invoke };
