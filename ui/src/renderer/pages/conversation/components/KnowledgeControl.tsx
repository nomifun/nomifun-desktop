/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * KnowledgeControl — Knowledge-base selection and write-back policy popover.
 *
 * Trigger button + popover panel are deliberately aligned with the sibling
 * conversation-header capability controls: a compact
 * `Button size='mini' shape='round'` with an icon + label + tri-state status
 * dot, and a popover using the shared capability-control design language
 * (icon-chip header + status pill + rounded `bg-fill-1` card sections), instead
 * of the earlier bespoke square icon-button + full-bleed-divider panel.
 *
 * Supported ownership modes:
 * - Guid draft → one new AgentSession's initial Knowledge resources/policy
 * - Conversation → live AgentSession Knowledge subset, applied between turns
 * - terminal → workpath and companion → per-profile target resolution
 * - Binding read/write via `POST /api/knowledge/binding/{kind}/{target_id}`
 * - AgentSession/legacy binding-change + base-created/updated/deleted WS refresh
 * - `disabledReason` tooltip, `applyNote`, `footer` passthrough
 * - First-time discoverability hint tooltip
 */

import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { TFunction } from 'i18next';
import { Button, Input, Message, Popover, Switch, Tooltip } from '@arco-design/web-react';
import { BookOne } from '@icon-park/react';
import { useNavigate } from 'react-router-dom';
import { ipcBridge } from '@/common';
import type { CompanionId, ConversationId, KnowledgeBaseId, TerminalId } from '@/common/types/ids';
import type {
  IKnowledgeBase,
  IKnowledgeBinding,
  IKnowledgeTag,
  KnowledgeWritebackEagerness,
} from '@/common/adapter/ipcBridge';
import { useTerminalSessions } from '@/renderer/pages/terminal/useTerminalSessions';
import {
  workpathKeyForTerminal,
} from '@/renderer/pages/conversation/SessionList/utils/sessionWorkpath';
import { useKnowledgeTags } from '@/renderer/pages/knowledge/useKnowledgeTags';
import { CAPABILITY_COLORS } from '@/renderer/components/capability/CapabilityIcon';
import {
  filterKnowledgeBasesByQuery,
  shouldShowKnowledgeBaseSearch,
} from './KnowledgeControl.utils';
import { capabilityHeaderButtonClass, capabilityHeaderButtonStyle } from './CapabilityHeaderButton';
import {
  workpathDisplayForKnowledgeTarget,
  type ResolvedKnowledgeBindingTarget,
} from '../Workspace/KnowledgePanel/knowledgeBindingTarget';

export type KnowledgeTarget =
  | { kind: 'conversation'; id: ConversationId }
  | { kind: 'terminal'; id: TerminalId }
  | { kind: 'companion'; id: CompanionId }
  | { kind: 'workpath'; id: string };

type KnowledgeControlProps = {
  target?: KnowledgeTarget;
  draft?: KnowledgeDraft;
  writebackAvailable?: boolean;
  disabledReason?: string;
  applyNote?: string;
  footer?: React.ReactNode;
};

export type KnowledgeDraft = {
  value: IKnowledgeBinding;
  onChange: (next: IKnowledgeBinding) => void;
};

export const defaultKnowledgeBinding = (): IKnowledgeBinding => ({
  enabled: false,
  writeback: false,
  writeback_eagerness: 'manual',
  channel_write_enabled: false,
  kb_ids: [],
});

// ─── Shared capability-panel visual tokens ──────────────────────────────────

/** Rounded card section shared by capability panels. */
const sectionClass =
  'flex flex-col gap-8px rounded-12px border border-solid border-[var(--color-border-2)] bg-[var(--color-bg-1)] px-12px py-10px';
const fieldStackClass = 'min-w-0 flex flex-col gap-4px';
const fieldLabelClass = 'text-[var(--color-text-1)] text-11px font-600 leading-15px';
const subtleInsetClass =
  'rounded-8px border border-solid border-[var(--color-border-2)] bg-[var(--color-bg-1)] px-10px py-8px';
/** Tinted surface; `--primary-6` is an RGB triplet. */
const tintBg = (color: string, amount = 12): string =>
  `color-mix(in srgb, rgb(${color}) ${amount}%, var(--color-bg-1))`;
const lineClamp2Style: React.CSSProperties = {
  display: '-webkit-box',
  WebkitBoxOrient: 'vertical',
  WebkitLineClamp: 2,
  overflow: 'hidden',
};

// ─── Kind badge config (mirrors KnowledgeCard getKindConfig) ─────────────────

type KindBadgeStyle = { bgClass: string; textClass: string; borderClass: string };

function getKindBadge(kind: IKnowledgeBase['kind']): KindBadgeStyle {
  switch (kind) {
    case 'local':
      return {
        bgClass: 'bg-[rgba(var(--primary-6),0.1)]',
        textClass: 'text-primary-5',
        borderClass: 'border-[rgba(var(--primary-6),0.3)]',
      };
    case 'web':
      return {
        bgClass: 'bg-[rgba(var(--success-6),0.1)]',
        textClass: 'text-success-5',
        borderClass: 'border-[rgba(var(--success-6),0.3)]',
      };
    case 'blank':
    default:
      return {
        bgClass: 'bg-fill-2',
        textClass: 'text-[var(--color-text-2)]',
        borderClass: 'border-[var(--color-border-2)]',
      };
  }
}

function kindLabel(kind: IKnowledgeBase['kind'], t: TFunction): string {
  switch (kind) {
    case 'local':
      return t('knowledge.card.kindLocal', { defaultValue: '本地文件夹' });
    case 'web':
      return t('knowledge.card.kindWeb', { defaultValue: '网页' });
    case 'blank':
    default:
      return t('knowledge.card.kindBlank', { defaultValue: '空白' });
  }
}

// ─── Main Component ──────────────────────────────────────────────────────────

const KnowledgeControl: React.FC<KnowledgeControlProps> = ({ target, draft, writebackAvailable = true, disabledReason, applyNote, footer }) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { sessions: terminalSessions } = useTerminalSessions();
  const { tags: allTags } = useKnowledgeTags();

  // Build tag key → IKnowledgeTag map
  const tagMap = useMemo(() => {
    const m: Record<string, IKnowledgeTag> = {};
    for (const tag of allTags) m[tag.key] = tag;
    return m;
  }, [allTags]);

  const tagLabelsByKey = useMemo(() => {
    const m: Record<string, string> = {};
    for (const tag of allTags) m[tag.key] = tag.label;
    return m;
  }, [allTags]);

  const kindLabelsByKind = useMemo(
    () => ({
      blank: kindLabel('blank', t),
      local: kindLabel('local', t),
      web: kindLabel('web', t),
    }),
    [t]
  );

  // ─── Target resolution ───────────────────────────────────────────────────
  const resolved = useMemo((): ResolvedKnowledgeBindingTarget | null => {
    if (!target) return null;
    if (target.kind === 'companion') return { kind: 'companion', target_id: target.id };
    if (target.kind === 'conversation') return { kind: 'conversation', target_id: target.id };
    if (target.kind === 'terminal') {
      const session = terminalSessions.find((s) => s.terminal_id === target.id);
      if (!session) return null;
      return { kind: 'workpath', target_id: workpathKeyForTerminal(session) };
    }
    return { kind: 'workpath', target_id: target.id };
  }, [target?.kind, target?.id, terminalSessions]);

  const kind = resolved?.kind;
  const id = resolved?.target_id;
  const targetUnresolved = !draft && Boolean(target) && target?.kind !== 'companion' && !resolved;

  // Only workpath targets display a shared workspace scope.
  const workpathDisplay = workpathDisplayForKnowledgeTarget(resolved);

  // ─── State ────────────────────────────────────────────────────────────────
  const [bases, setBases] = useState<IKnowledgeBase[]>([]);
  const [basesLoaded, setBasesLoaded] = useState(false);
  const [persistedBinding, setPersistedBinding] = useState<IKnowledgeBinding>(defaultKnowledgeBinding);
  const savingRef = useRef(false);
  const [saving, setSaving] = useState(false);
  const binding = draft?.value ?? persistedBinding;
  const [searchQuery, setSearchQuery] = useState('');
  const isDraftMode = Boolean(draft);

  const reloadBinding = useCallback(async () => {
    if (isDraftMode || !kind || !id) return;
    try {
      const next = kind === 'conversation'
        ? await ipcBridge.agentPlatform.sessions.getKnowledge.invoke({ agent_session_id: id })
        : await ipcBridge.knowledge.getBinding.invoke({ kind, target_id: id });
      setPersistedBinding({ channel_write_enabled: false, ...next });
    } catch {
      /* ignore — keep current binding */
    }
  }, [id, isDraftMode, kind]);

  // ─── Discoverability hint ─────────────────────────────────────────────────
  const [hintVisible, setHintVisible] = useState(false);
  const dismissHint = () => {
    setHintVisible(false);
    try {
      localStorage.setItem('knowledge.control.hintSeen', '1');
    } catch {
      /* private mode */
    }
  };
  useEffect(() => {
    let seen = false;
    try {
      seen = localStorage.getItem('knowledge.control.hintSeen') === '1';
    } catch {
      /* ignore */
    }
    if (!basesLoaded || seen || disabledReason) return;
    if (bases.length === 0) {
      setHintVisible(true);
      const timer = setTimeout(dismissHint, 8000);
      return () => clearTimeout(timer);
    }
    setHintVisible(false);
    return undefined;
  }, [basesLoaded, bases.length, disabledReason]);

  // ─── Load bases + binding ─────────────────────────────────────────────────
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const [list, b] = await Promise.all([
          ipcBridge.knowledge.listBases.invoke(),
          isDraftMode || !kind || !id
            ? Promise.resolve(null)
            : kind === 'conversation'
              ? ipcBridge.agentPlatform.sessions.getKnowledge.invoke({ agent_session_id: id })
              : ipcBridge.knowledge.getBinding.invoke({ kind, target_id: id }),
        ]);
        if (cancelled) return;
        setBases(list);
        if (b) setPersistedBinding({ channel_write_enabled: false, ...b });
      } catch {
        /* ignore — keep defaults */
      } finally {
        if (!cancelled) setBasesLoaded(true);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [id, isDraftMode, kind]);

  useEffect(() => {
    if (!draft || !basesLoaded) return;
    const available = new Set(
      bases
        .filter((base) => base.root_exists)
        .map((base) => base.knowledge_base_id)
    );
    const retained = draft.value.kb_ids.filter((id) => available.has(id)).slice(0, 32);
    const selectedHasReadOnly = retained.some((id) =>
      bases.find((base) => base.knowledge_base_id === id)?.tree_access !== 'editable'
    );
    const policyNeedsReset = (!writebackAvailable || selectedHasReadOnly) && (
      draft.value.writeback || draft.value.writeback_eagerness !== 'manual'
    );
    if (
      retained.length === draft.value.kb_ids.length
      && retained.every((id, index) => id === draft.value.kb_ids[index])
      && !policyNeedsReset
    ) return;
    const enabled = retained.length > 0;
    const writeback = enabled && writebackAvailable && draft.value.writeback;
    draft.onChange({
      ...draft.value,
      enabled,
      kb_ids: retained,
      writeback,
      writeback_eagerness: writeback ? draft.value.writeback_eagerness : 'manual',
    });
  }, [bases, basesLoaded, draft, writebackAvailable]);

  // Keep base list fresh
  useEffect(() => {
    const reload = () => {
      void ipcBridge.knowledge.listBases
        .invoke()
        .then(setBases)
        .catch(() => {});
    };
    const unsubs = [
      ipcBridge.knowledge.onBaseCreated.on(reload),
      ipcBridge.knowledge.onBaseUpdated.on(reload),
      ipcBridge.knowledge.onBaseDeleted.on(reload),
    ];
    return () => unsubs.forEach((u) => u());
  }, []);

  useEffect(() => {
    if (isDraftMode || !kind || !id) return;
    if (kind === 'conversation') {
      const unsub = ipcBridge.agentPlatform.sessions.onKnowledgeChanged.on((event) => {
        if (event.agent_session_id !== id) return;
        setPersistedBinding({ ...event.binding, channel_write_enabled: false });
      });
      return () => unsub();
    }
    const unsub = ipcBridge.knowledge.onBindingChanged.on((event) => {
      if (event.target_kind !== kind || event.target_id !== id) return;
      void reloadBinding();
    });
    return () => unsub();
  }, [id, isDraftMode, kind, reloadBinding]);

  // ─── Persist ──────────────────────────────────────────────────────────────
  const persist = async (next: IKnowledgeBinding) => {
    if (draft) {
      draft.onChange(next);
      return;
    }
    if (!kind || !id) return;
    if (savingRef.current) return;
    savingRef.current = true;
    setSaving(true);
    setPersistedBinding(next);
    try {
      const saved = kind === 'conversation'
        ? await ipcBridge.agentPlatform.sessions.updateKnowledge.invoke({
            agent_session_id: id,
            binding: {
              enabled: next.enabled,
              writeback: next.writeback,
              writeback_eagerness: next.writeback_eagerness,
              kb_ids: next.kb_ids,
            },
          })
        : await ipcBridge.knowledge.setBinding.invoke({ kind, target_id: id, ...next });
      setPersistedBinding({ channel_write_enabled: false, ...saved });
      if (next.enabled !== binding.enabled) {
        // Terminal workpath bindings re-sync into the live PTY workspace
        // immediately through the backend binding hook. Companion profile
        // rows are product-owned defaults; this control never mutates a live
        // AgentSession.
        const enabledKey = target?.kind === 'conversation'
          ? 'knowledge.control.enabledOkConversation'
          : target?.kind === 'terminal'
            ? 'knowledge.control.enabledOkTerminal'
            : 'knowledge.control.enabledOk';
        Message.success(next.enabled ? t(enabledKey) : t('knowledge.control.disabledOk'));
      }
    } catch (e) {
      setPersistedBinding(binding);
      Message.error(String(e));
    } finally {
      savingRef.current = false;
      setSaving(false);
    }
  };

  // ─── Handlers ─────────────────────────────────────────────────────────────
  const selectedBasesHaveReadOnly = binding.kb_ids.some((id) =>
    bases.find((base) => base.knowledge_base_id === id)?.tree_access !== 'editable'
  );

  const handleToggleBase = (baseId: KnowledgeBaseId) => {
    const isSelected = binding.kb_ids.includes(baseId);
    if (!isSelected && binding.kb_ids.length >= 32) return;
    const addsReadOnly = !isSelected
      && bases.find((base) => base.knowledge_base_id === baseId)?.tree_access !== 'editable';
    const nextIds = isSelected ? binding.kb_ids.filter((x) => x !== baseId) : [...binding.kb_ids, baseId];
    // Auto-enable when first base selected; auto-disable when last removed
    const nextEnabled = nextIds.length > 0 ? true : false;
    void persist({
      ...binding,
      kb_ids: nextIds,
      enabled: nextEnabled,
      // An unmounted binding must be genuinely inert. Do not carry a stale
      // write-back policy after the final base is removed.
      ...(nextEnabled && !addsReadOnly
        ? {}
        : { writeback: false, writeback_eagerness: 'manual' as const }),
    });
  };

  const handleWritebackToggle = (v: boolean) => {
    if (!binding.enabled || !writebackAvailable || (v && selectedBasesHaveReadOnly)) return;
    void persist({ ...binding, writeback: v });
  };

  const handleEnabledToggle = (enabled: boolean) => {
    if (enabled && binding.kb_ids.length === 0) return;
    void persist({
      ...binding,
      enabled,
      ...(enabled
        ? {}
        : { writeback: false, writeback_eagerness: 'manual' as const }),
    });
  };

  const handleWritebackEagerness = (eagerness: KnowledgeWritebackEagerness) => {
    void persist({ ...binding, writeback_eagerness: eagerness });
  };

  const mountedCount = binding.enabled ? binding.kb_ids.length : 0;

  // Filter bases by search query
  const filteredBases = useMemo(() => {
    return filterKnowledgeBasesByQuery(bases, searchQuery, tagLabelsByKey, kindLabelsByKind);
  }, [bases, searchQuery, kindLabelsByKind, tagLabelsByKey]);
  const missingSelectedIds = useMemo(() => {
    const available = new Set(bases.map((base) => base.knowledge_base_id));
    return binding.kb_ids.filter((id) => !available.has(id));
  }, [bases, binding.kb_ids]);

  // ─── Derived status (shared by trigger button + panel header pill) ─────────
  // Knowledge has no live run-state — the dot is a binary enabled/off marker
  // (primary when mounted, gray otherwise).
  const dotColor = binding.enabled ? CAPABILITY_COLORS.primary : CAPABILITY_COLORS.off;
  const statusText = !binding.enabled
    ? t('knowledge.control.off')
    : binding.writeback
      ? t(
          binding.writeback_eagerness === 'auto'
            ? 'knowledge.control.mountedAuto'
            : 'knowledge.control.mountedManual',
          { count: mountedCount }
        )
      : t('knowledge.control.mountedReadOnly', { count: mountedCount });

  // Compact segmented control (writeback disposition) — tinted track with a
  // primary active pill, sitting on the section's bg-fill-1 surface.
  const renderSegment = (
    current: string,
    options: Array<{ value: string; label: string }>,
    onPick: (v: string) => void
  ) => (
    <div className='inline-flex w-fit gap-2px rounded-8px bg-fill-2 p-2px'>
      {options.map((o) => {
        const activeSeg = current === o.value;
        return (
          <div
            key={o.value}
            className={[
              'cursor-pointer rounded-6px px-10px py-4px text-11px leading-none',
              activeSeg
                ? 'border border-solid border-[rgba(var(--primary-6),0.28)] bg-[rgba(var(--primary-6),0.12)] text-primary-6 font-600'
                : 'border border-solid border-transparent text-[var(--color-text-1)] hover:bg-[var(--color-fill-3)]',
              (targetUnresolved || saving) && 'cursor-not-allowed opacity-60',
            ]
              .filter(Boolean)
              .join(' ')}
            onClick={() => !targetUnresolved && !saving && onPick(o.value)}
          >
            {o.label}
          </div>
        );
      })}
    </div>
  );

  const writebackEagernessHint =
    binding.writeback_eagerness === 'manual'
      ? t('knowledge.control.eagernessManualHint')
      : t('knowledge.control.eagernessAutoHint');

  // One mounted base row.
  const renderBaseRow = (base: IKnowledgeBase) => {
    const isSelected = binding.kb_ids.includes(base.knowledge_base_id);
    const rootMissing = !base.root_exists;
    const writeUnavailable = writebackAvailable && base.tree_access !== 'editable';
    const cannotSelect = rootMissing && !isSelected;
    const badge = getKindBadge(base.kind);
    const baseTags = base.tags.map((tk) => tagMap[tk]).filter((x): x is IKnowledgeTag => !!x);
    const firstTag = baseTags[0];
    return (
      <div
        key={base.knowledge_base_id}
        className={[
          'flex items-center gap-9px rounded-8px border border-solid bg-[var(--color-bg-1)] px-8px py-7px cursor-pointer transition-colors',
          isSelected ? 'border-[rgba(var(--primary-6),0.38)]' : 'border-[var(--color-border-2)] hover:bg-fill-2',
          (targetUnresolved || saving) && 'opacity-50 cursor-not-allowed',
          cannotSelect && 'cursor-not-allowed opacity-65 hover:bg-[var(--color-bg-1)]',
        ]
          .filter(Boolean)
          .join(' ')}
        style={isSelected ? { background: tintBg('var(--primary-6)', 7) } : undefined}
        onClick={() => !targetUnresolved && !saving && (!cannotSelect || isSelected) && handleToggleBase(base.knowledge_base_id)}
      >
        {/* Checkbox */}
        <span
          className={[
            'grid h-17px w-17px flex-none place-items-center rounded-5px border-1.5px border-solid text-10px leading-none',
            isSelected
              ? 'border-[rgba(var(--primary-6),0.48)] bg-[rgba(var(--primary-6),0.12)] text-primary-6'
              : 'border-[var(--color-text-3)] bg-[var(--color-bg-1)] text-transparent',
          ].join(' ')}
        >
          ✓
        </span>

        {/* Content */}
        <span className='min-w-0 flex-1'>
          <span className='flex items-center gap-6px'>
            <span className='truncate text-13px font-600 text-[var(--color-text-1)]'>{base.name}</span>
            <span
              className={[
                'inline-flex shrink-0 items-center rounded-5px border border-solid px-5px py-1px text-9px font-600',
                badge.bgClass,
                badge.textClass,
                badge.borderClass,
              ].join(' ')}
            >
              {kindLabel(base.kind, t)}
            </span>
            <span className='knowledge-control-base-meta shrink-0 text-11px font-500 text-[var(--color-text-2)]'>
              {rootMissing
                ? t('knowledge.mount.rootMissing', { defaultValue: '目录不可用' })
                : base.kind === 'web'
                  ? t('knowledge.mount.realtime', { defaultValue: '实时' })
                  : t('knowledge.mount.fileCount', { defaultValue: '{{count}} 篇', count: base.file_count })}
            </span>
          </span>
          {rootMissing && (
            <span className='knowledge-control-root-missing mt-2px block text-11px leading-15px text-danger-6'>
              {t('knowledge.mount.rootMissingHint', {
                defaultValue: '源目录不存在或暂时不可访问，恢复目录后再挂载。',
              })}
            </span>
          )}
          {!rootMissing && writeUnavailable && (
            <span className='knowledge-control-write-unavailable mt-2px block text-11px leading-15px text-warning-6'>
              {t('knowledge.mount.writeAccessRequired')}
            </span>
          )}
          {firstTag && (
            <span className='mt-2px flex items-center gap-3px text-11px text-[var(--color-text-2)]'>
              <span
                className='inline-block h-6px w-6px rounded-full'
                style={{ background: firstTag.color || 'var(--color-text-3)' }}
              />
              {firstTag.label}
            </span>
          )}
        </span>
      </div>
    );
  };

  // ─── Panel content ────────────────────────────────────────────────────────
  const showSearch = shouldShowKnowledgeBaseSearch(bases.length);
  const panel = (
    <div className='box-border flex w-340px max-h-500px flex-col gap-10px overflow-hidden p-12px'>
      {/* Header uses the shared capability-panel structure: icon chip + title on the
          left, status pill on the right, hint below. The earlier "container
          misalignment" was Arco's own popover-shell padding (now zeroed via
          .knowledge-control-popover in arco-override.css
          already uses), NOT this row's layout. */}
      <div className='flex flex-col gap-6px'>
        <div className='flex items-center justify-between gap-10px'>
          <span className='inline-flex min-w-0 items-center gap-8px'>
            <span
              className='inline-flex h-24px w-24px shrink-0 items-center justify-center rounded-6px'
              style={{ background: tintBg('var(--primary-6)', 10), color: CAPABILITY_COLORS.primary }}
            >
              <BookOne theme='outline' size='15' fill='currentColor' />
            </span>
            <span className='min-w-0 truncate text-t-primary text-13px font-600'>{t('knowledge.control.label')}</span>
          </span>
          <span className='inline-flex shrink-0 items-center gap-5px rounded-full border border-solid border-[var(--color-border-2)] bg-[var(--color-bg-1)] px-7px py-3px text-11px font-500 text-[var(--color-text-1)]'>
            <span className='inline-block h-6px w-6px rounded-full' style={{ backgroundColor: dotColor }} />
            {statusText}
          </span>
        </div>
        <div className='text-[var(--color-text-2)] text-11px leading-16px' style={lineClamp2Style}>
          {t(isDraftMode ? 'knowledge.control.guidHint' : 'knowledge.control.hint')}
        </div>
        <div
          className='self-start text-11px font-600 text-primary-6 cursor-pointer hover:underline'
          onClick={() => navigate('/knowledge')}
        >
          {t('knowledge.mount.manage', { defaultValue: '管理知识库 ›' })}
        </div>
      </div>

      {basesLoaded && bases.length === 0 ? (
        // ─── Empty state ───────────────────────────────────────────────────
        <div className='flex flex-col items-center gap-12px px-12px py-22px text-center'>
          <span
            className='inline-flex h-44px w-44px items-center justify-center rounded-12px'
            style={{ background: tintBg('var(--primary-6)', 12), color: CAPABILITY_COLORS.primary }}
          >
            <BookOne theme='outline' size='22' fill='currentColor' />
          </span>
          <p className='m-0 whitespace-pre-line text-12px text-[var(--color-text-2)] leading-17px'>
            {t('knowledge.mount.emptyHint', {
              defaultValue: '你还没有任何知识库。\n知识库能给这个会话补上专属的领域知识。',
            })}
          </p>
          <Button type='primary' size='small' shape='round' onClick={() => navigate('/knowledge')}>
            {t('knowledge.mount.createFirst', { defaultValue: '＋ 新建第一个知识库' })}
          </Button>
        </div>
      ) : (
        <>
          {/* Scope inset + (conditional) search — fixed above the scroll body */}
          {workpathDisplay && (
            <div className={`${subtleInsetClass} text-[var(--color-text-2)] text-11px leading-16px`}>
              {t('knowledge.mount.scope', {
                defaultValue: '作用范围：工作区 {{path}}。同一工作区下的所有会话共享这套挂载设置。',
                path: workpathDisplay,
              })}
            </div>
          )}
          {showSearch && (
            <Input
              size='small'
              allowClear
              className='knowledge-control-search'
              value={searchQuery}
              onChange={(v: string) => setSearchQuery(v)}
              aria-label={t('knowledge.mount.searchPlaceholder', { defaultValue: '搜索 / 筛选知识库…' })}
              placeholder={t('knowledge.mount.searchPlaceholder', { defaultValue: '搜索 / 筛选知识库…' })}
            />
          )}

          {/* Scrollable body */}
          <div className='flex min-h-0 flex-1 flex-col gap-8px overflow-y-auto'>
            <div className={sectionClass}>
              <div className='flex items-center justify-between gap-10px'>
                <span className='min-w-0 flex flex-col gap-2px'>
                  <span className='text-[var(--color-text-1)] text-13px font-600'>
                    {t('knowledge.control.mountEnabled')}
                  </span>
                  <span className='text-[var(--color-text-2)] text-11px leading-15px'>
                    {t('knowledge.control.mountEnabledDesc')}
                  </span>
                </span>
                <Switch
                  size='small'
                  checked={binding.enabled}
                  disabled={targetUnresolved || saving || binding.kb_ids.length === 0}
                  onChange={handleEnabledToggle}
                />
              </div>
            </div>

            {/* Mounted-bases section */}
            <div className={sectionClass}>
              <span className={fieldLabelClass}>{t('knowledge.control.basesLabel', { defaultValue: '挂载的知识库' })}</span>
              <div className='flex flex-col gap-3px'>
                {filteredBases.length === 0 && missingSelectedIds.length === 0 ? (
                  <span className='py-4px text-[var(--color-text-2)] text-11px'>
                    {t('knowledge.filterEmpty', { defaultValue: '没有匹配的知识库' })}
                  </span>
                ) : (
                  filteredBases.map(renderBaseRow)
                )}
                {missingSelectedIds.map((baseId) => (
                  <button
                    key={baseId}
                    type='button'
                    disabled={targetUnresolved || saving}
                    className='flex w-full cursor-pointer items-center gap-8px rounded-8px border border-solid border-[rgba(var(--danger-6),0.35)] bg-[rgba(var(--danger-6),0.05)] px-9px py-8px text-left disabled:cursor-not-allowed disabled:opacity-60'
                    onClick={() => handleToggleBase(baseId)}
                  >
                    <span className='min-w-0 flex-1'>
                      <span className='block truncate text-12px font-600 text-danger-6'>{baseId}</span>
                      <span className='mt-2px block text-11px leading-15px text-[var(--color-text-2)]'>
                        {t('knowledge.control.missingSelection')}
                      </span>
                    </span>
                    <span className='shrink-0 text-11px font-600 text-danger-6'>
                      {t('knowledge.consumers.removeMount')}
                    </span>
                  </button>
                ))}
              </div>
            </div>

            {/* Writeback section */}
            <div className={sectionClass}>
              <div className='flex items-center justify-between gap-10px'>
                <span className='min-w-0 flex flex-col gap-2px'>
                  <span className='text-[var(--color-text-1)] text-13px font-600'>
                    {t('knowledge.control.writeback', { defaultValue: '回血知识库' })}
                  </span>
                  <span className='text-[var(--color-text-2)] text-11px leading-15px'>
                    {t(
                      !writebackAvailable
                        ? 'knowledge.control.writebackUnavailable'
                        : selectedBasesHaveReadOnly
                          ? 'knowledge.control.writebackReadOnlySelection'
                          : 'knowledge.mount.writebackDesc'
                    )}
                  </span>
                </span>
                <Switch
                  size='small'
                  checked={binding.writeback}
                  disabled={targetUnresolved || saving || !binding.enabled || !writebackAvailable || selectedBasesHaveReadOnly || binding.kb_ids.length === 0}
                  onChange={handleWritebackToggle}
                />
              </div>

              {binding.writeback && (
                <div className={fieldStackClass}>
                  <span className={fieldLabelClass}>
                    {t('knowledge.control.writebackEagerness', { defaultValue: '回写意识' })}
                  </span>
                  {renderSegment(
                    binding.writeback_eagerness,
                    [
                      {
                        value: 'manual',
                        label: t('knowledge.control.eagernessManual', { defaultValue: '手动型（推荐）' }),
                      },
                      {
                        value: 'auto',
                        label: t('knowledge.control.eagernessAuto', { defaultValue: '自动型' }),
                      },
                    ],
                    (v) => handleWritebackEagerness(v as KnowledgeWritebackEagerness)
                  )}
                  <span className='text-[var(--color-text-2)] text-11px leading-15px'>{writebackEagernessHint}</span>
                </div>
              )}
            </div>

            {applyNote && <div className='text-[var(--color-text-2)] text-11px leading-15px'>{applyNote}</div>}
          </div>
        </>
      )}

      {footer ? <div className='shrink-0 border-t border-t-solid border-[var(--color-border-1)] pt-8px'>{footer}</div> : null}
    </div>
  );

  // ─── Trigger button aligned with the other capability controls ────────────
  const button = (
    <Button
      size='mini'
      shape='round'
      type='secondary'
      disabled={!!disabledReason}
      className={capabilityHeaderButtonClass(binding.enabled, 'shrink-0')}
      style={capabilityHeaderButtonStyle(dotColor)}
    >
      <span className='inline-flex items-center gap-6px leading-none'>
        {/* Icon tinted by enabled-state (primary when mounted, gray off) — the
            status used to live on a separate dot beside a primary-blue button. */}
        <BookOne theme='outline' size='14' fill={dotColor} className='block' style={{ lineHeight: 0 }} />
        <span className='text-12px'>{t('knowledge.control.label')}</span>
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
    <Popover
      className='knowledge-control-popover'
      trigger='click'
      position='br'
      content={panel}
      onVisibleChange={(v: boolean) => {
        if (v) {
          dismissHint();
          setSearchQuery('');
        }
      }}
    >
      <Tooltip content={t(isDraftMode ? 'knowledge.control.guidDiscoverHint' : 'knowledge.control.discoverHint')} popupVisible={hintVisible} position='bottom'>
        {button}
      </Tooltip>
    </Popover>
  );
};

export default KnowledgeControl;
