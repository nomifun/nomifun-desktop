/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useEffect, useMemo, useState } from 'react';
import { ipcBridge } from '@/common';
import type {
  CompanionExposure,
  ICompanionProfile,
  ICompanionWithStatus,
  IFigureMeta,
} from '@/common/adapter/ipcBridge';
import { useCompanions } from '@renderer/pages/nomi/useNomi';
import { figureToCustomPatch } from '@renderer/pages/nomi/useFigures';
import { CUSTOM_CHARACTER_ID } from '@renderer/pages/companion/characters';

/** The exposure posture that marks a companion as an 外呼员工 (outbound employee). */
export const PUBLIC_SERVICE_EXPOSURE: CompanionExposure = 'public_service';

/** Whether a companion profile is an outbound employee (public-service posture). */
export const isOutboundEmployee = (p: { exposure?: CompanionExposure }): boolean =>
  p.exposure === PUBLIC_SERVICE_EXPOSURE;

/** Per-employee reachability counters surfaced on the roster cards. */
export interface EmployeeStats {
  /** Configured IM channel rows bound to this employee. */
  channelCount: number;
  /** Of those, how many are enabled (actively greeting). */
  activeChannelCount: number;
  /** Public knowledge bases this employee can retrieve (0 when the binding is off). */
  kbCount: number;
}

const EMPTY_STATS: EmployeeStats = { channelCount: 0, activeChannelCount: 0, kbCount: 0 };

/**
 * Outbound-employee roster + reachability stats + hire/retire actions.
 *
 * The roster is the subset of desktop 伙伴 whose exposure is `public_service`.
 * Channel counts come from one `GET /plugins` snapshot (kept live via the
 * plugin-status WS event); public-KB counts are read per employee from the
 * companion knowledge binding (small N — a handful of employees).
 */
export const useOutbound = () => {
  const { companions, loading, refresh } = useCompanions();

  const employees = useMemo<ICompanionWithStatus[]>(
    () => companions.filter(isOutboundEmployee),
    [companions]
  );

  const [channelStats, setChannelStats] = useState<Record<string, { total: number; active: number }>>({});
  const [kbCounts, setKbCounts] = useState<Record<string, number>>({});

  // ── Channel reachability (one snapshot, live via WS) ──
  const refreshChannels = useCallback(async () => {
    try {
      const plugins = await ipcBridge.channel.getPluginStatus.invoke();
      const byCompanion: Record<string, { total: number; active: number }> = {};
      for (const pl of plugins ?? []) {
        // Real rows carry an encrypted config (hasToken); the `/plugins` list pads
        // every builtin platform with a placeholder row — skip those.
        if (!pl.companionId || !pl.hasToken) continue;
        const s = byCompanion[pl.companionId] ?? { total: 0, active: 0 };
        s.total += 1;
        if (pl.enabled) s.active += 1;
        byCompanion[pl.companionId] = s;
      }
      setChannelStats(byCompanion);
    } catch {
      /* ignore — counters refresh on the next event */
    }
  }, []);

  useEffect(() => {
    void refreshChannels();
    const unsub = ipcBridge.channel.pluginStatusChanged.on(() => void refreshChannels());
    return () => unsub();
  }, [refreshChannels]);

  // ── Public-KB counts (per employee binding) ──
  const employeeIdsKey = useMemo(() => employees.map((e) => e.id).join(','), [employees]);

  const refreshKb = useCallback(async (ids: string[]) => {
    const entries = await Promise.all(
      ids.map(async (id) => {
        try {
          const b = await ipcBridge.knowledge.getBinding.invoke({ kind: 'companion', target_id: id });
          return [id, b.enabled ? b.kb_ids.length : 0] as const;
        } catch {
          return [id, 0] as const;
        }
      })
    );
    setKbCounts(Object.fromEntries(entries));
  }, []);

  useEffect(() => {
    const ids = employeeIdsKey ? employeeIdsKey.split(',') : [];
    if (ids.length) void refreshKb(ids);
    else setKbCounts({});
    const unsub = ipcBridge.knowledge.onBindingChanged.on((evt) => {
      if (evt.target_kind === 'companion' && ids.includes(evt.target_id)) void refreshKb(ids);
    });
    return () => unsub();
  }, [employeeIdsKey, refreshKb]);

  const statsOf = useCallback(
    (id: string): EmployeeStats => {
      const ch = channelStats[id];
      return {
        channelCount: ch?.total ?? 0,
        activeChannelCount: ch?.active ?? 0,
        kbCount: kbCounts[id] ?? EMPTY_STATS.kbCount,
      };
    },
    [channelStats, kbCounts]
  );

  /** Full stat + roster refresh — call after drawer edits reconcile. */
  const refreshStats = useCallback(() => {
    void refreshChannels();
    const ids = employeeIdsKey ? employeeIdsKey.split(',') : [];
    if (ids.length) void refreshKb(ids);
  }, [refreshChannels, refreshKb, employeeIdsKey]);

  /**
   * 招聘：always create a DEDICATED companion, then flip it to public_service.
   * (Locked product decision — never convert an existing private 伙伴.)
   */
  const hireEmployee = useCallback(
    async (input: { name: string; character: string; figure?: IFigureMeta | null }): Promise<ICompanionProfile> => {
      const created = await ipcBridge.companion.createCompanion.invoke({
        name: input.name,
        character: input.figure ? CUSTOM_CHARACTER_ID : input.character,
      });
      // createCompanion only takes name + character; link the library figure first.
      if (input.figure) {
        await ipcBridge.companion.patchCompanion.invoke({
          companion_id: created.id,
          patch: { appearance: { custom_figure: figureToCustomPatch(input.figure) } },
        });
      }
      const promoted = await ipcBridge.companion.setExposure.invoke({
        companion_id: created.id,
        exposure: PUBLIC_SERVICE_EXPOSURE,
      });
      await refresh();
      return promoted;
    },
    [refresh]
  );

  /** Flip one employee's exposure (启用 = public_service / 停用 = private). */
  const setExposure = useCallback(
    async (companionId: string, exposure: CompanionExposure): Promise<ICompanionProfile> => {
      const updated = await ipcBridge.companion.setExposure.invoke({ companion_id: companionId, exposure });
      await refresh();
      return updated;
    },
    [refresh]
  );

  return { employees, loading, refresh, statsOf, refreshStats, hireEmployee, setExposure };
};
