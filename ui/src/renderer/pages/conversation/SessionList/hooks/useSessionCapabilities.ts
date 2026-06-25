/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { AutoWorkRunState, IAutoWorkState, IIdmmState, IdmmRunState } from '@/common/adapter/ipcBridge';
import { useEffect, useState } from 'react';

type CapabilityTargetKind = 'conversation' | 'terminal';

// Composite string key for the capability maps: the numeric session id is
// namespaced by kind, so a number id is fine to interpolate here (the map key
// stays a string by design — it is not an id comparison).
export const capabilityKey = (kind: CapabilityTargetKind, id: number | string) => `${kind}:${id}`;

export type SessionCapabilitySnapshot = {
  /** `capabilityKey(kind, id)` → run_state. Only AutoWork-enabled sessions are present. */
  autowork: ReadonlyMap<string, AutoWorkRunState>;
  /** `capabilityKey(kind, id)` → run_state. Only IDMM-enabled sessions are present. */
  idmm: ReadonlyMap<string, IdmmRunState>;
};

// 模块级最近快照。AutoWork / IDMM 的 enabled 位不随会话列表的 extra 返回（后端
// 只有 per-target GET），而侧边栏不允许逐会话 N+1 查询，所以：
// - AutoWork：启动时用 requirements.tagBindings 一次批量拉全部 enabled 绑定
//   （含 conversation / terminal 两种 kind 与 run_state）；
// - IDMM：没有批量查询端点，初始为空；由 idmm.statusChanged 事件以及
//   IdmmControl 已经拿到的 GET / save 返回状态增量点亮；
// 之后都靠 WS 事件维护。Map 常驻模块级，侧边栏卸载重挂不丢已知状态。
const autoworkMap = new Map<string, AutoWorkRunState>();
const idmmMap = new Map<string, IdmmRunState>();
const listeners = new Set<() => void>();
let started = false;

const notify = () => listeners.forEach((listener) => listener());

export const getSessionCapabilitySnapshot = (): SessionCapabilitySnapshot => ({
  autowork: new Map(autoworkMap),
  idmm: new Map(idmmMap),
});

export const applyAutoWorkStateToSessionCapabilities = (
  state: Pick<IAutoWorkState, 'kind' | 'target_id' | 'enabled' | 'run_state'>
) => {
  const key = capabilityKey(state.kind, state.target_id);
  if (state.enabled) autoworkMap.set(key, state.run_state);
  else autoworkMap.delete(key);
  notify();
};

export const applyIdmmStateToSessionCapabilities = (
  state: Pick<IIdmmState, 'kind' | 'target_id' | 'enabled' | 'run_state'>
) => {
  const key = capabilityKey(state.kind, state.target_id);
  if (state.enabled) idmmMap.set(key, state.run_state);
  else idmmMap.delete(key);
  notify();
};

export const resetSessionCapabilitiesForTest = () => {
  autoworkMap.clear();
  idmmMap.clear();
  listeners.clear();
  started = false;
};

const ensureStarted = () => {
  if (started) return;
  started = true;

  void ipcBridge.requirements.tagBindings
    .invoke()
    .then((tags) => {
      let changed = false;
      for (const tag of tags ?? []) {
        for (const binding of tag.bindings) {
          autoworkMap.set(capabilityKey(binding.kind, binding.target_id), binding.run_state);
          changed = true;
        }
      }
      if (changed) notify();
    })
    .catch(() => {
      /* best-effort initial snapshot — events still correct the map */
    });

  // App-lifetime module subscriptions (deliberately never unsubscribed).
  ipcBridge.requirements.onAutoWork.on((state) => {
    applyAutoWorkStateToSessionCapabilities(state);
  });
  ipcBridge.idmm.onStatus.on((state) => {
    applyIdmmStateToSessionCapabilities(state);
  });
};

/**
 * AutoWork / IDMM enabled-state snapshot for every session, maintained as one
 * bulk fetch + WS event stream (no per-row requests). Subscribe once at the
 * SessionList level and hand the resolved run states down to the rows.
 */
export function useSessionCapabilities(): SessionCapabilitySnapshot {
  const [snapshot, setSnapshot] = useState<SessionCapabilitySnapshot>(getSessionCapabilitySnapshot);

  useEffect(() => {
    ensureStarted();
    const listener = () => setSnapshot(getSessionCapabilitySnapshot());
    listeners.add(listener);
    // Re-sync after mount: events may have landed between useState init and here.
    listener();
    return () => {
      listeners.delete(listener);
    };
  }, []);

  return snapshot;
}
