/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ArrowUp, CloseSmall, Robot, Square } from '@icon-park/react';
import { Button, Input, Popover } from '@arco-design/web-react';
import React from 'react';

import {
  NomiCreativeModelSelect,
  type CreativeModelFilter,
  type CreativeModelSelectionRef,
} from '../models';
import type { CreativeStudioAgentSendInput } from './types';
import type {
  CreativeStudioAgentContextItem,
  CreativeStudioAgentSkillOption,
} from './types';
import styles from './CreativeStudioAgentPanel.module.css';

export const CREATIVE_STUDIO_AGENT_MODEL_FILTER = {
  capability: 'task',
  task: 'chat',
} as const satisfies CreativeModelFilter;

interface CreativeStudioAgentComposerProps {
  draft: string;
  model: CreativeModelSelectionRef | null;
  modelLocked: boolean;
  isRunning: boolean;
  disabled: boolean;
  contextItems: readonly CreativeStudioAgentContextItem[];
  skillOptions: readonly CreativeStudioAgentSkillOption[];
  selectedSkillIds: readonly string[];
  onDraftChange(draft: string): void;
  onModelChange(model: CreativeModelSelectionRef): void;
  onRemoveContextItem(itemId: string): void;
  onToggleSkill(skillId: string): void;
  onSend(input: CreativeStudioAgentSendInput): void;
  onStop(): void;
  onOpenModelSettings?(): void;
}

const CreativeStudioAgentComposer: React.FC<CreativeStudioAgentComposerProps> = ({
  draft,
  model,
  modelLocked,
  isRunning,
  disabled,
  contextItems,
  skillOptions,
  selectedSkillIds,
  onDraftChange,
  onModelChange,
  onRemoveContextItem,
  onToggleSkill,
  onSend,
  onStop,
  onOpenModelSettings,
}) => {
  const canSend =
    !disabled &&
    !isRunning &&
    Boolean(model) &&
    Boolean(draft.trim()) &&
    selectedSkillIds.length > 0;

  const submit = () => {
    const prompt = draft.trim();
    if (!prompt || !model || disabled || isRunning) return;
    onSend({
      prompt,
      model,
      contextNodeIds: contextItems.map((item) => item.id),
      skillIds: [...selectedSkillIds],
    });
  };

  const modelPicker = (
    <div className={styles.modelPopover}>
      <NomiCreativeModelSelect
        filter={CREATIVE_STUDIO_AGENT_MODEL_FILTER}
        value={model}
        onChange={onModelChange}
        disabled={disabled || isRunning || modelLocked}
        label='对话模型'
        copy={{
          placeholder: '选择 Agent 对话模型',
          noCompatibleModel: '没有支持 chat 任务的已启用模型。',
        }}
        onOpenModelSettings={onOpenModelSettings}
      />
    </div>
  );

  return (
    <div
      className={styles.composerShell}
      data-agent-composer
      data-agent-model-locked={modelLocked || undefined}
    >
      <div className={styles.composerCard}>
        {contextItems.length > 0 ? (
          <div className={styles.planningField} data-agent-context-items>
            <span className={styles.planningLabel}>画布上下文</span>
            <div className={styles.contextChips}>
              {contextItems.map((item) => (
                <span
                  key={item.id}
                  className={styles.contextChip}
                  data-selected={item.selected || undefined}
                  title={`${item.type} · ${item.id}`}
                >
                  <span>{item.label}</span>
                  <button
                    type='button'
                    aria-label={`移除上下文：${item.label}`}
                    disabled={disabled || isRunning}
                    onClick={() => onRemoveContextItem(item.id)}
                  >
                    <CloseSmall theme='outline' size='11' />
                  </button>
                </span>
              ))}
            </div>
          </div>
        ) : null}
        <div className={styles.planningField} data-agent-skill-options>
          <span className={styles.planningLabel}>创作技能</span>
          <div className={styles.skillChips}>
            {skillOptions.map((skill) => {
              const active = selectedSkillIds.includes(skill.id);
              return (
                <button
                  key={skill.id}
                  type='button'
                  className={styles.skillChip}
                  data-active={active || undefined}
                  aria-pressed={active}
                  disabled={disabled || isRunning}
                  title={skill.description}
                  onClick={() => onToggleSkill(skill.id)}
                >
                  {skill.label}
                </button>
              );
            })}
          </div>
        </div>
        <Input.TextArea
          className={styles.composerInput}
          value={draft}
          disabled={disabled}
          autoSize={{ minRows: 3, maxRows: 7 }}
          placeholder='描述创作目标，或继续讨论当前方案'
          onChange={onDraftChange}
          onKeyDown={(event) => {
            if (
              event.key !== 'Enter' ||
              event.shiftKey ||
              event.ctrlKey ||
              event.metaKey ||
              event.altKey ||
              event.nativeEvent.isComposing
            ) {
              return;
            }
            event.preventDefault();
            submit();
          }}
        />
        <div className={styles.composerFooter}>
          <Popover
            trigger='click'
            position='top'
            content={modelPicker}
            disabled={disabled || modelLocked}
          >
            <Button
              className={styles.modelTrigger}
              type='text'
              size='small'
              icon={<Robot theme='outline' size='14' />}
              disabled={disabled || modelLocked}
              title={
                modelLocked
                  ? `会话模型已锁定：${model?.model ?? ''}`
                  : model
                    ? `${model.providerId} / ${model.model}`
                    : '选择对话模型'
              }
            >
              <span className={styles.modelTriggerLabel}>{model?.model ?? '选择模型'}</span>
            </Button>
          </Popover>
          <Button
            className={styles.sendButton}
            type='primary'
            shape='circle'
            size='large'
            disabled={isRunning ? disabled : !canSend}
            aria-label={isRunning ? '停止 Agent' : '发送给 Agent'}
            icon={
              isRunning ? (
                <Square theme='filled' size='15' />
              ) : (
                <ArrowUp theme='outline' size='17' />
              )
            }
            onClick={isRunning ? onStop : submit}
          />
        </div>
      </div>
    </div>
  );
};
export default CreativeStudioAgentComposer;
