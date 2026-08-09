/**
 * Compact two-dimension tag vocabulary management for presets. Built-in tags
 * remain locked while user tags support inline rename and confirmed deletion.
 */
import type {
  PresetTag,
  PresetTagDimension,
  CreatePresetTagRequest,
} from '@/common/types/agent/presetTypes';
import type { PresetTagId } from '@/common/types/ids';
import type { ArcoMessageInstance } from '@/renderer/utils/ui/useArcoMessage';
import { Input, Modal } from '@arco-design/web-react';
import { Check, Close, CloseSmall, Lock, Plus } from '@icon-park/react';
import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';

type TagManagementModalProps = {
  visible: boolean;
  onClose: () => void;
  audienceTags: PresetTag[];
  scenarioTags: PresetTag[];
  localeKey: string;
  onCreate: (req: CreatePresetTagRequest) => Promise<unknown>;
  onRename: (presetTagId: PresetTagId, label: string) => Promise<void>;
  onDelete: (presetTagId: PresetTagId) => Promise<void>;
  message: ArcoMessageInstance;
};

const errorText = (error: unknown): string => {
  if (error instanceof Error) return error.message;
  if (typeof error === 'string') return error;
  return '';
};

const TagChip: React.FC<{
  tag: PresetTag;
  localeKey: string;
  busy: boolean;
  onRename: (presetTagId: PresetTagId, label: string) => void;
  onDelete: (tag: PresetTag) => void;
}> = ({ tag, localeKey, busy, onRename, onDelete }) => {
  const { t } = useTranslation();
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState('');
  const label = tag.label_i18n?.[localeKey] || tag.label;
  const builtinLabel = t('settings.presetTagBuiltinLocked', { defaultValue: 'Built-in tag' });

  if (tag.builtin) {
    return (
      <div
        className='box-border inline-flex h-28px max-w-full items-center gap-4px rounded-full border border-solid border-transparent bg-[var(--color-fill-1)] px-7px text-[var(--color-text-2)] opacity-65'
        data-testid={`tag-row-${tag.key}`}
        title={`${label} · ${builtinLabel}`}
      >
        <Lock theme='outline' size={10} className='flex-shrink-0 text-[var(--color-text-3)]' />
        <span className='max-w-150px min-w-0 truncate text-12px leading-16px'>{label}</span>
      </div>
    );
  }

  const commit = () => {
    const next = draft.trim();
    if (next && next !== label) {
      onRename(tag.preset_tag_id, next);
    }
    setEditing(false);
  };

  const beginRename = () => {
    if (busy) return;
    setDraft(label);
    setEditing(true);
  };

  return (
    <div
      className='group box-border inline-flex h-28px max-w-full items-center gap-4px rounded-full border border-solid border-[var(--color-border-2)] bg-[var(--color-bg-2)] px-7px transition-colors hover:border-[var(--color-border-3)] hover:bg-[var(--color-fill-1)]'
      data-testid={`tag-row-${tag.key}`}
    >
      {editing ? (
        <>
          <Input
            size='mini'
            autoFocus
            value={draft}
            onChange={setDraft}
            onPressEnter={commit}
            disabled={busy}
            className='w-110px !rounded-6px'
          />
          <div
            role='button'
            tabIndex={0}
            onClick={commit}
            onKeyDown={(event) => {
              if (event.key === 'Enter') commit();
            }}
            className='flex h-16px w-16px flex-shrink-0 cursor-pointer items-center justify-center rounded-full text-primary-6 transition-colors hover:bg-[var(--color-primary-light-1)]'
          >
            <Check theme='outline' size={10} strokeWidth={3} />
          </div>
          <div
            role='button'
            tabIndex={0}
            onClick={() => setEditing(false)}
            onKeyDown={(event) => {
              if (event.key === 'Enter') setEditing(false);
            }}
            className='flex h-16px w-16px flex-shrink-0 cursor-pointer items-center justify-center rounded-full text-[var(--color-text-3)] transition-colors hover:bg-[var(--color-fill-2)]'
          >
            <Close theme='outline' size={10} strokeWidth={3} />
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
            className='max-w-150px min-w-0 cursor-text truncate text-12px font-500 leading-16px text-[var(--color-text-1)]'
            title={t('settings.presetTagRenameHint', { defaultValue: 'Click to rename' })}
          >
            {label}
          </span>
          <div
            role='button'
            tabIndex={0}
            aria-label={`${t('common.delete', { defaultValue: 'Delete' })}: ${label}`}
            data-testid={`tag-delete-${tag.key}`}
            onClick={() => !busy && onDelete(tag)}
            onKeyDown={(event) => {
              if ((event.key === 'Enter' || event.key === ' ') && !busy) {
                event.preventDefault();
                onDelete(tag);
              }
            }}
            className='flex h-16px w-16px flex-shrink-0 cursor-pointer items-center justify-center rounded-full text-[var(--color-text-3)] transition-colors hover:bg-[rgba(var(--danger-6),0.08)] hover:text-danger-6'
          >
            <CloseSmall theme='outline' size={10} strokeWidth={3} />
          </div>
        </>
      )}
    </div>
  );
};

const TagColumn: React.FC<{
  title: string;
  dimension: PresetTagDimension;
  tags: PresetTag[];
  localeKey: string;
  busy: boolean;
  onCreate: (label: string) => void;
  onRename: (presetTagId: PresetTagId, label: string) => void;
  onDelete: (tag: PresetTag) => void;
}> = ({ title, dimension, tags, localeKey, busy, onCreate, onRename, onDelete }) => {
  const { t } = useTranslation();
  const [newLabel, setNewLabel] = useState('');

  const submit = () => {
    const label = newLabel.trim();
    if (!label || busy) return;
    onCreate(label);
    setNewLabel('');
  };

  return (
    <section className='flex min-w-0 flex-col gap-8px'>
      <div className='flex items-center gap-7px'>
        <span className='inline-block h-13px w-3px rounded-[2px] bg-[var(--color-primary-light-3)]' aria-hidden='true' />
        <span className='text-13px font-medium text-[var(--color-text-1)]'>{title}</span>
        <span className='text-11px text-[var(--color-text-3)]'>({tags.length})</span>
      </div>

      <div
        className='flex flex-wrap content-start items-start gap-6px overflow-y-auto pr-4px'
        style={{ maxHeight: 'min(30vh, 200px)' }}
        data-testid={`tag-column-${dimension}`}
      >
        {tags.length === 0 ? (
          <div className='w-full rounded-10px border border-dashed border-[var(--color-border-2)] px-10px py-12px text-center text-12px text-[var(--color-text-3)]'>
            {t('settings.presetTagColumnEmpty', { defaultValue: 'No tags in this group yet.' })}
          </div>
        ) : (
          tags.map((tag) => (
            <TagChip
              key={tag.preset_tag_id}
              tag={tag}
              localeKey={localeKey}
              busy={busy}
              onRename={onRename}
              onDelete={onDelete}
            />
          ))
        )}
      </div>

      <div className='mt-2px flex items-center gap-8px border-0 border-t border-solid border-[var(--color-border-2)] pt-10px'>
        <Input
          size='small'
          value={newLabel}
          onChange={setNewLabel}
          onPressEnter={submit}
          disabled={busy}
          data-testid={`tag-add-input-${dimension}`}
          placeholder={t('settings.presetTagAddPlaceholder', { defaultValue: 'New tag…' })}
          className='flex-1 !rounded-8px'
        />
        <div
          role='button'
          tabIndex={0}
          data-testid={`tag-add-btn-${dimension}`}
          onClick={submit}
          onKeyDown={(event) => {
            if (event.key === 'Enter' || event.key === ' ') {
              event.preventDefault();
              submit();
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
    </section>
  );
};

const TagManagementModal: React.FC<TagManagementModalProps> = ({
  visible,
  onClose,
  audienceTags,
  scenarioTags,
  localeKey,
  onCreate,
  onRename,
  onDelete,
  message,
}) => {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);

  const handleCreate = async (dimension: PresetTagDimension, label: string) => {
    if (busy) return;
    setBusy(true);
    try {
      await onCreate({ dimension, label });
    } catch (error) {
      console.error('Failed to create tag:', error);
      message.error(
        errorText(error) || t('settings.presetTagCreateFailed', { defaultValue: 'Failed to create tag' })
      );
    } finally {
      setBusy(false);
    }
  };

  const handleRename = async (presetTagId: PresetTagId, label: string) => {
    if (busy) return;
    setBusy(true);
    try {
      await onRename(presetTagId, label);
    } catch (error) {
      console.error('Failed to rename tag:', error);
      message.error(
        errorText(error) || t('settings.presetTagRenameFailed', { defaultValue: 'Failed to rename tag' })
      );
    } finally {
      setBusy(false);
    }
  };

  const handleDelete = (tag: PresetTag) => {
    const label = tag.label_i18n?.[localeKey] || tag.label;
    Modal.confirm({
      title: t('settings.presetTagDeleteTitle', { defaultValue: 'Delete tag' }),
      content: t('settings.presetTagDeleteConfirm', {
        defaultValue: 'Delete "{{label}}"? It will be removed from all presets.',
        label,
      }),
      okText: t('common.delete', { defaultValue: 'Delete' }),
      cancelText: t('common.cancel', { defaultValue: 'Cancel' }),
      okButtonProps: { status: 'danger' },
      onOk: async () => {
        setBusy(true);
        try {
          await onDelete(tag.preset_tag_id);
        } catch (error) {
          console.error('Failed to delete tag:', error);
          message.error(
            errorText(error) || t('settings.presetTagDeleteFailed', { defaultValue: 'Failed to delete tag' })
          );
        } finally {
          setBusy(false);
        }
      },
    });
  };

  return (
    <Modal
      visible={visible}
      onCancel={onClose}
      footer={null}
      title={t('settings.presetTagModalTitle', { defaultValue: 'Manage Tags' })}
      style={{ width: 680, maxWidth: '92vw', maxHeight: '82vh', borderRadius: 16 }}
      maskClosable={!busy}
      data-testid='tag-management-modal'
    >
      <p className='mt-0 mb-12px text-12px leading-18px text-[var(--color-text-3)]'>
        {t('settings.presetTagModalDesc', {
          defaultValue:
            'Organize presets by audience and skill scenario. Built-in tags are locked; your own tags can be renamed or deleted.',
        })}
      </p>
      <div
        className='grid gap-18px'
        style={{ gridTemplateColumns: 'repeat(auto-fit, minmax(min(240px, 100%), 1fr))' }}
      >
        <TagColumn
          title={t('settings.presetTagAudience', { defaultValue: 'Audience' })}
          dimension='audience'
          tags={audienceTags}
          localeKey={localeKey}
          busy={busy}
          onCreate={(label) => void handleCreate('audience', label)}
          onRename={(key, label) => void handleRename(key, label)}
          onDelete={handleDelete}
        />
        <TagColumn
          title={t('settings.presetTagScenario', { defaultValue: 'Skill Scenario' })}
          dimension='scenario'
          tags={scenarioTags}
          localeKey={localeKey}
          busy={busy}
          onCreate={(label) => void handleCreate('scenario', label)}
          onRename={(key, label) => void handleRename(key, label)}
          onDelete={handleDelete}
        />
      </div>
    </Modal>
  );
};

export default TagManagementModal;
