/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { ipcBridge } from '@/common';
import type { ICsAgent, ICsAgentPatch } from '@/common/adapter/ipcBridge';
import type { CsAgentId } from '@/common/types/ids';

/**
 * 客服（Customer Service）花名册 —— 面向陌生访客的客服员工列表 + 创建。
 *
 * 与「桌面伙伴」完全独立：独立数据 / 配置 / 控制台，绝不混入桌面伙伴列表或
 * 会话侧边栏。数据经 `/api/customer-service` REST 契约拉取。
 */
export const useCsAgents = () => {
  const [agents, setAgents] = useState<ICsAgent[]>([]);
  const [loading, setLoading] = useState(true);
  const requests = useRef({ active: false, latest: 0 }).current;

  const refresh = useCallback(async () => {
    if (!requests.active) return;
    const request = ++requests.latest;
    const isCurrent = () => requests.active && request === requests.latest;
    setLoading(true);
    try {
      const agents = (await ipcBridge.customerService.listAgents.invoke()) ?? [];
      if (isCurrent()) setAgents(agents);
    } catch {
      if (isCurrent()) setAgents([]);
    } finally {
      if (isCurrent()) setLoading(false);
    }
  }, [requests]);

  useEffect(() => {
    requests.active = true;
    void refresh();
    return () => { requests.active = false; requests.latest++; };
  }, [refresh, requests]);

  const create = useCallback(
    async (input: { name: string } & ICsAgentPatch): Promise<ICsAgent> => {
      const created = await ipcBridge.customerService.createAgent.invoke(input);
      await refresh();
      return created;
    },
    [refresh]
  );

  return { agents, loading, refresh, create };
};

/**
 * 单个客服员工的档案 + 乐观 PATCH 通道。乐观更新本地状态，失败则回读权威值。
 */
export const useCsAgent = (csAgentId: CsAgentId | null) => {
  const [agent, setAgent] = useState<ICsAgent | null>(null);
  const [loading, setLoading] = useState(true);
  // Each visit owns its callbacks, including an A -> B -> A route change.
  const requests = useMemo(() => ({ active: false, latest: 0 }), [csAgentId]);

  const load = useCallback(async () => {
    if (!requests.active) return;
    if (!csAgentId) {
      setAgent(null);
      setLoading(false);
      return;
    }
    const request = ++requests.latest;
    const isCurrent = () => requests.active && request === requests.latest;
    setLoading(true);
    try {
      const agent = await ipcBridge.customerService.getAgent.invoke({ cs_agent_id: csAgentId });
      if (isCurrent()) setAgent(agent);
    } catch {
      if (isCurrent()) setAgent(null);
    } finally {
      if (isCurrent()) setLoading(false);
    }
  }, [csAgentId, requests]);

  useLayoutEffect(() => {
    requests.active = true;
    setAgent(null);
    void load();
    return () => { requests.active = false; requests.latest++; };
  }, [load, requests]);

  const patch = useCallback(
    async (p: ICsAgentPatch): Promise<ICsAgent | undefined> => {
      if (!csAgentId || !requests.active) return undefined;
      requests.latest++;
      setAgent((prev) => (prev ? { ...prev, ...p } as ICsAgent : prev));
      try {
        const updated = await ipcBridge.customerService.patchAgent.invoke({
          cs_agent_id: csAgentId,
          patch: p,
        });
        if (requests.active) {
          // A GET begun before this write settled cannot undo its response.
          requests.latest++;
          setAgent(updated);
          setLoading(false);
        }
        return updated;
      } catch (e) {
        // Re-sync to the authoritative record so the UI never lies after a failed save.
        await load();
        throw e;
      }
    },
    [csAgentId, load, requests]
  );

  return { agent, loading, reload: load, patch };
};
