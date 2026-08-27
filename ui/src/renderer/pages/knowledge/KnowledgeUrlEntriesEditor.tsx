/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { Input } from '@arco-design/web-react';
import { Close, Plus } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import {
  MAX_KNOWLEDGE_SOURCE_ENTRIES,
  type KnowledgeUrlDraft,
} from './knowledgeUrlEntries';

interface KnowledgeUrlEntriesEditorProps {
  entries: KnowledgeUrlDraft[];
  onChange: (entries: KnowledgeUrlDraft[]) => void;
  maxEntries?: number;
  disabled?: boolean;
  compact?: boolean;
}

const inputClass =
  'knowledge-source-input rounded-12px border-transparent bg-[var(--color-fill-1)] transition-[background-color,border-color,box-shadow] hover:bg-[var(--color-fill-2)] focus-within:shadow-[0_0_0_3px_rgba(var(--primary-6),0.1)]';

const KnowledgeUrlEntriesEditor: React.FC<KnowledgeUrlEntriesEditorProps> = ({
  entries,
  onChange,
  maxEntries = MAX_KNOWLEDGE_SOURCE_ENTRIES,
  disabled = false,
  compact = false,
}) => {
  const { t } = useTranslation();
  const safeEntries = entries.length > 0 ? entries.slice(0, maxEntries) : [{ url: '', title: '' }];

  const updateEntry = (index: number, field: keyof KnowledgeUrlDraft, value: string) => {
    const next = [...safeEntries];
    next[index] = { ...next[index], [field]: value };
    onChange(next);
  };

  const removeEntry = (index: number) => {
    if (safeEntries.length <= 1) return;
    onChange(safeEntries.filter((_, entryIndex) => entryIndex !== index));
  };

  return (
    <div className='knowledge-url-entries-editor'>
      <div className='space-y-8px'>
        {safeEntries.map((entry, index) => (
          <div
            key={index}
            className='flex items-start gap-8px'
          >
            <div
              className={`grid min-w-0 flex-1 grid-cols-1 gap-8px ${compact ? 'sm:grid-cols-[minmax(0,1fr)_116px]' : 'sm:grid-cols-[minmax(0,1fr)_138px]'}`}
            >
              <Input
                className={inputClass}
                placeholder='https://example.com/docs'
                value={entry.url}
                disabled={disabled}
                aria-label={t('knowledge.detail.docs.webUrlAria', {
                  defaultValue: '网址 {{index}}',
                  index: index + 1,
                })}
                onChange={(value) => updateEntry(index, 'url', value)}
              />
              <Input
                className={inputClass}
                placeholder={t('knowledge.studio.webTitleOptional', { defaultValue: '标题（可选）' })}
                value={entry.title}
                disabled={disabled}
                aria-label={t('knowledge.detail.docs.webTitleAria', {
                  defaultValue: '网址 {{index}} 的标题（可选）',
                  index: index + 1,
                })}
                onChange={(value) => updateEntry(index, 'title', value)}
              />
            </div>
            <button
              type='button'
              className='flex size-34px cursor-pointer items-center justify-center rounded-10px border-none bg-[var(--color-fill-1)] text-[var(--color-text-3)] transition-colors hover:bg-[var(--color-danger-light-1)] hover:text-danger-6 focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--danger-6),0.12)] disabled:cursor-not-allowed disabled:opacity-45'
              onClick={() => removeEntry(index)}
              disabled={disabled || safeEntries.length <= 1}
              aria-label={t('knowledge.detail.docs.removeWebUrl', {
                defaultValue: '移除网址 {{index}}',
                index: index + 1,
              })}
            >
              <Close theme='outline' size='14' />
            </button>
          </div>
        ))}
      </div>

      {safeEntries.length < maxEntries && (
        <button
          type='button'
          className='mt-9px inline-flex w-full cursor-pointer items-center justify-center gap-5px rounded-12px border-none bg-[rgba(var(--primary-6),0.07)] p-9px text-12px font-500 text-primary-6 transition-colors hover:bg-[rgba(var(--primary-6),0.12)] focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--primary-6),0.12)] disabled:cursor-not-allowed disabled:opacity-50'
          onClick={() => onChange([...safeEntries, { url: '', title: '' }])}
          disabled={disabled}
        >
          <Plus theme='outline' size='13' />
          {t('knowledge.studio.webAddUrl', { defaultValue: '添加网址' })}
        </button>
      )}
    </div>
  );
};

export default KnowledgeUrlEntriesEditor;
