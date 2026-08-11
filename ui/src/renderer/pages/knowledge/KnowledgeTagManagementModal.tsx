/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Compact tag vocabulary CRUD for knowledge bases. Tags wrap across rows while
 * the vocabulary area scrolls independently, keeping the modal height stable.
 */
import type { IKnowledgeTag } from '@/common/adapter/ipcBridge';
import { Input, Modal, Popconfirm, Popover } from '@arco-design/web-react';
import { Check, Close, CloseSmall, Plus } from '@icon-park/react';
import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';

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

export type KnowledgeTagManagementModalProps = {
  visible: boolean;
  onClose: () => void;
  tags: IKnowledgeTag[];
  createTag: (label: string, color?: string) => Promise<unknown>;
  updateTag: (key: string, patch: { label?: string; color?: string }) => Promise<void>;
  deleteTag: (key: string) => Promise<void>;
};

const errorText = (error: unknown): string => {
  if (error instanceof Error) return error.message;
  if (typeof error === 'string') return error;
  return '';
};

const ColorDot: React.FC<{ color?: string; size?: number }> = ({ color, size = 14 }) => (
  <span
    className='inline-block flex-shrink-0 rounded-full'
    style={{
      width: size,
      height: size,
      backgroundColor: color || 'var(--color-fill-3)',
    }}
  />
);

const ColorPicker: React.FC<{
  value?: string;
  onChange: (color: string) => void;
}> = ({ value, onChange }) => (
  <div className='flex flex-wrap items-center gap-6px'>
    {PRESET_COLORS.map((color) => (
      <div
        key={color}
        role='button'
        tabIndex={0}
        onClick={() => onChange(color)}
        onKeyDown={(event) => {
          if (event.key === 'Enter' || event.key === ' ') {
            event.preventDefault();
            onChange(color);
          }
        }}
        className='flex h-20px w-20px cursor-pointer items-center justify-center rounded-full transition-all'
        style={{
          backgroundColor: color,
          outline: value === color ? '2px solid rgb(var(--primary-6))' : 'none',
          outlineOffset: 2,
        }}
      >
        {value === color && <Check theme='outline' size={10} strokeWidth={4} fill='#fff' />}
      </div>
    ))}
  </div>
);

const TagChip: React.FC<{
  tag: IKnowledgeTag;
  busy: boolean;
  onRename: (key: string, label: string) => void;
  onChangeColor: (key: string, color: string) => void;
  onDelete: (tag: IKnowledgeTag) => void;
}> = ({ tag, busy, onRename, onChangeColor, onDelete }) => {
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

  const beginRename = () => {
    if (busy) return;
    setDraft(tag.label);
    setEditing(true);
  };

  return (
    <div
      className='group box-border inline-flex h-28px max-w-full items-center gap-4px rounded-full border border-solid border-[var(--color-border-2)] bg-[var(--color-bg-2)] px-7px transition-colors hover:border-[var(--color-border-3)] hover:bg-[var(--color-fill-1)]'
      data-testid={`kb-tag-row-${tag.key}`}
    >
      <Popover
        trigger='click'
        position='bl'
        popupVisible={colorOpen}
        onVisibleChange={(open) => !busy && setColorOpen(open)}
        content={
          <div className='p-2px' onClick={(event) => event.stopPropagation()}>
            <ColorPicker
              value={tag.color}
              onChange={(color) => {
                onChangeColor(tag.key, color);
                setColorOpen(false);
              }}
            />
          </div>
        }
      >
        <div
          role='button'
          tabIndex={0}
          onKeyDown={(event) => {
            if ((event.key === 'Enter' || event.key === ' ') && !busy) {
              event.preventDefault();
              setColorOpen((open) => !open);
            }
          }}
          className='inline-flex flex-shrink-0 cursor-pointer rounded-full focus-visible:outline-none'
          title={t('knowledge.tags.changeColor', { defaultValue: 'Change color' })}
        >
          <ColorDot color={tag.color} size={10} />
        </div>
      </Popover>

      {editing ? (
        <>
          <Input
            size='mini'
            autoFocus
            value={draft}
            onChange={setDraft}
            onPressEnter={commit}
            disabled={busy}
            className='w-120px !rounded-6px'
          />
          <div
            role='button'
            tabIndex={0}
            onClick={commit}
            onKeyDown={(event) => {
              if (event.key === 'Enter') commit();
            }}
            className='flex h-20px w-20px flex-shrink-0 cursor-pointer items-center justify-center rounded-full text-primary-6 transition-colors hover:bg-[var(--color-primary-light-1)]'
          >
            <Check theme='outline' size={12} strokeWidth={3} />
          </div>
          <div
            role='button'
            tabIndex={0}
            onClick={() => setEditing(false)}
            onKeyDown={(event) => {
              if (event.key === 'Enter') setEditing(false);
            }}
            className='flex h-20px w-20px flex-shrink-0 cursor-pointer items-center justify-center rounded-full text-[var(--color-text-3)] transition-colors hover:bg-[var(--color-fill-2)]'
          >
            <Close theme='outline' size={12} strokeWidth={3} />
          </div>
        </>
      ) : (
        <>
          <span
            role='button'
            tabIndex={0}
            onClick={beginRename}
            onKeyDown={(event) => {
              if (event.key === 'Enter' || event.key === ' ') {
                event.preventDefault();
                beginRename();
              }
            }}
            className='max-w-160px min-w-0 cursor-text truncate text-12px font-500 leading-16px text-[var(--color-text-1)]'
            title={t('knowledge.tags.renameHint', { defaultValue: 'Click to rename' })}
          >
            {tag.label}
          </span>

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
              aria-label={t('common.delete', { defaultValue: 'Delete' })}
              data-testid={`kb-tag-delete-${tag.key}`}
              onKeyDown={(event) => {
                if (event.key === 'Enter' || event.key === ' ') {
                  event.preventDefault();
                  event.currentTarget.click();
                }
              }}
              className='flex h-16px w-16px flex-shrink-0 cursor-pointer items-center justify-center rounded-full text-[var(--color-text-3)] transition-colors hover:bg-[rgba(var(--danger-6),0.08)] hover:text-danger-6'
            >
              <CloseSmall theme='outline' size={10} strokeWidth={3} />
            </div>
          </Popconfirm>
        </>
      )}
    </div>
  );
};

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
    if (!label || busy) return;
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
      style={{ width: 520, maxWidth: '92vw', maxHeight: '82vh', borderRadius: 16 }}
      maskClosable={!busy}
      data-testid='kb-tag-management-modal'
    >
      <p className='mt-0 mb-12px text-12px leading-18px text-[var(--color-text-3)]'>
        {t('knowledge.tags.modalDesc', {
          defaultValue: 'Organize knowledge bases with tags. Tags can be renamed, recolored, or deleted.',
        })}
      </p>

      <div
        className='mb-12px flex flex-wrap content-start items-start gap-6px overflow-y-auto pr-4px'
        style={{ maxHeight: 'min(38vh, 280px)' }}
        data-testid='kb-tag-list'
      >
        {tags.length === 0 ? (
          <div className='w-full rounded-10px border border-dashed border-[var(--color-border-2)] px-10px py-12px text-center text-12px text-[var(--color-text-3)]'>
            {t('knowledge.tags.empty', { defaultValue: 'No tags yet. Create one below.' })}
          </div>
        ) : (
          tags.map((tag) => (
            <TagChip
              key={tag.key}
              tag={tag}
              busy={busy}
              onRename={handleRename}
              onChangeColor={handleChangeColor}
              onDelete={handleDelete}
            />
          ))
        )}
      </div>

      <div className='flex flex-col gap-8px border-0 border-t border-solid border-[var(--color-border-2)] pt-12px'>
        <div className='flex items-center gap-8px'>
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
            onKeyDown={(event) => {
              if (event.key === 'Enter' || event.key === ' ') {
                event.preventDefault();
                void handleCreate();
              }
            }}
            className={[
              'inline-flex h-30px flex-shrink-0 cursor-pointer items-center gap-4px rounded-8px px-10px text-12px font-medium leading-none',
              'border border-solid transition-all duration-150',
              newLabel.trim() && !busy
                ? 'border-primary-6 bg-primary-6 text-white hover:opacity-90'
                : 'cursor-not-allowed border-[var(--color-border-2)] bg-[var(--color-fill-2)] text-[var(--color-text-3)]',
            ].join(' ')}
          >
            <span className='inline-flex h-14px w-14px flex-none items-center justify-center leading-none [&_svg]:block'>
              <Plus theme='outline' size={13} strokeWidth={3} fill='currentColor' className='block' />
            </span>
            <span className='inline-flex h-16px items-center leading-16px'>
              {t('common.add', { defaultValue: 'Add' })}
            </span>
          </div>
        </div>
        <div className='pl-22px'>
          <ColorPicker value={newColor} onChange={setNewColor} />
        </div>
      </div>
    </Modal>
  );
};

export default KnowledgeTagManagementModal;
