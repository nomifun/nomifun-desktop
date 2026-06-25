/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import classNames from 'classnames';
import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Down, FolderOpen, Message, Right, Robot, Terminal, User } from '@icon-park/react';
import type { IKnowledgeConsumer } from '@/common/adapter/ipcBridge';
import { useKnowledgeConsumers } from './useKnowledge';

interface KnowledgeConsumersSectionProps {
  baseId: string;
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

/**
 * Collapsible "who is using this base?" section. Collapsed: a one-line count.
 * Expanded: one row per binding (workspace / companion / conversation /
 * terminal), greying disabled ones.
 */
const KnowledgeConsumersSection: React.FC<KnowledgeConsumersSectionProps> = ({ baseId }) => {
  const { t } = useTranslation();
  const { consumers, loading } = useKnowledgeConsumers(baseId);
  const [open, setOpen] = useState(false);

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
          {consumers.map((c, i) => (
            <div
              key={`${c.target_kind}-${c.target_id ?? i}`}
              className={classNames(
                'knowledge-consumers-row flex items-center gap-8px rd-8px bg-[var(--color-bg-2)] px-12px py-8px text-13px',
                'shadow-[inset_0_0_0_1px_rgba(0,0,0,0.03)]',
                c.enabled ? 'text-[var(--color-text-2)]' : 'text-[var(--color-text-4)]'
              )}
            >
              <span className='shrink-0 text-[var(--color-text-3)]'>{kindIcon(c.target_kind)}</span>
              <span className='truncate' title={label(c)}>
                {label(c)}
              </span>
              {!c.enabled && <span className='shrink-0 text-11px text-[var(--color-text-4)]'>{t('knowledge.consumers.disabled')}</span>}
            </div>
          ))}
        </div>
      )}
    </div>
  );
};

export default KnowledgeConsumersSection;
