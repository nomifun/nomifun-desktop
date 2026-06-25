/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * In-house namespaced key-value storage.
 *
 * Replaces the storage surface of the former third-party platform package,
 * preserving its exact behavior:
 *  - Each group exposes get/set/remove backed by bridge RPC channels
 *    `"<group>.storage.{get,set,clear,remove}"`.
 *  - `interceptor(impl)` registers the providers for those channels (the main
 *    process plugs its file-backed store in here).
 *  - When `debug` is enabled, ops use a `window.localStorage` fallback with
 *    the RAW key (no group prefix) — matching the original library so existing
 *    persisted values remain readable.
 *  - The `set` RPC payload shape is `{ key, data }`; the interceptor unpacks it
 *    to positional `impl.set(key, data)`.
 */
import { buildProvider } from './bridge';

interface StorageInterceptor<S> {
  get?<K extends keyof S>(key: K): Promise<S[K]>;
  set?<K extends keyof S>(key: K, data: S[K]): Promise<S[K] | void>;
  clear?(): Promise<unknown>;
  remove?<K extends keyof S>(key: K): Promise<unknown>;
}

const hasLocalStorage = (): boolean => typeof window !== 'undefined' && !!window.localStorage;

export const buildStorage = <S extends Record<string, any> = Record<string, any>>(
  group: string,
  options?: { debug: boolean }
) => {
  const debug = !!options?.debug;

  const getCh = buildProvider<any, string>(group + '.storage.get');
  const setCh = buildProvider<any, { key: string; data: any }>(group + '.storage.set');
  const clearCh = buildProvider<any>(group + '.storage.clear');
  const removeCh = buildProvider<any, string>(group + '.storage.remove');

  // When debug, prefer the local fallback; otherwise route through the bridge.
  const route = <T>(viaBridge: () => Promise<T>, viaLocal: () => Promise<T>): Promise<T> =>
    debug ? viaLocal() : viaBridge();

  return {
    get<K extends keyof S>(key: K): Promise<S[K]> {
      return route(
        () => getCh.invoke(String(key)),
        async () => {
          if (!hasLocalStorage()) return undefined as S[K];
          const raw = window.localStorage.getItem(String(key));
          if (!raw) return undefined as S[K];
          try {
            return JSON.parse(raw) as S[K];
          } catch (e) {
            console.error(e);
            return undefined as S[K];
          }
        }
      );
    },
    set<K extends keyof S>(key: K, data: S[K]): Promise<any> {
      return route(
        () => setCh.invoke({ key: String(key), data }),
        async () => {
          if (hasLocalStorage()) window.localStorage.setItem(String(key), JSON.stringify(data));
        }
      );
    },
    clear(): Promise<any> {
      return route(
        () => clearCh.invoke(),
        async () => {
          if (hasLocalStorage()) window.localStorage.clear();
        }
      );
    },
    remove(key: keyof S): Promise<any> {
      return route(
        () => removeCh.invoke(String(key)),
        async () => {
          if (hasLocalStorage()) window.localStorage.removeItem(String(key));
        }
      );
    },
    debug(_flag: boolean): void {
      // Parity stub; routing is fixed at build time via options.debug.
    },
    /**
     * Register the backing store. Mirrors the original contract: `get`/`remove`
     * receive the raw key; `set` receives `{ key, data }` and is unpacked to
     * positional args.
     */
    interceptor(impl: StorageInterceptor<S>): void {
      if (impl.get) getCh.provider((key: string) => impl.get!(key as keyof S));
      if (impl.set)
        setCh.provider((payload: { key: string; data: any }) => impl.set!(payload.key as keyof S, payload.data));
      if (impl.clear) clearCh.provider(() => impl.clear!());
      if (impl.remove) removeCh.provider((key: string) => impl.remove!(key as keyof S));
    },
  };
};

// Top-level "global" group, mirroring the original convenience exports.
const globalStore = buildStorage('global');
export const get = globalStore.get;
export const set = globalStore.set;
export const remove = globalStore.remove;
export const interceptor = globalStore.interceptor;
export const debug = globalStore.debug;
