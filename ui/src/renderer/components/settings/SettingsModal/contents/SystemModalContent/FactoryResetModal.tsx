/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import { configService } from '@/common/config/configService';
import { isElectronDesktop } from '@/renderer/utils/platform';
import { Alert, Button, Input, Message, Modal } from '@arco-design/web-react';
import React, { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

interface FactoryResetModalProps {
  visible: boolean;
  onClose: () => void;
}

/**
 * Factory reset confirmation modal (type-to-confirm).
 *
 * This is the most destructive, irreversible action in the app, so a single
 * danger button is not enough — the user must type an exact confirmation phrase
 * before the button is enabled.
 *
 * Flow on confirm: arm the reset on the backend (writes a marker; nothing is
 * deleted yet) -> clear front-end residual UI state -> relaunch. The actual
 * database + derived-data wipe happens early on the next boot, before the DB
 * pool opens or any background loop starts (see nomifun_common::factory_reset).
 * On WebUI, relaunch is a no-op, so we tell the user to restart the service.
 */
const FactoryResetModal: React.FC<FactoryResetModalProps> = ({ visible, onClose }) => {
  const { t } = useTranslation();
  const [input, setInput] = useState('');
  const [loading, setLoading] = useState(false);

  const phrase = t('settings.factoryReset.confirmPhrase');
  const matched = input.trim() === phrase;

  // Reset the typed phrase whenever the modal is reopened.
  useEffect(() => {
    if (!visible) {
      setInput('');
      setLoading(false);
    }
  }, [visible]);

  const handleConfirm = async () => {
    if (!matched || loading) return;
    setLoading(true);
    try {
      // 1. Arm the reset (backend writes the marker; wipe happens on next boot).
      await ipcBridge.application.factoryReset.invoke();
      // 2. Clear front-end residual state so the relaunch lands truly fresh
      //    (localStorage holds theme / collapse / recent-workspace UI bits that
      //    survive a desktop relaunch; configService caches client prefs).
      try {
        localStorage.clear();
      } catch {
        /* ignore */
      }
      configService.reset();
      Message.success(t('settings.factoryReset.armed'));
      // 3. Relaunch. Desktop: boot will perform the wipe. WebUI: no-op -> guide
      //    the user to restart the service manually.
      await ipcBridge.application.restart.invoke();
      if (!isElectronDesktop()) {
        Message.info(t('settings.factoryReset.restartManually'));
        setLoading(false);
        onClose();
      }
    } catch {
      Message.error(t('settings.factoryReset.failed'));
      setLoading(false);
    }
  };

  return (
    <Modal
      title={t('settings.factoryReset.title')}
      visible={visible}
      onCancel={loading ? undefined : onClose}
      maskClosable={!loading}
      escToExit={!loading}
      footer={
        <div className='flex justify-end gap-8px'>
          <Button onClick={onClose} disabled={loading}>
            {t('common.cancel')}
          </Button>
          <Button status='danger' type='primary' disabled={!matched} loading={loading} onClick={handleConfirm}>
            {t('settings.factoryReset.confirmButton')}
          </Button>
        </div>
      }
    >
      <div className='space-y-12px'>
        <Alert type='error' content={t('settings.factoryReset.warning')} />
        <div className='text-13px text-t-secondary leading-22px'>{t('settings.factoryReset.clearsIntro')}</div>
        <ul className='text-13px text-t-secondary leading-22px pl-18px list-disc space-y-2px'>
          <li>{t('settings.factoryReset.clearsConversations')}</li>
          <li>{t('settings.factoryReset.clearsRequirements')}</li>
          <li>{t('settings.factoryReset.clearsCompanions')}</li>
          <li>{t('settings.factoryReset.clearsKnowledge')}</li>
          <li>{t('settings.factoryReset.clearsSettings')}</li>
        </ul>
        <div className='text-13px text-t-secondary leading-22px'>{t('settings.factoryReset.restartNotice')}</div>
        <div className='text-13px text-t-primary leading-22px mt-4px'>
          {t('settings.factoryReset.typePrompt', { phrase })}
        </div>
        <Input
          value={input}
          onChange={setInput}
          placeholder={phrase}
          disabled={loading}
          onPressEnter={handleConfirm}
          autoComplete='off'
        />
      </div>
    </Modal>
  );
};

export default FactoryResetModal;
