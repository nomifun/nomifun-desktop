/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type {
  AutoWorkRunState,
  IAutoWorkState,
  ITagBindings,
  SessionCapabilityTargetId,
} from '@/common/adapter/ipcBridge';
import { useEffect, useState } from 'react';

type CapabilityTargetKind = 'conversation';

// Composite string key for the capability maps. The business UUID is
// namespaced by kind so future target domains cannot collide.
export const capabilityKey = (kind: CapabilityTargetKind, id: SessionCapabilityTargetId) => `${kind}:${id}`;

export type SessionCapabilitySnapshot = {
  /** `capabilityKey(kind, id)` → run_state. Only AutoWork-enabled sessions are present. */
  autowork: ReadonlyMap<string, AutoWorkRunState>;
};

// 模块级最近快照。AutoWork 的 enabled 位不随会话列表的 extra 返回，
// 而侧边栏不允许逐会话 N+1 查询，所以启动时用
// requirements.tagBindings 一次批量拉取 conversation 绑定，之后靠 WS 事件维护。
// Map 常驻模块级，侧边栏卸载重挂不丢已知状态。
const autoworkMap = new Map<string, AutoWorkRunState>();
const listeners = new Set<() => void>();
let started = false;
let refreshRequest = 0;

const notify = () => listeners.forEach((listener) => listener());

export const getSessionCapabilitySnapshot = (): SessionCapabilitySnapshot => ({
  autowork: new Map(autoworkMap),
});

export const applyAutoWorkStateToSessionCapabilities = (
  state: Pick<IAutoWorkState, 'kind' | 'target_id' | 'enabled' | 'run_state'>
) => {
  // A live event is newer than any bulk request already in flight.
  refreshRequest += 1;
  const key = capabilityKey(state.kind, state.target_id);
  if (state.enabled) autoworkMap.set(key, state.run_state);
  else autoworkMap.delete(key);
  notify();
};

/** Replace, rather than merge, the durable bulk projection. This is the
 * reconnect path: events have no replay, so entries removed while the socket
 * was down must disappear from the module-lifetime cache as well. */
export const replaceAutoWorkSessionCapabilities = (groups: ITagBindings[]) => {
  autoworkMap.clear();
  for (const group of groups) {
    for (const binding of group.bindings) {
      autoworkMap.set(
        capabilityKey(binding.kind, binding.target_id),
        binding.run_state
      );
    }
  }
  notify();
};

export const resetSessionCapabilitiesForTest = () => {
  autoworkMap.clear();
  listeners.clear();
  started = false;
  refreshRequest += 1;
};

const ensureStarted = () => {
  if (started) return;
  started = true;

  const refresh = () => {
    const request = ++refreshRequest;
    return ipcBridge.requirements.tagBindings
      .invoke()
      .then((groups) => {
        if (request === refreshRequest) {
          replaceAutoWorkSessionCapabilities(groups ?? []);
        }
      })
      .catch(() => {
        /* best-effort authoritative snapshot — live events still correct the map */
      });
  };
  void refresh();

  // App-lifetime module subscriptions (deliberately never unsubscribed).
  ipcBridge.requirements.onAutoWork.on((state) => {
    applyAutoWorkStateToSessionCapabilities(state);
  });
  ipcBridge.conversation.reconnected.on(() => {
    void refresh();
  });
};

/**
 * AutoWork enabled-state snapshot for every conversation, maintained as one
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
