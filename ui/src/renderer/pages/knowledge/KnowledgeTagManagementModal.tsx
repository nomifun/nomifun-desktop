/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * KnowledgeTagManagementModal — Single-column tag vocabulary CRUD for knowledge
 * bases. Supports inline rename, color picker (preset palette), sort-order
 * adjustment (move up/down), and delete with Popconfirm warning.
 *
 * Mirrors structure from settings/AssistantSettings/TagManagementModal.
 * Theme variables only; `<div onClick>` for clickables (no <button>).
 */
import type { IKnowledgeTag } from '@/common/adapter/ipcBridge';
import { Input, Modal, Popconfirm } from '@arco-design/web-react';
import { Check, Close, Delete, Down, Plus, Up } from '@icon-park/react';
import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';

// ─── Color palette (theme-safe presets) ─────────────────────────────────────

const PRESET_COLORS = [
  '#3491FA', // blue
  '#722ED1', // purple
  '#F77234', // orange
  '#00B42A', // green
  '#E83F8C', // pink
  '#0FC6C2', // teal
  '#F5A623', // amber
  '#86909C', // grey
];

// ─── Props ──────────────────────────────────────────────────────────────────

export type KnowledgeTagManagementModalProps = {
  visible: boolean;
  onClose: () => void;
  tags: IKnowledgeTag[];
  createTag: (label: string, color?: string) => Promise<unknown>;
  updateTag: (key: string, patch: { label?: string; color?: string; sortOrder?: number }) => Promise<void>;
  deleteTag: (key: string) => Promise<void>;
};

// ─── Helpers ────────────────────────────────────────────────────────────────

const errorText = (error: unknown): string => {
  if (error instanceof Error) return error.message;
  if (typeof error === 'string') return error;
  return '';
};

// ─── Color dot ──────────────────────────────────────────────────────────────

const ColorDot: React.FC<{ color?: string; size?: number }> = ({ color, size = 14 }) => (
  <span
    className='inline-block rounded-full flex-shrink-0'
    style={{
      width: size,
      height: size,
      backgroundColor: color || 'var(--color-fill-3)',
    }}
  />
);

// ─── Color picker row ───────────────────────────────────────────────────────

const ColorPicker: React.FC<{
  value?: string;
  onChange: (color: string) => void;
}> = ({ value, onChange }) => (
  <div className='flex items-center gap-6px flex-wrap'>
    {PRESET_COLORS.map((c) => (
      <div
        key={c}
        role='button'
        tabIndex={0}
        onClick={() => onChange(c)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') onChange(c);
        }}
        className='w-20px h-20px rounded-full cursor-pointer transition-all flex items-center justify-center'
        style={{
          backgroundColor: c,
          outline: value === c ? '2px solid rgb(var(--primary-6))' : 'none',
          outlineOffset: 2,
        }}
      >
        {value === c && <Check theme='outline' size={10} strokeWidth={4} fill='#fff' />}
      </div>
    ))}
  </div>
);

// ─── Tag row ────────────────────────────────────────────────────────────────

const TagRow: React.FC<{
  tag: IKnowledgeTag;
  busy: boolean;
  isFirst: boolean;
  isLast: boolean;
  onRename: (key: string, label: string) => void;
  onChangeColor: (key: string, color: string) => void;
  onMoveUp: (key: string) => void;
  onMoveDown: (key: string) => void;
  onDelete: (tag: IKnowledgeTag) => void;
}> = ({ tag, busy, isFirst, isLast, onRename, onChangeColor, onMoveUp, onMoveDown, onDelete }) => {
  const { t } = useTranslation();
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState('');
  const [colorOpen, setColorOpen] = useState(false);

  const commit = () => {
    const next = draft.trim();
    if (next && next !== tag.label) {
      onRename(tag.key, next);
    }
    setEditing(false);
  };

  return (
    <div
      className='group flex flex-col gap-6px rounded-10px px-10px py-8px bg-[var(--color-bg-2)] border border-solid border-[var(--color-border-2)] hover:border-[var(--color-border-3)] transition-colors'
      data-testid={`kb-tag-row-${tag.key}`}
    >
      <div className='flex items-center gap-8px'>
        {/* Color dot (click to toggle color picker) */}
        <div
          role='button'
          tabIndex={0}
          onClick={() => setColorOpen(!colorOpen)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') setColorOpen(!colorOpen);
          }}
          className='cursor-pointer'
          title={t('knowledge.tags.changeColor', { defaultValue: 'Change color' })}
        >
          <ColorDot color={tag.color} />
        </div>

        {editing ? (
          <>
            <Input
              size='small'
              autoFocus
              value={draft}
              onChange={setDraft}
              onPressEnter={commit}
              disabled={busy}
              className='flex-1 !rounded-6px'
            />
            <div
              role='button'
              tabIndex={0}
              onClick={commit}
              onKeyDown={(e) => {
                if (e.key === 'Enter') commit();
              }}
              className='flex-shrink-0 flex items-center justify-center w-22px h-22px rounded-6px cursor-pointer text-[rgb(var(--primary-6))] hover:bg-[var(--color-primary-light-1)] transition-colors'
            >
              <Check theme='outline' size={14} strokeWidth={3} />
            </div>
            <div
              role='button'
              tabIndex={0}
              onClick={() => setEditing(false)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') setEditing(false);
              }}
              className='flex-shrink-0 flex items-center justify-center w-22px h-22px rounded-6px cursor-pointer text-[var(--color-text-3)] hover:bg-[var(--color-fill-2)] transition-colors'
            >
              <Close theme='outline' size={14} strokeWidth={3} />
            </div>
          </>
        ) : (
          <>
            <span
              role='button'
              tabIndex={0}
              onClick={() => {
                setDraft(tag.label);
                setEditing(true);
              }}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  setDraft(tag.label);
                  setEditing(true);
                }
              }}
              className='flex-1 min-w-0 truncate text-13px text-[var(--color-text-1)] cursor-text'
              title={t('knowledge.tags.renameHint', { defaultValue: 'Click to rename' })}
            >
              {tag.label}
            </span>

            {/* Move up */}
            <div
              role='button'
              tabIndex={0}
              onClick={() => !isFirst && onMoveUp(tag.key)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && !isFirst) onMoveUp(tag.key);
              }}
              className={[
                'flex-shrink-0 flex items-center justify-center w-22px h-22px rounded-6px transition-all',
                isFirst
                  ? 'opacity-30 cursor-not-allowed'
                  : 'cursor-pointer text-[var(--color-text-3)] opacity-0 group-hover:opacity-100 hover:bg-[var(--color-fill-2)]',
              ].join(' ')}
            >
              <Up theme='outline' size={14} strokeWidth={3} />
            </div>

            {/* Move down */}
            <div
              role='button'
              tabIndex={0}
              onClick={() => !isLast && onMoveDown(tag.key)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && !isLast) onMoveDown(tag.key);
              }}
              className={[
                'flex-shrink-0 flex items-center justify-center w-22px h-22px rounded-6px transition-all',
                isLast
                  ? 'opacity-30 cursor-not-allowed'
                  : 'cursor-pointer text-[var(--color-text-3)] opacity-0 group-hover:opacity-100 hover:bg-[var(--color-fill-2)]',
              ].join(' ')}
            >
              <Down theme='outline' size={14} strokeWidth={3} />
            </div>

            {/* Delete */}
            <Popconfirm
              title={t('knowledge.tags.deleteConfirm', {
                defaultValue: 'Delete "{{label}}"? It will be removed from all knowledge bases.',
                label: tag.label,
              })}
              okText={t('common.delete', { defaultValue: 'Delete' })}
              cancelText={t('common.cancel', { defaultValue: 'Cancel' })}
              okButtonProps={{ status: 'danger' }}
              onOk={() => onDelete(tag)}
            >
              <div
                role='button'
                tabIndex={0}
                data-testid={`kb-tag-delete-${tag.key}`}
                className='flex-shrink-0 flex items-center justify-center w-22px h-22px rounded-6px cursor-pointer text-[var(--color-text-3)] opacity-0 group-hover:opacity-100 hover:text-[rgb(var(--danger-6))] hover:bg-[rgba(var(--danger-6),0.08)] transition-all'
              >
                <Delete theme='outline' size={14} strokeWidth={3} />
              </div>
            </Popconfirm>
          </>
        )}
      </div>

      {/* Color picker row (toggled) */}
      {colorOpen && (
        <div className='pl-22px'>
          <ColorPicker
            value={tag.color}
            onChange={(c) => {
              onChangeColor(tag.key, c);
              setColorOpen(false);
            }}
          />
        </div>
      )}
    </div>
  );
};

// ─── Main modal ─────────────────────────────────────────────────────────────

const KnowledgeTagManagementModal: React.FC<KnowledgeTagManagementModalProps> = ({
  visible,
  onClose,
  tags,
  createTag,
  updateTag,
  deleteTag,
}) => {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [newLabel, setNewLabel] = useState('');
  const [newColor, setNewColor] = useState<string | undefined>(undefined);

  const handleCreate = async () => {
    const label = newLabel.trim();
    if (!label) return;
    setBusy(true);
    try {
      await createTag(label, newColor);
      setNewLabel('');
      setNewColor(undefined);
    } catch (error) {
      console.error('Failed to create knowledge tag:', error);
      Modal.error({
        title: t('knowledge.tags.createFailed', { defaultValue: 'Failed to create tag' }),
        content: errorText(error),
      });
    } finally {
      setBusy(false);
    }
  };

  const handleRename = async (key: string, label: string) => {
    setBusy(true);
    try {
      await updateTag(key, { label });
    } catch (error) {
      console.error('Failed to rename knowledge tag:', error);
      Modal.error({
        title: t('knowledge.tags.renameFailed', { defaultValue: 'Failed to rename tag' }),
        content: errorText(error),
      });
    } finally {
      setBusy(false);
    }
  };

  const handleChangeColor = async (key: string, color: string) => {
    setBusy(true);
    try {
      await updateTag(key, { color });
    } catch (error) {
      console.error('Failed to update tag color:', error);
    } finally {
      setBusy(false);
    }
  };

  const handleMoveUp = async (key: string) => {
    const idx = tags.findIndex((t) => t.key === key);
    if (idx <= 0) return;
    // Swap sortOrder with the previous tag
    const prev = tags[idx - 1];
    const curr = tags[idx];
    setBusy(true);
    try {
      await updateTag(curr.key, { sortOrder: prev.sortOrder });
      await updateTag(prev.key, { sortOrder: curr.sortOrder });
    } catch (error) {
      console.error('Failed to reorder tags:', error);
    } finally {
      setBusy(false);
    }
  };

  const handleMoveDown = async (key: string) => {
    const idx = tags.findIndex((t) => t.key === key);
    if (idx < 0 || idx >= tags.length - 1) return;
    // Swap sortOrder with the next tag
    const next = tags[idx + 1];
    const curr = tags[idx];
    setBusy(true);
    try {
      await updateTag(curr.key, { sortOrder: next.sortOrder });
      await updateTag(next.key, { sortOrder: curr.sortOrder });
    } catch (error) {
      console.error('Failed to reorder tags:', error);
    } finally {
      setBusy(false);
    }
  };

  const handleDelete = async (tag: IKnowledgeTag) => {
    setBusy(true);
    try {
      await deleteTag(tag.key);
    } catch (error) {
      console.error('Failed to delete knowledge tag:', error);
      Modal.error({
        title: t('knowledge.tags.deleteFailed', { defaultValue: 'Failed to delete tag' }),
        content: errorText(error),
      });
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      visible={visible}
      onCancel={onClose}
      footer={null}
      title={t('knowledge.tags.modalTitle', { defaultValue: 'Manage Tags' })}
      style={{ width: 480, maxWidth: '92vw', borderRadius: 16 }}
      maskClosable={!busy}
      data-testid='kb-tag-management-modal'
    >
      <p className='mt-0 mb-16px text-12px leading-18px text-[var(--color-text-3)]'>
        {t('knowledge.tags.modalDesc', {
          defaultValue:
            'Organize knowledge bases with tags. Tags can be renamed, recolored, reordered, or deleted.',
        })}
      </p>

      {/* Tag list */}
      <div className='flex flex-col gap-6px mb-16px' data-testid='kb-tag-list'>
        {tags.length === 0 ? (
          <div className='rounded-10px border border-dashed border-[var(--color-border-2)] px-10px py-12px text-center text-12px text-[var(--color-text-3)]'>
            {t('knowledge.tags.empty', { defaultValue: 'No tags yet. Create one below.' })}
          </div>
        ) : (
          tags.map((tag, idx) => (
            <TagRow
              key={tag.key}
              tag={tag}
              busy={busy}
              isFirst={idx === 0}
              isLast={idx === tags.length - 1}
              onRename={handleRename}
              onChangeColor={handleChangeColor}
              onMoveUp={handleMoveUp}
              onMoveDown={handleMoveDown}
              onDelete={handleDelete}
            />
          ))
        )}
      </div>

      {/* Create new tag */}
      <div className='flex flex-col gap-8px'>
        <div className='flex items-center gap-8px'>
          {/* Color preview dot for new tag */}
          <ColorDot color={newColor} />
          <Input
            size='small'
            value={newLabel}
            onChange={setNewLabel}
            onPressEnter={() => void handleCreate()}
            disabled={busy}
            data-testid='kb-tag-add-input'
            placeholder={t('knowledge.tags.addPlaceholder', { defaultValue: 'New tag...' })}
            className='flex-1 !rounded-8px'
          />
          <div
            role='button'
            tabIndex={0}
            data-testid='kb-tag-add-btn'
            onClick={() => void handleCreate()}
            onKeyDown={(e) => {
              if (e.key === 'Enter') void handleCreate();
            }}
            className={[
              'flex-shrink-0 inline-flex items-center gap-4px rounded-8px px-10px h-30px text-12px font-medium cursor-pointer',
              'border border-solid transition-all duration-150',
              newLabel.trim() && !busy
                ? 'bg-[var(--color-primary-light-1)] text-[rgb(var(--primary-6))] border-[var(--color-primary-light-3)] hover:bg-[var(--color-primary-light-2)]'
                : 'bg-[var(--color-fill-2)] text-[var(--color-text-3)] border-[var(--color-border-2)] cursor-not-allowed',
            ].join(' ')}
          >
            <Plus theme='outline' size={13} strokeWidth={3} />
            {t('common.add', { defaultValue: 'Add' })}
          </div>
        </div>
        {/* Color picker for new tag */}
        <div className='pl-22px'>
          <ColorPicker value={newColor} onChange={setNewColor} />
        </div>
      </div>
    </Modal>
  );
};

export default KnowledgeTagManagementModal;
