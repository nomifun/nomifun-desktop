/**
 * DeletePresetModal — Confirmation modal for deleting an preset.
 */
import type { PresetListItem } from './types';
import PresetAvatar from './PresetAvatar';
import { Modal } from '@arco-design/web-react';
import React from 'react';
import { useTranslation } from 'react-i18next';

type DeletePresetModalProps = {
  visible: boolean;
  onCancel: () => void;
  onConfirm: () => void;
  activePreset: PresetListItem | null;
  avatarImageMap: Record<string, string>;
};

const DeletePresetModal: React.FC<DeletePresetModalProps> = ({
  visible,
  onCancel,
  onConfirm,
  activePreset,
  avatarImageMap,
}) => {
  const { t } = useTranslation();

  return (
    <Modal
      title={t('settings.deletePresetTitle', { defaultValue: 'Delete Preset' })}
      visible={visible}
      onCancel={onCancel}
      onOk={onConfirm}
      okButtonProps={{ status: 'danger' }}
      wrapClassName='delete-preset-modal'
      data-testid='modal-delete-preset'
      okText={t('common.delete', { defaultValue: 'Delete' })}
      cancelText={t('common.cancel', { defaultValue: 'Cancel' })}
      className='w-[90vw] md:w-[400px]'
      wrapStyle={{ zIndex: 10000 }}
      maskStyle={{ zIndex: 9999 }}
    >
      <p>
        {t('settings.deletePresetConfirm', {
          defaultValue: 'Are you sure you want to delete this preset? This action cannot be undone.',
        })}
      </p>
      {activePreset && (
        <div className='mt-12px p-12px bg-fill-2 rounded-lg flex items-center gap-12px'>
          <PresetAvatar preset={activePreset} size={32} avatarImageMap={avatarImageMap} />
          <div>
            <div className='font-medium'>{activePreset.name}</div>
            <div className='text-12px text-t-secondary'>{activePreset.description}</div>
          </div>
        </div>
      )}
    </Modal>
  );
};

export default DeletePresetModal;
