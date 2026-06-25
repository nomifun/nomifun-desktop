/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * TagPicker — multi-select existing knowledge tags + inline create-on-Enter.
 *
 * Controlled via `value: string[]` (tag keys) / `onChange`.
 * Uses the `useKnowledgeTags` hook for listing & creating tags.
 * Renders as chips with toggle selection; a trailing input allows inline creation.
 * Theme variables only; no hard-coded semantic colors.
 */
import React, { useCallback, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Message, Spin } from '@arco-design/web-react';
import type { IKnowledgeTag } from '@/common/adapter/ipcBridge';

// ─── Props ──────────────────────────────────────────────────────────────────

export interface TagPickerProps {
  value: string[];
  onChange: (keys: string[]) => void;
  tags: IKnowledgeTag[];
  createTag: (label: string) => Promise<IKnowledgeTag>;
}

// ─── Component ──────────────────────────────────────────────────────────────

const TagPicker: React.FC<TagPickerProps> = ({ value, onChange, tags, createTag }) => {
  const { t } = useTranslation();
  const [inputValue, setInputValue] = useState('');
  const [creating, setCreating] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  const toggleTag = useCallback(
    (key: string) => {
      if (value.includes(key)) {
        onChange(value.filter((k) => k !== key));
      } else {
        onChange([...value, key]);
      }
    },
    [value, onChange],
  );

  const handleKeyDown = async (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key !== 'Enter') return;
    e.preventDefault();
    const label = inputValue.trim();
    if (!label) return;
    // If a tag with the same label already exists, just select it
    const existing = tags.find((t) => t.label === label);
    if (existing) {
      if (!value.includes(existing.key)) {
        onChange([...value, existing.key]);
      }
      setInputValue('');
      return;
    }
    // Otherwise create new
    setCreating(true);
    try {
      const newTag = await createTag(label);
      onChange([...value, newTag.key]);
      setInputValue('');
    } catch (err) {
      Message.error(String(err));
    } finally {
      setCreating(false);
    }
  };

  return (
    <div className='flex flex-wrap items-center gap-6px'>
      {tags.map((tag) => {
        const selected = value.includes(tag.key);
        return (
          <div
            key={tag.key}
            onClick={() => toggleTag(tag.key)}
            className={[
              'knowledge-studio-tag-chip cursor-pointer select-none rounded-8px px-10px py-5px text-12px font-500 transition-[background-color,color,box-shadow]',
              selected
                ? 'bg-[rgba(var(--primary-6),0.12)] text-[rgb(var(--primary-6))] shadow-[inset_0_0_0_1px_rgba(var(--primary-6),0.22)]'
                : 'bg-[var(--color-fill-1)] text-[var(--color-text-2)] shadow-[inset_0_0_0_1px_rgba(0,0,0,0.035)] hover:bg-[var(--color-fill-2)] hover:text-[var(--color-text-1)]',
            ].join(' ')}
          >
            {tag.label}
          </div>
        );
      })}
      {/* Inline create input */}
      <div className='relative inline-flex items-center'>
        <input
          ref={inputRef}
          className='knowledge-studio-tag-input w-86px rounded-8px border border-transparent bg-[var(--color-fill-1)] px-9px py-5px text-12px text-[var(--color-text-2)] outline-none font-[inherit] shadow-[inset_0_0_0_1px_rgba(0,0,0,0.035)] transition-[background-color,border-color,box-shadow] placeholder:text-[var(--color-text-4)] hover:bg-[var(--color-fill-2)] focus:border-[rgba(var(--primary-6),0.32)] focus:bg-[var(--color-bg-2)] focus-visible:shadow-[0_0_0_3px_rgba(var(--primary-6),0.12)]'
          placeholder={t('knowledge.studio.tagNewPlaceholder', { defaultValue: '+ 新标签' })}
          value={inputValue}
          onChange={(e) => setInputValue(e.target.value)}
          onKeyDown={(e) => void handleKeyDown(e)}
          disabled={creating}
        />
        {creating && <Spin size={12} className='absolute right-6px' />}
      </div>
    </div>
  );
};

export default TagPicker;
