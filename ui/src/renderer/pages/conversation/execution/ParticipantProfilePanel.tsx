/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { AddUser, Check, Down, Experiment, Up } from '@icon-park/react';
import { ipcBridge } from '@/common';
import type { CreatePresetRequest } from '@/common/types/agent/presetTypes';
import type { TAgentExecutionDetail } from '@/common/types/agentExecution/agentExecutionTypes';
import { latestAttemptForStep } from '@/common/types/agentExecution/agentExecutionTypes';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import type { ProviderId } from '@/common/types/ids';

/** A reusable role candidate distilled from one completed execution. */
interface RoleCandidate {
  /** The role name; becomes the preset's name. */
  name: string;
  /** Short synthesized one-liner shown on the card + saved as the description. */
  description: string;
  /** Distinct models used by participants in this role. */
  models: string[];
  modelPreferences: Array<{
    provider_id?: ProviderId;
    model: string;
    required: false;
  }>;
  agentIds: string[];
  instructions: string;
  /** Union of `enabled_skills` over the role's participants. */
  enabledSkills: string[];
  /** Union of `disabled_builtin_skills` over the role's participants. */
  disabledBuiltinSkills: string[];
  /** True when a preset with this name already exists (case-insensitive). */
  exists: boolean;
}

/** Push every non-empty, de-duplicated value of `items` into `set`. */
function collect(set: Set<string>, items: readonly string[] | undefined): void {
  if (!items) return;
  for (const raw of items) {
    const v = raw?.trim();
    if (v) set.add(v);
  }
}

/**
 * Offers reusable presets for roles observed in a completed execution. Each
 * candidate retains the models, skills, and task descriptions that produced it.
 */
const ParticipantProfilePanel: React.FC<{ detail: TAgentExecutionDetail }> = ({ detail }) => {
  const { t } = useTranslation();
  const [message, ctx] = useArcoMessage();
  const [collapsed, setCollapsed] = useState(false);
  /** Names of presets that already exist, lower-cased + trimmed. */
  const [existingNames, setExistingNames] = useState<Set<string> | null>(null);
  /** Role names the user has just saved this session (for the ✓ saved state). */
  const [savedNames, setSavedNames] = useState<Set<string>>(() => new Set());
  /** Role names whose save call is currently in flight. */
  const [savingNames, setSavingNames] = useState<Set<string>>(() => new Set());

  // Map a plain vertical mouse wheel onto the lane's horizontal scroll. A bare
  // wheel does NOT scroll a horizontal-overflow container in Chromium/WebView2
  // unless Shift is held, so mouse-only users couldn't reach the overflowing
  // cards; trackpad horizontal gestures are left untouched. A callback ref
  // (re)attaches a NON-passive listener on every mount so it survives the
  // collapse / role-less null-return cycles (a plain useEffect keyed on state
  // would miss the null→cards remount). `preventDefault` needs the non-passive
  // listener — React's own onWheel is passive and would ignore it.
  const wheelCleanup = useRef<(() => void) | null>(null);
  const laneRef = useCallback((el: HTMLDivElement | null) => {
    wheelCleanup.current?.();
    wheelCleanup.current = null;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (el.scrollWidth <= el.clientWidth) return; // nothing to scroll
      if (Math.abs(e.deltaY) <= Math.abs(e.deltaX)) return; // let native horizontal through
      el.scrollLeft += e.deltaY;
      e.preventDefault();
    };
    el.addEventListener('wheel', onWheel, { passive: false });
    wheelCleanup.current = () => el.removeEventListener('wheel', onWheel);
  }, []);
  // Detach on unmount so we never leak the listener.
  useEffect(() => () => wheelCleanup.current?.(), []);

  const participantById = useMemo(() => {
    const map = new Map<string, (typeof detail.participants)[number]>();
    for (const participant of detail.participants) map.set(participant.id, participant);
    return map;
  }, [detail.participants]);

  const participantByStep = useMemo(() => {
    const map = new Map<string, (typeof detail.participants)[number]>();
    for (const step of detail.steps.filter((item) => item.superseded_in_revision == null)) {
      const attempt = latestAttemptForStep(detail.attempts, step.id);
      const participantId = attempt?.participant_id ?? step.assigned_participant_id;
      const participant = participantId ? participantById.get(participantId) : undefined;
      if (participant) map.set(step.id, participant);
    }
    return map;
  }, [detail.attempts, detail.steps, participantById]);

  // Group current tasks by their planner-named role into ranked candidates.
  const candidates = useMemo<RoleCandidate[]>(() => {
    interface Acc {
      name: string;
      titles: string[];
      memberDescription?: string;
      models: Set<string>;
      modelPreferences: Map<string, { provider_id?: ProviderId; model: string; required: false }>;
      agentIds: Set<string>;
      enabledSkills: Set<string>;
      disabledBuiltinSkills: Set<string>;
    }
    const byRole = new Map<string, Acc>();
    for (const step of detail.steps.filter((item) => item.superseded_in_revision == null)) {
      const role = step.role?.trim();
      if (!role) continue;
      const key = role.toLowerCase();
      let acc = byRole.get(key);
      if (!acc) {
        acc = {
          name: role,
          titles: [],
          models: new Set<string>(),
          modelPreferences: new Map(),
          agentIds: new Set<string>(),
          enabledSkills: new Set<string>(),
          disabledBuiltinSkills: new Set<string>(),
        };
        byRole.set(key, acc);
      }
      const title = step.title?.trim();
      if (title && acc.titles.length < 3 && !acc.titles.includes(title)) acc.titles.push(title);
      const participant = participantByStep.get(step.id);
      if (participant) {
        const model = participant.model?.trim();
        if (model) {
          acc.models.add(model);
          const providerId = participant.provider_id ?? undefined;
          acc.modelPreferences.set(`${providerId ?? ''}::${model}`, {
            provider_id: providerId,
            model,
            required: false,
          });
        }
        if (participant.source_agent_id?.trim()) acc.agentIds.add(participant.source_agent_id.trim());
        collect(acc.enabledSkills, participant.enabled_skills);
        collect(acc.disabledBuiltinSkills, participant.disabled_builtin_skills);
        const desc = participant.description?.trim();
        if (desc && !acc.memberDescription) acc.memberDescription = desc;
      }
    }

    return Array.from(byRole.values())
      .map((acc) => {
        // Prefer a participant's own description; otherwise synthesize a short line
        // from the role + a couple of the task titles it covered.
        const description = acc.memberDescription
          ? acc.memberDescription
          : acc.titles.length > 0
            ? t('agentExecution.profile.synthDesc', {
                role: acc.name,
                tasks: acc.titles.join(t('agentExecution.profile.taskSep')),
              })
            : t('agentExecution.profile.synthDescBare', { role: acc.name });
        const lowerName = acc.name.toLowerCase();
        return {
          name: acc.name,
          description,
          models: Array.from(acc.models),
          modelPreferences: Array.from(acc.modelPreferences.values()),
          agentIds: Array.from(acc.agentIds),
          instructions: acc.memberDescription || description,
          enabledSkills: Array.from(acc.enabledSkills),
          disabledBuiltinSkills: Array.from(acc.disabledBuiltinSkills),
          exists: existingNames?.has(lowerName) ?? false,
        } satisfies RoleCandidate;
      })
      .sort((a, b) => a.name.localeCompare(b.name));
  }, [detail.steps, participantByStep, existingNames, t]);

  // There are roles to precipitate at all — gate the one-time preset fetch on
  // this so role-less executions never make the request.
  const hasRoles = candidates.length > 0;

  // Load the existing presets once (when there is at least one role) so we
  // can mark already-precipitated roles instead of offering a duplicate.
  useEffect(() => {
    if (!hasRoles || existingNames !== null) return;
    let cancelled = false;
    void (async () => {
      try {
        const list = await ipcBridge.presets.list.invoke();
        if (cancelled) return;
        const names = new Set<string>();
        for (const a of list ?? []) collect(names, [a.name.toLowerCase()]);
        setExistingNames(names);
      } catch {
        // Non-fatal: if the list can't load we just show every role as new.
        if (!cancelled) setExistingNames(new Set<string>());
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [hasRoles, existingNames]);

  const handleSave = useCallback(
    async (candidate: RoleCandidate) => {
      setSavingNames((prev) => {
        const next = new Set(prev);
        next.add(candidate.name);
        return next;
      });
      try {
        const payload: CreatePresetRequest = {
          name: candidate.name,
          description: candidate.description,
          routing_description: candidate.description,
          instructions: candidate.instructions,
          targets: ['conversation'],
          agent_preferences: candidate.agentIds.map((agent_id) => ({
            agent_id,
            required: false,
          })),
          model_preferences: candidate.modelPreferences,
          included_skills: candidate.enabledSkills.map((skill_name) => ({
            skill_name,
            required: false,
          })),
          excluded_auto_skills: candidate.disabledBuiltinSkills,
          fallback_allowed: true,
          knowledge_policy: {
            enabled: false,
            mode: 'inherit',
            writeback: false,
            grounded: false,
          },
        };
        await ipcBridge.presets.create.invoke(payload);
        setSavedNames((prev) => {
          const next = new Set(prev);
          next.add(candidate.name);
          return next;
        });
        message.success(t('agentExecution.profile.saveOk', { name: candidate.name }));
      } catch (e) {
        message.error(t('agentExecution.profile.saveError', { error: String(e) }));
      } finally {
        setSavingNames((prev) => {
          const next = new Set(prev);
          next.delete(candidate.name);
          return next;
        });
      }
    },
    [message, t],
  );

  // Render nothing for role-less executions or once every role is already a preset.
  // (Locally-saved roles still render — disabled with a ✓ — so the user gets the
  // satisfying confirmation; only roles that pre-existed before this panel
  // opened are hidden entirely.)
  if (!hasRoles) return null;
  const visible = candidates.filter((c) => !c.exists);
  if (visible.length === 0) return null;

  return (
    <div className='shrink-0 border-b border-b-base bg-1'>
      {ctx}
      {/* Banner header — click to collapse/expand without ever covering the canvas. */}
      <div
        role='button'
        tabIndex={0}
        aria-expanded={!collapsed}
        onClick={() => setCollapsed((v) => !v)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            setCollapsed((v) => !v);
          }
        }}
        className='flex cursor-pointer select-none items-center gap-8px px-16px py-10px transition-colors hover:bg-fill-1'
      >
        <span className='flex size-22px shrink-0 items-center justify-center rd-6px bg-fill-2 text-primary-6'>
          <Experiment theme='outline' size='14' strokeWidth={3} />
        </span>
        <div className='min-w-0 flex-1'>
          <div className='truncate text-13px font-600 text-t-primary'>{t('agentExecution.profile.title')}</div>
          <div className='truncate text-11px text-t-tertiary'>{t('agentExecution.profile.subtitle', { count: visible.length })}</div>
        </div>
        <span className='flex size-22px shrink-0 items-center justify-center text-t-tertiary'>
          {collapsed ? <Down theme='outline' size='16' strokeWidth={3} /> : <Up theme='outline' size='16' strokeWidth={3} />}
        </span>
      </div>

      {!collapsed && (
        // Single horizontal lane — the candidates NEVER wrap onto extra rows
        // (which would grow this `shrink-0` banner and squeeze the `flex-1`
        // canvas below). Overflowing cards scroll right instead; the global 6px
        // scrollbar (base.css) surfaces on hover and the next card peeks past the
        // right edge, both hinting there is more to scroll to.
        <div ref={laneRef} className='flex flex-nowrap gap-10px overflow-x-auto overflow-y-hidden px-16px pb-12px'>
          {visible.map((candidate) => {
            const saved = savedNames.has(candidate.name);
            const saving = savingNames.has(candidate.name);
            return (
              <div
                key={candidate.name}
                // Fixed width + `shrink-0` so cards keep their size and overflow
                // horizontally instead of squeezing to fit the lane.
                className='flex w-248px shrink-0 flex-col gap-8px rd-10px border border-b-base bg-2 p-12px'
              >
                <div className='flex items-start justify-between gap-8px'>
                  <div className='min-w-0'>
                    <div className='truncate text-13px font-600 text-t-primary'>{candidate.name}</div>
                    <div className='mt-2px line-clamp-2 text-11px leading-16px text-t-tertiary'>{candidate.description}</div>
                  </div>
                </div>

                {/* Models + skill count meta */}
                <div className='flex flex-col gap-5px'>
                  {candidate.models.length > 0 && (
                    <div className='flex flex-wrap items-center gap-4px'>
                      <span className='shrink-0 text-10px text-t-tertiary'>{t('agentExecution.profile.modelsLabel')}</span>
                      {candidate.models.map((m) => (
                        <span key={m} className='rd-full bg-fill-2 px-7px py-1px text-10px font-500 text-t-secondary'>
                          {m}
                        </span>
                      ))}
                    </div>
                  )}
                  {candidate.enabledSkills.length > 0 && (
                    <div className='text-10px text-t-tertiary'>
                      {t('agentExecution.profile.skillsLabel', {
                        count: candidate.enabledSkills.length,
                      })}
                    </div>
                  )}
                </div>

                {/* Save-as-preset control */}
                <div
                  role='button'
                  tabIndex={saved || saving ? -1 : 0}
                  aria-disabled={saved || saving}
                  aria-label={t('agentExecution.profile.saveAsPreset')}
                  onClick={saved || saving ? undefined : () => void handleSave(candidate)}
                  onKeyDown={(e) => {
                    if ((e.key === 'Enter' || e.key === ' ') && !saved && !saving) {
                      e.preventDefault();
                      void handleSave(candidate);
                    }
                  }}
                  className='flex h-28px items-center justify-center gap-5px rd-8px text-12px font-500 transition-all'
                  style={
                    saved
                      ? {
                          background: 'var(--bg-3)',
                          color: 'var(--success)',
                          cursor: 'default',
                        }
                      : {
                          background: 'rgb(var(--primary-6))',
                          color: '#fff',
                          cursor: saving ? 'default' : 'pointer',
                          opacity: saving ? 0.6 : 1,
                        }
                  }
                >
                  {saved ? (
                    <>
                      <Check theme='outline' size='13' strokeWidth={4} />
                      <span>{t('agentExecution.profile.saved')}</span>
                    </>
                  ) : (
                    <>
                      <AddUser theme='outline' size='13' strokeWidth={3} />
                      <span>{t('agentExecution.profile.saveAsPreset')}</span>
                    </>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
};

export default ParticipantProfilePanel;
