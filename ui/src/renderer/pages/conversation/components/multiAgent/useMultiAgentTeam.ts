/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Runtime binding for the session-level "multi-agent collaboration" switch
 * (spec §6, Task 14). The persisted config layer lives on the conversation's
 * `extra.multi_agent` (Task 13 — see `multiAgentConfig.ts` and the toggle in
 * `MultiAgentControl.tsx`). This hook turns that switch into a real team:
 *
 *   enable(cfg)  — the CURRENT conversation becomes the team lead. We adopt it
 *                  into a fresh team (`POST /api/teams` with the lead carrying
 *                  `conversation_id`, which the backend wires in by writing
 *                  `extra.teamId`) and start the orchestration event loop
 *                  (`ensureSession`). In manual mode each declared subagent is
 *                  added as a teammate; in auto mode only the lead is created
 *                  and the leader spawns teammates at runtime via the
 *                  `team_spawn_agent` MCP tool. If the conversation already has
 *                  an `extra.teamId`, we reuse that team and only ensure the
 *                  session.
 *
 *   disable()    — stop the orchestration (`DELETE /api/teams/{id}/session`)
 *                  but KEEP the team record and `extra.teamId` so a later
 *                  enable() reuses the same team instead of rebuilding it.
 *
 * Failure semantics: enable() rolls the persisted `extra.multi_agent.enabled`
 * flag back to `false` and surfaces a `Message.error`, so the UI never ends up
 * with "config says on, but no team exists". Concurrent enable/disable calls on
 * the same conversation are guarded by an in-flight ref.
 *
 * NOTE: we POST `/api/teams` directly (via `httpRequest`) rather than going
 * through `ipcBridge.team.create`, because the shared `toBackendAgent` mapper
 * deliberately strips `conversation_id` (the standalone TeamCreateModal always
 * created fresh conversations). Lead adoption needs that field, so the body is
 * assembled here. The response is mapped with the shared `fromBackendTeam`.
 */

import { useCallback, useRef } from 'react';
import { Message } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import { httpRequest } from '@/common/adapter/httpBridge';
import { fromBackendTeam, toBackendAgent } from '@/common/adapter/teamMapper';
import { ipcBridge } from '@/common';
import type { TChatConversation } from '@/common/config/storage';
import type { TTeam } from '@/common/types/team/teamTypes';
import type { TMultiAgentConfig } from './multiAgentConfig';

/** Conversation `extra` fields this hook reads. The backend writes `teamId`
 *  (camelCase — see nomifun-team `create_team`) when a conversation is adopted;
 *  older rows may carry the snake_case `team_id`, so we tolerate both. */
type MultiAgentConversationExtra = {
  teamId?: string;
  team_id?: string;
  workspace?: string;
  backend?: string;
  current_model_id?: string;
  multi_agent?: TMultiAgentConfig;
};

export type UseMultiAgentTeam = {
  /** Build (or reuse) the team for this conversation and start orchestration. */
  enable: (cfg: TMultiAgentConfig) => Promise<boolean>;
  /** Stop orchestration; keep the team record + `extra.teamId` for reuse. */
  disable: () => Promise<boolean>;
};

/** Read `extra.teamId` (tolerating the legacy `team_id`). */
function readTeamId(extra: MultiAgentConversationExtra | undefined): string | undefined {
  const id = extra?.teamId ?? extra?.team_id;
  return id && id.length > 0 ? id : undefined;
}

/**
 * Derive the lead agent's execution backend from the conversation. ACP-family
 * conversations carry the concrete backend in `extra.backend`; nomi/codex/etc.
 * conversations use their `type` as the backend token (matching the backend's
 * own `parse_agent_type`). Falls back to `'nomi'` for the rare untyped row.
 */
function resolveLeadBackend(conv: TChatConversation, extra: MultiAgentConversationExtra | undefined): string {
  if (conv.type === 'acp') return extra?.backend ?? 'acp';
  return conv.type || extra?.backend || 'nomi';
}

/**
 * Derive the lead agent's model. nomi conversations carry the resolved model on
 * the top-level `model` field; other backends persist it in
 * `extra.current_model_id`. `'default'` is the backend's accepted sentinel when
 * neither is set.
 */
function resolveLeadModel(conv: TChatConversation, extra: MultiAgentConversationExtra | undefined): string {
  const m = conv as TChatConversation & { model?: { model?: string } | string };
  if (typeof m.model === 'string' && m.model) return m.model;
  if (m.model && typeof m.model === 'object' && typeof m.model.model === 'string' && m.model.model) {
    return m.model.model;
  }
  return extra?.current_model_id || 'default';
}

/** Raw `/api/teams` agent payload — mirrors backend `TeamAgentInput`. The lead
 *  carries `conversation_id` (adoption); teammates omit it (fresh conversation). */
type RawTeamAgentBody = {
  name: string;
  role: string;
  backend: string;
  model: string;
  custom_agent_id?: string;
  /** conversation id — backend INTEGER (numeric-id spec §1). */
  conversation_id?: number;
};

export function useMultiAgentTeam(conversationId: number | undefined): UseMultiAgentTeam {
  const { t } = useTranslation();
  // Guards re-entrancy: a second enable/disable for the same conversation while
  // one is in flight is a no-op (returns the in-flight promise's settled value).
  const inFlight = useRef<Promise<boolean> | null>(null);

  /** Best-effort rollback: flip the persisted `enabled` flag back to false so a
   *  failed enable doesn't leave the config claiming the team is on. Merges
   *  extra so the rest of `multi_agent` (mode / roster) is preserved. */
  const rollbackEnabled = useCallback(
    async (cfg: TMultiAgentConfig) => {
      if (!conversationId) return;
      try {
        await ipcBridge.conversation.update.invoke({
          id: conversationId,
          updates: {
            extra: { multi_agent: { ...cfg, enabled: false } },
          } as unknown as Partial<TChatConversation>,
          merge_extra: true,
        });
      } catch {
        /* ignore — the next toggle resyncs; the error toast already fired */
      }
    },
    [conversationId]
  );

  const runEnable = useCallback(
    async (cfg: TMultiAgentConfig): Promise<boolean> => {
      if (!conversationId) return false;
      let conv: TChatConversation | undefined;
      try {
        conv = (await ipcBridge.conversation.get.invoke({ id: conversationId })) as TChatConversation | undefined;
      } catch {
        conv = undefined;
      }
      if (!conv) {
        Message.error(t('common.error'));
        await rollbackEnabled(cfg);
        return false;
      }
      const extra = conv.extra as MultiAgentConversationExtra | undefined;

      // Reuse path: the conversation is already wired into a team — just make
      // sure the orchestration session is running.
      const existingTeamId = readTeamId(extra);
      if (existingTeamId) {
        try {
          await ipcBridge.team.ensureSession.invoke({ team_id: existingTeamId });
          return true;
        } catch (e) {
          Message.error(`${t('common.error')}: ${String(e)}`);
          await rollbackEnabled(cfg);
          return false;
        }
      }

      // Build path: adopt the current conversation as the team lead.
      const leadBackend = resolveLeadBackend(conv, extra);
      const leadModel = resolveLeadModel(conv, extra);
      const agents: RawTeamAgentBody[] = [
        {
          name: conv.name || leadBackend,
          role: 'lead',
          backend: leadBackend,
          model: leadModel || 'default',
          conversation_id: conversationId,
        },
      ];

      // Manual mode: pre-declare the user's roster as teammates. Reuse the
      // shared `toBackendAgent` mapper (it intentionally omits conversation_id,
      // which is exactly right for teammates — the backend creates their
      // conversations). Auto mode adds nothing here: the leader spawns
      // teammates at runtime via the team_spawn_agent MCP tool.
      if (cfg.mode === 'manual') {
        for (const a of cfg.manual_agents ?? []) {
          if (!a.backend) continue;
          const mapped = toBackendAgent({
            role: 'teammate',
            agent_type: a.backend,
            agent_name: a.name || a.backend,
            model: a.model,
            status: 'pending',
            conversation_type: '',
          });
          agents.push(mapped as unknown as RawTeamAgentBody);
        }
      }

      const body: { name: string; agents: RawTeamAgentBody[]; workspace?: string } = {
        name: conv.name || t('multiAgent.label'),
        agents,
        ...(extra?.workspace ? { workspace: extra.workspace } : {}),
      };

      let team: TTeam;
      try {
        const raw = await httpRequest<unknown>('POST', '/api/teams', body);
        team = fromBackendTeam(raw);
      } catch (e) {
        Message.error(`${t('common.error')}: ${String(e)}`);
        await rollbackEnabled(cfg);
        return false;
      }
      if (!team.id) {
        Message.error(t('common.error'));
        await rollbackEnabled(cfg);
        return false;
      }

      // Start the orchestration event loop. The backend already auto-ensures a
      // session after create_team, but calling it explicitly is idempotent and
      // makes the contract (enable ⇒ running session) self-evident.
      try {
        await ipcBridge.team.ensureSession.invoke({ team_id: team.id });
      } catch (e) {
        Message.error(`${t('common.error')}: ${String(e)}`);
        await rollbackEnabled(cfg);
        return false;
      }
      return true;
    },
    [conversationId, rollbackEnabled, t]
  );

  const runDisable = useCallback(async (): Promise<boolean> => {
    if (!conversationId) return false;
    let conv: TChatConversation | undefined;
    try {
      conv = (await ipcBridge.conversation.get.invoke({ id: conversationId })) as TChatConversation | undefined;
    } catch {
      conv = undefined;
    }
    const teamId = readTeamId(conv?.extra as MultiAgentConversationExtra | undefined);
    // No team to stop — disabling is a no-op (config-only state).
    if (!teamId) return true;
    try {
      // Keep the team record + extra.teamId; only stop the running session.
      await ipcBridge.team.stop.invoke({ team_id: teamId });
      return true;
    } catch (e) {
      Message.error(`${t('common.error')}: ${String(e)}`);
      return false;
    }
  }, [conversationId, t]);

  const enable = useCallback(
    (cfg: TMultiAgentConfig): Promise<boolean> => {
      if (inFlight.current) return inFlight.current;
      const p = runEnable(cfg).finally(() => {
        inFlight.current = null;
      });
      inFlight.current = p;
      return p;
    },
    [runEnable]
  );

  const disable = useCallback((): Promise<boolean> => {
    if (inFlight.current) return inFlight.current;
    const p = runDisable().finally(() => {
      inFlight.current = null;
    });
    inFlight.current = p;
    return p;
  }, [runDisable]);

  return { enable, disable };
}
