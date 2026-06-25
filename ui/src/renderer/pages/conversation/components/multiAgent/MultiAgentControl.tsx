/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Message, Popover, Radio, Select, Switch, Tooltip } from '@arco-design/web-react';
import { Peoples, Plus, Close } from '@icon-park/react';
import { ipcBridge } from '@/common';
import type { TChatConversation } from '@/common/config/storage';
import type { AcpModelInfo } from '@/common/types/platform/acpTypes';
import { CAPABILITY_COLORS } from '@/renderer/components/capability/CapabilityIcon';
import { useConversationAgents } from '@/renderer/pages/conversation/hooks/useConversationAgents';
import { useModelProviderList } from '@/renderer/hooks/agent/useModelProviderList';
import {
  cliAgentToOption,
  assistantToOption,
  filterTeamSupportedAgents,
  AgentOptionLabel,
  resolveConversationType,
  type TeamAgentOption,
} from './agentSelectUtils';
import { resolveDefaultTeamAgentModel } from './teamCreateModelResolver';
import {
  defaultMultiAgentConfig,
  isMultiAgentConfigReady,
  normalizeMultiAgentConfig,
  type TMultiAgentConfig,
  type TMultiAgentManualAgent,
  type TMultiAgentMode,
} from './multiAgentConfig';
import { useMultiAgentTeam } from './useMultiAgentTeam';

export type MultiAgentTarget = {
  /** Only chat conversations carry multi-agent config — terminals never do. */
  kind: 'conversation';
  /** conversation id — backend INTEGER (numeric-id spec §1). */
  id: number;
};

/** Draft (pre-creation) multi-agent config: held by the parent (Guid page) and
 * applied once the conversation exists. Mirrors AutoWorkDraft / IdmmDraft. */
export type MultiAgentDraft = {
  value: TMultiAgentConfig;
  onChange: (next: TMultiAgentConfig) => void;
};

type MultiAgentControlProps = {
  /** The conversation this control configures. Omit when using `draft` mode. */
  target?: MultiAgentTarget;
  /** Controlled draft state — when set, the control never reads or writes the
   * backend; `target` is ignored (the parent persists after create). */
  draft?: MultiAgentDraft;
  /** When set, the control is disabled and this reason shows as a tooltip. */
  disabledReason?: string;
  /** When the config takes effect, shown at the panel bottom (draft mode). */
  applyNote?: string;
};

/**
 * Per-conversation "multi-agent collaboration" control (spec §6). Sits next to
 * AutoWork / IDMM / Knowledge in a conversation header (and as a draft in the
 * Guid action row). A compact button opens a popover with the mode picker
 * (auto / manual) and — in manual mode — the subagent roster editor.
 *
 * Scope: config + persistence only. Enabling persists `extra.multi_agent.enabled`
 * via `conversation.update` (merge_extra) — it does NOT build the team. The
 * runtime team-ensure (lead = this conversation) is handled at send time
 * (Task 14); the status dot therefore reflects the persisted `enabled` flag,
 * not live subagent activity.
 */
const MultiAgentControl: React.FC<MultiAgentControlProps> = ({ target, draft, disabledReason, applyNote }) => {
  const { t } = useTranslation();
  const id = target?.id;
  const isDraft = !!draft;
  const draftRef = useRef(draft);
  draftRef.current = draft;

  const [persistedCfg, setPersistedCfg] = useState<TMultiAgentConfig>(defaultMultiAgentConfig);
  const cfg = draft ? draft.value : persistedCfg;
  // Apply a config change. Draft mode reports upward; live (non-draft) mode sets
  // local state and — when already enabled — PATCHes the new config so roster /
  // mode edits stay in sync without an off/on cycle. Always takes the fully
  // computed next value, so it never PATCHes a stale closure read.
  const mutate = (next: TMultiAgentConfig) => {
    const d = draftRef.current;
    if (d) {
      d.onChange(next);
      return;
    }
    setPersistedCfg(next);
    if (id && next.enabled) {
      void ipcBridge.conversation.update
        .invoke({ id, updates: { extra: { multi_agent: next } } as unknown as Partial<TChatConversation>, merge_extra: true })
        .catch(() => {
          /* ignore — the next toggle will resync */
        });
    }
  };

  // Runtime team binding (Task 14): enabling builds/reuses a team with this
  // conversation as lead and starts orchestration; disabling stops it. The hook
  // owns rollback of the persisted `enabled` flag on failure. Inert in draft
  // mode (no conversation yet) — `id` is undefined there.
  const team = useMultiAgentTeam(id);

  // Available agents (CLI engines + preset assistants), filtered to team-capable
  // ones — the same union TeamCreateModal built before team mode was folded into
  // the conversation header.
  const { cliAgents, presetAssistants } = useConversationAgents();
  const { providers, getAvailableModels } = useModelProviderList();

  const teamCapableKeys = useMemo(
    () =>
      new Set(
        cliAgents
          .filter((a) => a.team_capable)
          .flatMap((a) => [a.id, a.backend, a.agent_type].filter(Boolean) as string[])
      ),
    [cliAgents]
  );
  const agentOptions = useMemo(() => {
    const cli = cliAgents.map(cliAgentToOption);
    const presets = presetAssistants.map((a) => assistantToOption(a, teamCapableKeys));
    return filterTeamSupportedAgents([...cli, ...presets]);
  }, [cliAgents, presetAssistants, teamCapableKeys]);

  // Seed the persisted form from the conversation's saved `extra.multi_agent`.
  useEffect(() => {
    if (isDraft || !id) return;
    let cancelled = false;
    void (async () => {
      try {
        const conv = (await ipcBridge.conversation.get.invoke({ id })) as TChatConversation | undefined;
        if (cancelled || !conv) return;
        const raw = (conv.extra as { multi_agent?: unknown } | undefined)?.multi_agent;
        setPersistedCfg(normalizeMultiAgentConfig(raw));
      } catch {
        /* ignore — keep defaults */
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [id, isDraft]);

  const enabled = cfg.enabled;
  const dotColor = enabled ? CAPABILITY_COLORS.primary : CAPABILITY_COLORS.off;
  const statusText = isDraft
    ? enabled
      ? t('guid.advanced.draftOn')
      : t('guid.advanced.draftOff')
    : enabled
      ? t('multiAgent.state.enabled')
      : t('multiAgent.state.off');

  /** Model options for a given backend: ACP backends read the agent handshake's
   * `available_models`; nomi reads the configured provider models. */
  const modelOptionsFor = (backend: string | undefined): Array<{ label: string; value: string }> => {
    if (!backend) return [];
    const convType = resolveConversationType(backend);
    if (convType === 'nomi') {
      // Flatten every provider's enabled models into one option list.
      const seen = new Set<string>();
      const opts: Array<{ label: string; value: string }> = [];
      for (const p of providers) {
        for (const m of getAvailableModels(p)) {
          if (seen.has(m)) continue;
          seen.add(m);
          opts.push({ label: m, value: m });
        }
      }
      return opts;
    }
    // ACP backends: pull the handshake model list off the matching agent row.
    const matched = cliAgents.find((a) => (a.backend ?? a.agent_type) === backend);
    const handshake = matched?.handshake?.available_models as AcpModelInfo | undefined;
    return (handshake?.available_models ?? []).map((m) => ({ label: m.label || m.id, value: m.id }));
  };

  const manualAgents = cfg.manual_agents ?? [];

  const setMode = (mode: TMultiAgentMode) => mutate({ ...cfg, mode });

  const addAgent = async () => {
    // Default the new row to the first team-capable backend with a resolved model.
    const first = agentOptions[0];
    const backend = first?.backend ?? '';
    let model = '';
    if (backend) {
      try {
        model = await resolveDefaultTeamAgentModel({
          agent_type: backend,
          conversation_type: resolveConversationType(backend),
        });
      } catch {
        model = '';
      }
    }
    mutate({ ...cfg, manual_agents: [...(cfg.manual_agents ?? []), { backend, model }] });
  };

  const updateAgent = (idx: number, patch: Partial<TMultiAgentManualAgent>) =>
    mutate({
      ...cfg,
      manual_agents: (cfg.manual_agents ?? []).map((a, i) => (i === idx ? { ...a, ...patch } : a)),
    });

  const removeAgent = (idx: number) =>
    mutate({ ...cfg, manual_agents: (cfg.manual_agents ?? []).filter((_, i) => i !== idx) });

  const onAgentBackendChange = async (idx: number, backend: string) => {
    // Backend changed → resolve a fresh default model so the runtime never gets
    // a bare backend name (see teamCreateModelResolver header).
    let model = '';
    try {
      model = await resolveDefaultTeamAgentModel({
        agent_type: backend,
        conversation_type: resolveConversationType(backend),
      });
    } catch {
      model = '';
    }
    updateAgent(idx, { backend, model });
  };

  const toggle = async (next: boolean) => {
    if (next && !isMultiAgentConfigReady({ ...cfg, enabled: next })) {
      Message.warning(t('multiAgent.manual.incomplete'));
      return;
    }
    const nextCfg: TMultiAgentConfig = { ...cfg, enabled: next };
    const d = draftRef.current;
    if (d) {
      // Draft mode: just report upward — the parent persists after create.
      d.onChange(nextCfg);
      return;
    }
    if (!id) return;
    try {
      await ipcBridge.conversation.update.invoke({
        id,
        updates: { extra: { multi_agent: nextCfg } } as unknown as Partial<TChatConversation>,
        merge_extra: true,
      });
      setPersistedCfg(nextCfg);
    } catch (e) {
      Message.error(String(e));
      return;
    }
    // Config persisted — now bind the runtime team. enable() builds/reuses the
    // team (lead = this conversation) and starts orchestration; disable() stops
    // it. On enable failure the hook has already PATCHed `enabled` back to
    // false, so we mirror that locally and skip the success toast.
    if (next) {
      const ok = await team.enable(nextCfg);
      if (!ok) {
        setPersistedCfg({ ...nextCfg, enabled: false });
        return;
      }
      Message.success(t('multiAgent.enabledOk'));
    } else {
      await team.disable();
      Message.success(t('multiAgent.disabledOk'));
    }
  };

  // Persisting roster/mode edits while enabled is handled by `mutate` (it
  // PATCHes the computed next config), so there is no separate roster-sync path.

  const panel = (
    <div className='flex flex-col gap-10px w-300px p-4px'>
      <div className='text-t-primary text-13px font-600'>{t('multiAgent.label')}</div>
      <div className='text-t-tertiary text-12px leading-16px'>{t('multiAgent.hint')}</div>

      <div className='flex flex-col gap-4px'>
        <span className='text-t-secondary text-12px'>{t('multiAgent.modeLabel')}</span>
        <Radio.Group type='button' size='small' value={cfg.mode} onChange={(m: TMultiAgentMode) => setMode(m)}>
          <Radio value='auto'>{t('multiAgent.mode.auto')}</Radio>
          <Radio value='manual'>{t('multiAgent.mode.manual')}</Radio>
        </Radio.Group>
      </div>

      {cfg.mode === 'auto' ? (
        <div className='text-t-tertiary text-12px leading-16px'>{t('multiAgent.auto.hint')}</div>
      ) : (
        <div className='flex flex-col gap-8px'>
          {manualAgents.length === 0 ? (
            <div className='text-t-tertiary text-12px leading-16px'>{t('multiAgent.manual.empty')}</div>
          ) : null}
          {manualAgents.map((agent, idx) => {
            const selectedOption: TeamAgentOption | undefined = agentOptions.find((o) => o.backend === agent.backend);
            return (
              <div key={idx} className='flex flex-col gap-4px border-b border-[rgb(var(--gray-3))] pb-8px'>
                <div className='flex items-center gap-6px'>
                  <Select
                    size='small'
                    className='flex-1'
                    placeholder={t('multiAgent.manual.agentForm')}
                    value={agent.backend || undefined}
                    onChange={(v: string) => void onAgentBackendChange(idx, v)}
                    renderFormat={() =>
                      selectedOption ? <AgentOptionLabel agent={selectedOption} /> : t('multiAgent.manual.agentForm')
                    }
                  >
                    {agentOptions.map((o) => (
                      <Select.Option key={o.backend ?? o.id} value={o.backend ?? o.id}>
                        <AgentOptionLabel agent={o} />
                      </Select.Option>
                    ))}
                  </Select>
                  <Tooltip content={t('multiAgent.manual.remove')}>
                    <Button
                      size='mini'
                      shape='circle'
                      type='text'
                      icon={<Close theme='outline' size='14' />}
                      onClick={() => {
                        removeAgent(idx);
                      }}
                    />
                  </Tooltip>
                </div>
                <Select
                  size='small'
                  placeholder={t('multiAgent.manual.model')}
                  value={agent.model || undefined}
                  allowClear
                  options={modelOptionsFor(agent.backend)}
                  onChange={(v: string | undefined) => {
                    updateAgent(idx, { model: v ?? '' });
                  }}
                />
              </div>
            );
          })}
          <Button
            size='mini'
            type='text'
            icon={<Plus theme='outline' size='14' />}
            onClick={() => {
              void addAgent();
            }}
            className='self-start'
          >
            {t('multiAgent.manual.addAgent')}
          </Button>
        </div>
      )}

      <div className='flex items-center justify-between border-t border-[rgb(var(--gray-3))] pt-8px'>
        <span className='inline-flex items-center gap-6px text-t-secondary text-12px'>
          <span className='inline-block w-6px h-6px rounded-full' style={{ backgroundColor: dotColor }} />
          {statusText}
        </span>
        <Switch checked={enabled} onChange={toggle} />
      </div>
      {applyNote ? <div className='text-t-quaternary text-11px leading-15px'>{applyNote}</div> : null}
    </div>
  );

  const button = (
    <Button
      size='mini'
      shape='round'
      type={enabled ? 'primary' : 'secondary'}
      disabled={!!disabledReason}
      className='shrink-0'
    >
      <span className='inline-flex items-center gap-6px leading-none'>
        <Peoples theme='outline' size='14' fill='currentColor' className='block' style={{ lineHeight: 0 }} />
        <span className='text-12px'>{t('multiAgent.label')}</span>
        <span className='inline-block w-6px h-6px rounded-full' style={{ backgroundColor: dotColor }} />
      </span>
    </Button>
  );

  if (disabledReason) {
    return (
      <Tooltip content={disabledReason}>
        <span className='inline-flex'>{button}</span>
      </Tooltip>
    );
  }

  return (
    <Popover trigger='click' position='br' content={panel}>
      {button}
    </Popover>
  );
};

export default MultiAgentControl;
