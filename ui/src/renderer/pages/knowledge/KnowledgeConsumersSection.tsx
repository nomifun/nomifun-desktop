/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import classNames from 'classnames';
import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Message as ArcoMessage, Modal, Tooltip } from '@arco-design/web-react';
import { Down, FolderOpen, Message, Right, Robot, Terminal, Unlink, User } from '@icon-park/react';
import { ipcBridge } from '@/common';
import type { IKnowledgeBinding, IKnowledgeConsumer, KnowledgeBindingKind } from '@/common/adapter/ipcBridge';
import { useKnowledgeConsumers } from './useKnowledge';
import type { KnowledgeBaseId } from '@/common/types/ids';

interface KnowledgeConsumersSectionProps {
  baseId: KnowledgeBaseId;
}

function kindIcon(kind: string): React.ReactNode {
  const size = '14';
  switch (kind) {
    case 'workpath':
      return <FolderOpen theme='outline' size={size} />;
    case 'companion':
      return <Robot theme='outline' size={size} />;
    case 'conversation':
      return <Message theme='outline' size={size} />;
    case 'terminal':
      return <Terminal theme='outline' size={size} />;
    default:
      return <User theme='outline' size={size} />;
  }
}

const supportedBindingKinds = new Set<KnowledgeBindingKind>(['conversation', 'terminal', 'companion', 'workpath']);

function isSupportedBindingKind(kind: string): kind is KnowledgeBindingKind {
  return supportedBindingKinds.has(kind as KnowledgeBindingKind);
}

function consumerKey(c: IKnowledgeConsumer, fallback = 0): string {
  return `${c.target_kind}-${c.target_id ?? fallback}`;
}

export function removeBaseFromBinding(binding: IKnowledgeBinding, baseId: KnowledgeBaseId): IKnowledgeBinding {
  const kbIds = binding.kb_ids.filter((id) => id !== baseId);
  return {
    ...binding,
    enabled: kbIds.length > 0 ? binding.enabled : false,
    kb_ids: kbIds,
  };
}

/**
 * Collapsible "who is using this base?" section. Collapsed: a one-line count.
 * Expanded: one row per binding (workspace / companion / conversation /
 * terminal), greying disabled ones.
 */
const KnowledgeConsumersSection: React.FC<KnowledgeConsumersSectionProps> = ({ baseId }) => {
  const { t } = useTranslation();
  const { consumers, loading, refresh } = useKnowledgeConsumers(baseId);
  const [open, setOpen] = useState(false);
  const [removingKey, setRemovingKey] = useState<string | null>(null);

  if (loading && consumers.length === 0) return null;
  if (consumers.length === 0) return null;

  const label = (c: IKnowledgeConsumer): string => {
    const id = c.target_id ?? '—';
    switch (c.target_kind) {
      case 'workpath':
        return id;
      case 'conversation':
        return t('knowledge.consumers.conversationLabel', { id });
      case 'terminal':
        return t('knowledge.consumers.terminalLabel', { id });
      case 'companion':
        return t('knowledge.consumers.companionLabel', { id });
      default:
        return `${c.target_kind}: ${id}`;
    }
  };

  const handleUnmountConsumer = (consumer: IKnowledgeConsumer, rowKey: string) => {
    if (!isSupportedBindingKind(consumer.target_kind) || !consumer.target_id) return;

    const targetKind = consumer.target_kind;
    const targetId = consumer.target_id;
    const targetLabel = label(consumer);

    Modal.confirm({
      title: t('knowledge.consumers.removeConfirmTitle', { defaultValue: '取消挂载此知识库？' }),
      content: (
        <div className='text-13px leading-20px text-[var(--color-text-2)]'>
          <div>
            {t('knowledge.consumers.removeConfirmContent', {
              defaultValue: '将从「{{target}}」取消挂载此知识库。知识库内容不会被删除。',
              target: targetLabel,
            })}
          </div>
          {targetKind === 'workpath' && (
            <div className='mt-6px text-[var(--color-text-3)]'>
              {t('knowledge.consumers.workpathRemoveHint', {
                defaultValue: '该工作区下共享此挂载配置的会话都会受影响。',
              })}
            </div>
          )}
        </div>
      ),
      okText: t('knowledge.consumers.removeMount', { defaultValue: '取消挂载' }),
      cancelText: t('knowledge.actions.cancel', { defaultValue: '取消' }),
      okButtonProps: { status: 'danger' },
      onOk: async () => {
        setRemovingKey(rowKey);
        try {
          const binding = await ipcBridge.knowledge.getBinding.invoke({ kind: targetKind, target_id: targetId });
          const next = removeBaseFromBinding(binding, baseId);
          if (next.kb_ids.length !== binding.kb_ids.length || next.enabled !== binding.enabled) {
            await ipcBridge.knowledge.setBinding.invoke({ kind: targetKind, target_id: targetId, ...next });
          }
          await refresh();
          ArcoMessage.success(t('knowledge.consumers.removeOk', { defaultValue: '已取消挂载' }));
        } catch (e) {
          const message = e instanceof Error ? e.message : String(e);
          ArcoMessage.error(
            t('knowledge.consumers.removeFailed', {
              defaultValue: '取消挂载失败：{{message}}',
              message,
            })
          );
        } finally {
          setRemovingKey(null);
        }
      },
    });
  };

  return (
    <div className='knowledge-consumers-disclosure box-border w-full rd-10px bg-[var(--color-fill-2)] p-4px shadow-[inset_0_0_0_1px_rgba(var(--primary-6),0.08)]'>
      <button
        type='button'
        className={classNames(
          'flex w-full cursor-pointer items-center gap-7px rd-8px border-none bg-transparent px-12px py-9px text-left text-13px font-500',
          'text-[var(--color-text-2)] transition-colors hover:bg-[var(--color-fill-3)] hover:text-[var(--color-text-1)]',
          'focus-visible:outline-none focus-visible:bg-[var(--color-fill-3)] focus-visible:text-[var(--color-text-1)]'
        )}
        onClick={() => setOpen((v) => !v)}
      >
        <span className={classNames('shrink-0 text-[var(--color-text-3)]', open && 'text-[rgb(var(--primary-6))]')}>
          {open ? <Down theme='outline' size='14' /> : <Right theme='outline' size='14' />}
        </span>
        <span className='truncate'>{t('knowledge.consumers.summary', { count: consumers.length })}</span>
      </button>
      {open && (
        <div className='mt-2px flex flex-col gap-3px'>
          {consumers.map((c, i) => {
            const rowKey = consumerKey(c, i);
            const canUnmount = isSupportedBindingKind(c.target_kind) && !!c.target_id;
            return (
              <div
                key={rowKey}
                className={classNames(
                  'knowledge-consumers-row group flex items-center gap-8px rd-8px bg-[var(--color-bg-2)] px-12px py-8px text-13px',
                  'shadow-[inset_0_0_0_1px_rgba(0,0,0,0.03)]',
                  c.enabled ? 'text-[var(--color-text-2)]' : 'text-[var(--color-text-4)]'
                )}
              >
                <span className='shrink-0 text-[var(--color-text-3)]'>{kindIcon(c.target_kind)}</span>
                <span className='min-w-0 flex-1 truncate' title={label(c)}>
                  {label(c)}
                </span>
                {!c.enabled && (
                  <span className='shrink-0 text-11px text-[var(--color-text-4)]'>
                    {t('knowledge.consumers.disabled')}
                  </span>
                )}
                {canUnmount && (
                  <Tooltip content={t('knowledge.consumers.removeMount', { defaultValue: '取消挂载' })}>
                    <Button
                      type='text'
                      size='mini'
                      shape='circle'
                      loading={removingKey === rowKey}
                      className='knowledge-consumers-remove !h-24px !w-24px !min-w-24px shrink-0 !p-0 !text-[var(--color-text-3)] hover:!bg-[rgba(var(--danger-6),0.1)] hover:!text-[rgb(var(--danger-6))] focus-visible:!bg-[rgba(var(--danger-6),0.1)] focus-visible:!text-[rgb(var(--danger-6))]'
                      icon={<Unlink theme='outline' size='13' />}
                      aria-label={t('knowledge.consumers.removeMount', { defaultValue: '取消挂载' })}
                      onClick={(event) => {
                        event.stopPropagation();
                        handleUnmountConsumer(c, rowKey);
                      }}
                    />
                  </Tooltip>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
};

export default KnowledgeConsumersSection;
