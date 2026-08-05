/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Message } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import type { ICompanionSkill } from '@/common/adapter/ipcBridge';
import { parseConversationId, type CompanionId, type CompanionSkillId } from '@/common/types/ids';
import type { CatalogSkillInfo } from './unify';

/** Registry rows arrive in pages; the list is client-sorted, so read them all. */
const PAGE = 100;
const MAX_ROWS = 600;

/**
 * All remote state the tab needs: the global Skill catalog (what CAN be
 * granted), the auto-injected default set, and this companion's own generated
 * skills. The registry list takes no scope argument any more: a generated skill
 * belongs to exactly one companion, so `companion_id` is the whole scope.
 */
export const useSkillsTabData = (companionId: CompanionId | null) => {
  const { t } = useTranslation();
  const [catalog, setCatalog] = useState<CatalogSkillInfo[]>([]);
  const [autoNames, setAutoNames] = useState<Set<string>>(new Set());
  const [catalogLoading, setCatalogLoading] = useState(true);
  const [generated, setGenerated] = useState<ICompanionSkill[]>([]);
  const [generatedLoading, setGeneratedLoading] = useState(true);
  const seqRef = useRef(0);

  useEffect(() => {
    let cancelled = false;
    setCatalogLoading(true);
    Promise.all([ipcBridge.fs.listAvailableSkills.invoke(), ipcBridge.fs.listBuiltinAutoSkills.invoke()])
      .then(([available, auto]) => {
        if (cancelled) return;
        setCatalog(available);
        setAutoNames(new Set(auto.map((item) => item.name)));
      })
      .catch((error) => {
        if (!cancelled) Message.error(String(error));
      })
      .finally(() => {
        if (!cancelled) setCatalogLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const refresh = useCallback(async () => {
    if (!companionId) {
      setGenerated([]);
      setGeneratedLoading(false);
      return;
    }
    const seq = ++seqRef.current;
    setGeneratedLoading(true);
    try {
      const items: ICompanionSkill[] = [];
      for (;;) {
        const page = await ipcBridge.companion.listSkills.invoke({
          companion_id: companionId,
          limit: PAGE,
          offset: items.length,
        });
        items.push(...page.items);
        if (page.items.length === 0 || items.length >= page.total || items.length >= MAX_ROWS) break;
      }
      if (seq === seqRef.current) setGenerated(items);
    } catch (error) {
      if (seq === seqRef.current) Message.error(String(error));
    } finally {
      if (seq === seqRef.current) setGeneratedLoading(false);
    }
  }, [companionId]);

  useEffect(() => {
    setGenerated([]);
    void refresh();
  }, [refresh]);

  // Live updates for THIS companion only: evolution runs in the background, so
  // a row can appear while the user is looking at the list.
  useEffect(() => {
    if (!companionId) return;
    const mine = (event: { companion_id: CompanionId }) => event.companion_id === companionId;
    const unsubs = [
      ipcBridge.companion.onSkillDrafted.on((event) => {
        if (mine(event)) void refresh();
      }),
      ipcBridge.companion.onSkillLearned.on((event) => {
        if (mine(event)) {
          void refresh();
          Message.success(t('nomi.skills.learnedToast', { defaultValue: '伙伴学会了一个新技能' }));
        }
      }),
      ipcBridge.companion.onSkillArchived.on((event) => {
        if (mine(event)) void refresh();
      }),
    ];
    return () => unsubs.forEach((unsub) => unsub());
  }, [companionId, refresh, t]);

  /**
   * Draft lifecycle: accept promotes to active, reject archives it. Resolves
   * false when the call failed (the error is already surfaced), so callers do
   * not announce success that did not happen.
   */
  const decide = useCallback(
    async (companionSkillId: CompanionSkillId, accept: boolean): Promise<boolean> => {
      if (!companionId) return false;
      try {
        await ipcBridge.companion.decideSkill.invoke({
          companion_id: companionId,
          companion_skill_id: companionSkillId,
          accept,
        });
        return true;
      } catch (error) {
        Message.error(String(error));
        return false;
      } finally {
        void refresh();
      }
    },
    [companionId, refresh]
  );

  /** 从会话学习: mine one work session's tool sequence into a draft skill. */
  const learnFromSession = useCallback(
    async (conversationId: string): Promise<boolean> => {
      if (!companionId) return false;
      const name = await ipcBridge.companion.draftFromSession.invoke({
        companion_id: companionId,
        conversation_id: parseConversationId(conversationId.trim()),
      });
      void refresh();
      return Boolean(name);
    },
    [companionId, refresh]
  );

  return {
    catalog,
    autoNames,
    generated,
    loading: catalogLoading || generatedLoading,
    initialLoading: (catalogLoading || generatedLoading) && generated.length === 0 && catalog.length === 0,
    refresh,
    decide,
    learnFromSession,
  };
};
