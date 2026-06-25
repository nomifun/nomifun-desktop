/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Message, Popover, Select, Switch, Tooltip } from '@arco-design/web-react';
import { Robot } from '@icon-park/react';
import classNames from 'classnames';
import { ipcBridge } from '@/common';
import type { AutoWorkRunState, AutoWorkTargetKind, IAutoWorkState } from '@/common/adapter/ipcBridge';
import { CAPABILITY_COLORS } from '@/renderer/components/capability/CapabilityIcon';
import { useRequirementTags } from '@renderer/pages/requirements/useRequirements';

export interface AutoWorkTarget {
  kind: AutoWorkTargetKind;
  /** conversation/terminal session id — backend INTEGER (numeric-id spec §1). */
  id: number;
}

/** Draft (pre-creation) AutoWork config: held by the parent and applied once
 * the conversation exists (e.g. Guid page applies it right after create). */
export type AutoWorkDraftValue = { enabled: boolean; tag?: string };
export type AutoWorkDraft = {
  value: AutoWorkDraftValue;
  onChange: (next: AutoWorkDraftValue) => void;
};

interface AutoWorkControlProps {
  /** What this control drives: a chat conversation or a terminal session.
   * Omit when using `draft` mode. */
  target?: AutoWorkTarget;
  /** Controlled draft state — when set, the control never reads or writes the
   * backend AutoWork state; `target` is ignored. */
  draft?: AutoWorkDraft;
  /** When set, the control is disabled and this reason shows as a tooltip
   * (e.g. a terminal that is not an agent CLI). */
  disabledReason?: string;
  /** Optional safety note shown inside the popover (e.g. a non-full-auto terminal). */
  safetyHint?: string;
  /** When the config takes effect, shown at the panel bottom (draft mode). */
  applyNote?: string;
}

/** Tri-state status dot colour — shared capability palette (CAPABILITY_COLORS). */
const DOT_COLOR: Record<AutoWorkRunState, string> = {
  off: CAPABILITY_COLORS.off,
  idle: CAPABILITY_COLORS.idle,
  active: CAPABILITY_COLORS.active,
};

/**
 * Per-session AutoWork control. Rendered in a conversation header (kind
 * `conversation`) or a terminal session header (kind `terminal`). A compact
 * button opens a popover with the tag picker + toggle + tri-state status.
 * The Guid page renders the same control in `draft` mode to pre-configure a
 * conversation before it exists.
 */
const AutoWorkControl: React.FC<AutoWorkControlProps> = ({ target, draft, disabledReason, safetyHint, applyNote }) => {
  const { t } = useTranslation();
  const { tags } = useRequirementTags();
  const [state, setState] = useState<IAutoWorkState | null>(null);
  const [persistedTag, setPersistedTag] = useState<string | undefined>();
  const kind = target?.kind;
  const id = target?.id;
  const tag = draft ? draft.value.tag : persistedTag;
  const setTag = (next: string | undefined) => {
    if (draft) draft.onChange({ ...draft.value, tag: next });
    else setPersistedTag(next);
  };

  useEffect(() => {
    if (draft || !kind || !id) return;
    void ipcBridge.requirements.getAutoWork
      .invoke({ kind, target_id: id })
      .then((s) => {
        setState(s);
        setPersistedTag(s.tag);
      })
      .catch(() => {});
    // `draft` identity changes every render; only its presence matters here.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [kind, id, !!draft]);

  useEffect(() => {
    if (draft || !kind || !id) return;
    const unsub = ipcBridge.requirements.onAutoWork.on((s) => {
      if (s.kind === kind && s.target_id === id) {
        setState(s);
        setPersistedTag(s.tag);
      }
    });
    return () => unsub();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [kind, id, !!draft]);

  const enabled = draft ? draft.value.enabled : (state?.enabled ?? false);
  const running = state?.running ?? false;
  const runState: AutoWorkRunState = state?.run_state ?? 'off';
  // Draft mode shows "preset" wording with a neutral/primary dot — the live
  // run-state palette (idle orange / active green) is reserved for a running
  // session.
  const dotColor = draft ? (enabled ? CAPABILITY_COLORS.primary : CAPABILITY_COLORS.off) : DOT_COLOR[runState];
  const statusText = draft
    ? enabled
      ? t('guid.advanced.draftOn')
      : t('guid.advanced.draftOff')
    : t(`requirements.autowork.state.${runState}`);

  const toggle = async (next: boolean) => {
    if (next && !tag) {
      Message.warning(t('requirements.autowork.tagRequired'));
      return;
    }
    if (draft) {
      // Draft mode: just report upward — the parent applies after creation.
      draft.onChange({ enabled: next, tag });
      return;
    }
    if (!kind || !id) return;
    try {
      const s = await ipcBridge.requirements.setAutoWork.invoke({ kind, target_id: id, enabled: next, tag });
      setState(s);
      Message.success(next ? t('requirements.autowork.enabledOk') : t('requirements.autowork.disabledOk'));
    } catch (e) {
      Message.error(String(e));
    }
  };

  const panel = (
    <div className='flex flex-col gap-10px w-240px p-4px'>
      <div className='text-t-primary text-13px font-600'>{t('requirements.autowork.label')}</div>
      <div className='text-t-tertiary text-12px leading-16px'>{t('requirements.autowork.hint')}</div>
      {safetyHint ? <div className='text-12px leading-16px text-[rgb(var(--warning-6))]'>{safetyHint}</div> : null}
      <div className='flex flex-col gap-4px'>
        <span className='text-t-secondary text-12px'>{t('requirements.autowork.tagLabel')}</span>
        <Tooltip disabled={!enabled} content={t('requirements.autowork.disableToChangeTag')}>
          <Select
            size='small'
            placeholder={t('requirements.autowork.selectTag')}
            value={tag}
            onChange={setTag}
            disabled={enabled}
            options={tags.map((tg) => ({ label: `${tg.tag} (${tg.done}/${tg.total})`, value: tg.tag }))}
            allowClear
          />
        </Tooltip>
      </div>
      <div className='flex items-center justify-between'>
        <span className='inline-flex items-center gap-6px text-t-secondary text-12px'>
          <span className='inline-block w-6px h-6px rounded-full' style={{ backgroundColor: dotColor }} />
          {statusText}
        </span>
        <Switch checked={enabled} onChange={toggle} />
      </div>
      {running && state?.completed_count != null ? (
        <div className='text-t-tertiary text-11px'>
          {t('requirements.autowork.completedCount', { count: state.completed_count })}
        </div>
      ) : null}
      {applyNote ? <div className='text-t-quaternary text-11px leading-15px'>{applyNote}</div> : null}
    </div>
  );

  // Icon + label + status dot share one flex baseline — this is the alignment fix
  // (the old control wrapped the icon in a Badge inside the Button's icon slot,
  // which knocked it out of vertical centering).
  const button = (
    <Button
      size='mini'
      shape='round'
      type={enabled ? 'primary' : 'secondary'}
      disabled={!!disabledReason}
      className='shrink-0'
    >
      <span className='inline-flex items-center gap-6px leading-none'>
        <Robot
          theme='outline'
          size='14'
          fill='currentColor'
          className={classNames('block', runState === 'active' && 'autowork-spin')}
          style={{ lineHeight: 0 }}
        />
        <span className='text-12px'>{t('requirements.autowork.label')}</span>
        <span className='inline-block w-6px h-6px rounded-full' style={{ backgroundColor: dotColor }} />
      </span>
    </Button>
  );

  if (disabledReason) {
    // A disabled button does not fire pointer events, so wrap it for the tooltip.
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

export default AutoWorkControl;
