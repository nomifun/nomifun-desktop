/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { IStartOnBootStatus } from '@/common/adapter/ipcBridge';
import { configService } from '@/common/config/configService';
import type {
  ThinkingContentDisplayLength,
  ThinkingSummaryDisplayLength,
} from '@/common/config/thinkingDisplay';
import FeedbackButton from '@/renderer/components/base/FeedbackButton';
import LanguageSwitcher from '@/renderer/components/settings/LanguageSwitcher';
import { isDesktopShell } from '@/renderer/utils/platform';
import { useKeepAwake } from '@renderer/hooks/ui/useKeepAwake';
import { useThinkingDisplayPreferences } from '@renderer/hooks/config/useThinkingDisplayPreferences';
import { useConfig } from '@/renderer/hooks/config/useConfig';
import { capabilityPermissionsHref } from '@/renderer/hooks/system/systemPermissionModel';
import { Alert, Button, Message, Modal } from '@arco-design/web-react';
import { FolderOpen, FolderSearch, Search } from '@icon-park/react';
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import useSWR from 'swr';
import { useSearchParams } from 'react-router-dom';
import { VisualChoice, VisualEmpty, VisualListRow, VisualPanel, VisualRow, VisualSearch, VisualSwitch, VisualTabs } from '@/renderer/pages/settings/components/CodeVisualPrimitives';
import PageHeader from '@/renderer/components/layout/PageHeader';
import FactoryResetModal from './FactoryResetModal';

/**
 * System settings content component
 *
 * Groups install-wide preferences without changing their configuration keys.
 */
const SystemModalContent: React.FC = () => {
  const { t } = useTranslation();
  const [searchParams, setSearchParams] = useSearchParams();
  const [query, setQuery] = useState('');
  const [changingDirectory, setChangingDirectory] = useState(false);
  const [committedWorkDir, setCommittedWorkDir] = useState<string>();
  // arco types Modal.useModal() methods (confirm/info/...) as optional even
  // though the hook always supplies them; assert the non-optional shape so
  // `modal.confirm(...)` doesn't trip TS2722.
  const [modalRaw, modalContextHolder] = Modal.useModal();
  const modal = modalRaw as Required<typeof modalRaw>;
  const [error, setError] = useState<string | null>(null);
  const savingRef = useRef(false);

  const [startOnBoot, setStartOnBoot] = useState<IStartOnBootStatus>({
    supported: false,
    enabled: false,
    isPackaged: false,
    platform: 'web',
  });
  const [notificationEnabled, setNotificationEnabled] = useState(true);
  const [cronNotificationEnabled, setCronNotificationEnabled] = useState(false);
  const [saveUploadToWorkspace, setSaveUploadToWorkspace] = useState(false);
  const [autoPreviewOfficeFiles, setAutoPreviewOfficeFiles] = useState(true);
  const [sendKey, setSendKey] = useState<'enter' | 'mod-enter'>('enter');
  const [factoryResetVisible, setFactoryResetVisible] = useState(false);
  const thinkingDisplay = useThinkingDisplayPreferences();
  const [showDecisionBasis, setShowDecisionBasis] = useConfig('chat.idmm.showDecisionBasis');

  useEffect(() => {
    // Start-on-boot is only meaningful in the Tauri desktop shell (backed by
    // tauri-plugin-autostart); the WebUI browser has no autostart concept.
    if (!isDesktopShell()) {
      return;
    }

    ipcBridge.application.getStartOnBootStatus
      .invoke()
      .then((result) => {
        if (result.success && result.data) {
          setStartOnBoot(result.data);
        }
      })
      .catch(() => {});
  }, []);

  useEffect(() => {
    setNotificationEnabled(configService.get('system.notificationEnabled') ?? true);
    setCronNotificationEnabled(configService.get('system.cronNotificationEnabled') ?? false);
    setSaveUploadToWorkspace(configService.get('upload.saveToWorkspace') ?? false);
    setAutoPreviewOfficeFiles(configService.get('system.autoPreviewOfficeFiles') ?? true);
    setSendKey(configService.get('chat.sendKey') ?? 'enter');
  }, []);

  const handleStartOnBootChange = useCallback(
    (checked: boolean) => {
      const previousStatus = startOnBoot;
      setStartOnBoot((prev) => ({ ...prev, enabled: checked }));

      return ipcBridge.application.setStartOnBoot
        .invoke({ enabled: checked })
        .then((result) => {
          if (result.success && result.data) {
            setStartOnBoot(result.data);
            return;
          }

          setStartOnBoot(previousStatus);
          Message.error(result.msg || t('settings.startOnBootUpdateFailed'));
        })
        .catch(() => {
          setStartOnBoot(previousStatus);
          Message.error(t('settings.startOnBootUpdateFailed'));
        });
    },
    [startOnBoot, t]
  );

  const ensureNotificationPermission = useCallback(async (): Promise<boolean> => {
    if (!isDesktopShell()) return true;
    try {
      let state = await ipcBridge.notification.permissionState.invoke();
      if (state !== 'granted') state = await ipcBridge.notification.requestPermission.invoke();
      if (state === 'granted') return true;
    } catch {
      // The permission page provides the durable recovery path below.
    }
    Message.error({
      duration: 6000,
      content: (
        <span className='inline-flex items-center gap-8px'>
          <span>{t('settings.notificationPermissionDenied')}</span>
          <a className='font-600 text-primary-6 no-underline' href={capabilityPermissionsHref('notifications')}>
            {t('settings.notificationPermissionManage')}
          </a>
        </span>
      ),
    });
    return false;
  }, [t]);

  const handleNotificationEnabledChange = useCallback(async (checked: boolean) => {
    if (checked && !(await ensureNotificationPermission())) return;
    setNotificationEnabled(checked);
    await configService.set('system.notificationEnabled', checked).catch(() => {
      setNotificationEnabled(!checked);
      configService.setLocal('system.notificationEnabled', !checked);
    });
  }, [ensureNotificationPermission]);

  const handleCronNotificationEnabledChange = useCallback(async (checked: boolean) => {
    if (checked && !(await ensureNotificationPermission())) return;
    setCronNotificationEnabled(checked);
    await configService.set('system.cronNotificationEnabled', checked).catch(() => {
      setCronNotificationEnabled(!checked);
      configService.setLocal('system.cronNotificationEnabled', !checked);
    });
  }, [ensureNotificationPermission]);

  const handleSaveUploadToWorkspaceChange = useCallback((checked: boolean) => {
    setSaveUploadToWorkspace(checked);
    return configService.set('upload.saveToWorkspace', checked).catch(() => {
      setSaveUploadToWorkspace(!checked);
      configService.setLocal('upload.saveToWorkspace', !checked);
    });
  }, []);

  const handleAutoPreviewOfficeFilesChange = useCallback((checked: boolean) => {
    setAutoPreviewOfficeFiles(checked);
    return configService.set('system.autoPreviewOfficeFiles', checked).catch(() => {
      setAutoPreviewOfficeFiles(!checked);
      configService.setLocal('system.autoPreviewOfficeFiles', !checked);
    });
  }, []);

  const handleSendKeyChange = useCallback((value: 'enter' | 'mod-enter') => {
    setSendKey(value);
    configService.set('chat.sendKey', value).catch(() => {
      const fallback = value === 'enter' ? 'mod-enter' : 'enter';
      setSendKey(fallback);
      configService.setLocal('chat.sendKey', fallback);
    });
  }, []);

  const handleThinkingVisibleChange = useCallback(
    (checked: boolean) => {
      const previous = thinkingDisplay.visible;
      return configService.set('chat.thinking.visible', checked).catch(() => {
        configService.setLocal('chat.thinking.visible', previous);
        Message.error(t('settings.thinkingDisplaySaveFailed'));
      });
    },
    [t, thinkingDisplay.visible]
  );

  const handleThinkingContentLengthChange = useCallback(
    (value: ThinkingContentDisplayLength) => {
      const previous = thinkingDisplay.contentLength;
      configService.set('chat.thinking.contentLength', value).catch(() => {
        configService.setLocal('chat.thinking.contentLength', previous);
        Message.error(t('settings.thinkingDisplaySaveFailed'));
      });
    },
    [t, thinkingDisplay.contentLength]
  );

  const handleThinkingSummaryLengthChange = useCallback(
    (value: ThinkingSummaryDisplayLength) => {
      const previous = thinkingDisplay.summaryLength;
      configService.set('chat.thinking.summaryLength', value).catch(() => {
        configService.setLocal('chat.thinking.summaryLength', previous);
        Message.error(t('settings.thinkingDisplaySaveFailed'));
      });
    },
    [t, thinkingDisplay.summaryLength]
  );

  const { keepAwake, setKeepAwake: applyKeepAwake } = useKeepAwake();
  const handleDecisionBasisChange = useCallback((checked: boolean) => {
    const previous = showDecisionBasis;
    return setShowDecisionBasis(checked).catch(() => {
      configService.setLocal('chat.idmm.showDecisionBasis', previous);
      Message.error(t('settings.idmmDecisionBasisSaveFailed'));
    });
  }, [setShowDecisionBasis, showDecisionBasis, t]);

  const handleKeepAwakeChange = useCallback(async (checked: boolean) => {
    try { await applyKeepAwake(checked); } catch (err) { Message.error(String(err)); }
  }, [applyKeepAwake]);

  // Get system directory info
  const { data: systemInfo } = useSWR('system.dir.info', () => ipcBridge.application.systemInfo.invoke());

  const handleOpenLogDir = useCallback(() => {
    if (!systemInfo?.logDir) return;
    void ipcBridge.shell.openFolderWith
      .invoke({ folder_path: systemInfo.logDir, tool: 'explorer' })
      .catch((caughtError) => {
        console.error('[SystemModalContent] Failed to open log directory:', caughtError);
      });
  }, [systemInfo?.logDir]);

  const handlePickWorkDir = useCallback(async () => {
    if (!systemInfo || savingRef.current) return;
    savingRef.current = true;
    setChangingDirectory(true);
    setError(null);
    try {
      const selected = await ipcBridge.dialog.showOpen.invoke({
        defaultPath: committedWorkDir ?? systemInfo.workDir,
        properties: ['openDirectory', 'createDirectory'],
      });
      const workDir = selected?.[0];
      if (!workDir || workDir === (committedWorkDir ?? systemInfo.workDir)) return;
      const confirmed = await new Promise<boolean>((resolve) => {
        modal.confirm({
          title: t('settings.workDirChangeConfirmTitle'),
          content: t('settings.workDirChangeConfirmContent'),
          onOk: () => resolve(true),
          onCancel: () => resolve(false),
        });
      });
      if (!confirmed) return;
      await ipcBridge.application.updateSystemInfo.invoke({ cacheDir: systemInfo.cacheDir, workDir });
      // Once persisted, show the committed directory even if relaunch fails.
      setCommittedWorkDir(workDir);
      await ipcBridge.application.restart.invoke();
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    } finally {
      savingRef.current = false;
      setChangingDirectory(false);
    }
  }, [committedWorkDir, modal, systemInfo, t]);

  const switchControl = (label: string, checked: boolean, onChange: (checked: boolean) => void | Promise<void>, disabled = false) =>
    <VisualSwitch label={label} checked={checked} onChange={onChange} disabled={disabled} />;

  const groups: Array<{
    key: string; title: string; description: string;
    rows: Array<{ key: string; label: string; description?: string; disabled?: boolean; control: React.ReactNode }>;
    directories?: Array<{ key: string; label: string; path?: string; icon: React.ReactNode; action: React.ReactNode }>;
  }> = [
    {
      key: 'preferences', title: t('settings.workspace.preferences'), description: t('settings.workspace.preferencesDesc'),
      rows: [
        { key: 'language', label: t('settings.language'), description: t('settings.languagePreferenceDesc'), control: <LanguageSwitcher /> },
        { key: 'startOnBoot', label: t('settings.startOnBoot'), description: startOnBoot.supported ? t('settings.startOnBootDesc') : t('settings.startOnBootUnsupported'), control: switchControl(t('settings.startOnBoot'), startOnBoot.enabled, handleStartOnBootChange, !startOnBoot.supported) },
        { key: 'keepAwake', label: t('settings.keepAwake'), description: t('settings.keepAwakeDesc'), control: switchControl(t('settings.keepAwake'), keepAwake, handleKeepAwakeChange) },
      ],
    },
    {
      key: 'conversation', title: t('settings.workspace.conversation'), description: t('settings.workspace.conversationDesc'),
      rows: [
        { key: 'sendKey', label: t('settings.sendKey'), description: t('settings.sendKeyDesc'), control:
          <VisualChoice label={t('settings.sendKey')} value={sendKey} onChange={handleSendKeyChange} options={[
            { value: 'enter', label: t('settings.sendKeyEnter') }, { value: 'mod-enter', label: t('settings.sendKeyModEnter') },
          ]} /> },
        { key: 'thinkingVisible', label: t('settings.thinkingProcessVisible'), description: t('settings.thinkingProcessVisibleDesc'), control: switchControl(t('settings.thinkingProcessVisible'), thinkingDisplay.visible, handleThinkingVisibleChange) },
        { key: 'thinkingContentLength', label: t('settings.thinkingContentLength'), description: t('settings.thinkingContentLengthDesc'), disabled: !thinkingDisplay.visible, control:
          <VisualChoice<ThinkingContentDisplayLength> label={t('settings.thinkingContentLength')} value={thinkingDisplay.contentLength} disabled={!thinkingDisplay.visible} onChange={handleThinkingContentLengthChange} options={[
            { value: 'compact', label: t('settings.thinkingContentCompact') }, { value: 'full', label: t('settings.thinkingContentFull') },
          ]} /> },
        { key: 'thinkingSummaryLength', label: t('settings.thinkingSummaryLength'), description: t('settings.thinkingSummaryLengthDesc'), disabled: !thinkingDisplay.visible, control:
          <VisualChoice<ThinkingSummaryDisplayLength> label={t('settings.thinkingSummaryLength')} value={thinkingDisplay.summaryLength} disabled={!thinkingDisplay.visible} onChange={handleThinkingSummaryLengthChange} options={[
            { value: 'hidden', label: t('settings.thinkingSummaryHidden') }, { value: 'shown', label: t('settings.thinkingSummaryShown') },
          ]} /> },
        { key: 'idmmDecisionBasis', label: t('settings.idmmDecisionBasis'), description: t('settings.idmmDecisionBasisDesc'), control: switchControl(t('settings.idmmDecisionBasis'), showDecisionBasis === true, handleDecisionBasisChange) },
      ],
    },
    {
      key: 'files', title: t('settings.workspace.files'), description: t('settings.workspace.filesDesc'),
      rows: [
        { key: 'saveUploadToWorkspace', label: t('settings.saveUploadToWorkspace'), control: switchControl(t('settings.saveUploadToWorkspace'), saveUploadToWorkspace, handleSaveUploadToWorkspaceChange) },
        { key: 'autoPreviewOfficeFiles', label: t('settings.autoPreviewOfficeFiles'), description: t('settings.autoPreviewOfficeFilesDesc'), control: switchControl(t('settings.autoPreviewOfficeFiles'), autoPreviewOfficeFiles, handleAutoPreviewOfficeFilesChange) },
      ],
      directories: [
        { key: 'workDir', label: t('settings.workDir'), path: committedWorkDir ?? systemInfo?.workDir, icon: <FolderOpen theme='outline' size={17} />, action:
          <Button size='small' loading={changingDirectory} disabled={!systemInfo} onClick={() => void handlePickWorkDir()}>{t('settings.workspace.changeDirectory')}</Button> },
        { key: 'logDir', label: t('settings.logDir'), path: systemInfo?.logDir, icon: <FolderSearch theme='outline' size={17} />, action:
          <Button size='small' disabled={!systemInfo?.logDir} onClick={handleOpenLogDir}>{t('settings.workspace.openDirectory')}</Button> },
      ],
    },
    {
      key: 'notifications', title: t('settings.notification'), description: t('settings.workspace.notificationsDesc'),
      rows: [
        { key: 'notification', label: t('settings.notification'), control: switchControl(t('settings.notification'), notificationEnabled, handleNotificationEnabledChange) },
        { key: 'cronNotificationEnabled', label: t('settings.cronNotificationEnabled'), description: t('settings.workspace.cronNotificationDesc'), disabled: !notificationEnabled, control: switchControl(t('settings.cronNotificationEnabled'), cronNotificationEnabled, handleCronNotificationEnabledChange, !notificationEnabled) },
      ],
    },
    {
      key: 'data', title: t('settings.workspace.data'), description: t('settings.workspace.dataDesc'),
      rows: [{ key: 'factoryReset', label: t('settings.factoryReset.title'), description: t('settings.factoryReset.rowDesc'), control:
        <Button status='danger' onClick={() => setFactoryResetVisible(true)}>{t('settings.factoryReset.button')}</Button> }],
    },
  ];
  const requestedSection = searchParams.get('section');
  const activeSection = groups.some((group) => group.key === requestedSection) ? requestedSection! : 'preferences';
  const search = query.trim().toLocaleLowerCase();
  const matches = (label: string, description?: string) => !search || (label + ' ' + (description ?? '')).toLocaleLowerCase().includes(search);
  const visibleGroups = groups.filter((group) => search || group.key === activeSection).map((group) => {
    const groupMatches = Boolean(search && matches(group.title, group.description));
    return { ...group, rows: group.rows.filter((row) => groupMatches || matches(row.label, row.description)),
      directories: group.directories?.filter((row) => groupMatches || matches(row.label, row.path)) };
  }).filter((group) => group.rows.length || group.directories?.length);

  return (
    <div>
      {modalContextHolder}
      <PageHeader title={t('settings.workspace.generalTitle')} />
      <div className='cv-toolbar'>
        <VisualSearch value={query} onChange={setQuery} label={t('settings.workspace.search')} clearLabel={t('settings.workspace.clearSearch')} />
        <span className='cv-toolbar__hint'>{t('settings.workspace.autoSave')}</span>
      </div>
      {!search && <VisualTabs id='general-settings' label={t('settings.workspace.generalTitle')}
        items={groups.map((group) => ({ key: group.key, label: group.title }))} activeKey={activeSection}
        onChange={(key) => setSearchParams((previous) => {
          const next = new URLSearchParams(previous); next.set('section', key); return next;
        }, { replace: true })} />}
      <div className='cv-stack' id='general-settings-panel' role={search ? undefined : 'tabpanel'}
        aria-labelledby={search ? undefined : 'general-settings-tab-' + activeSection}>
        {visibleGroups.map((group) => <VisualPanel key={group.key} label={group.title}
          title={search ? group.title : undefined} description={search ? group.description : undefined}
          className={group.key === 'data' ? 'cv-danger' : undefined}>
          {group.rows.map((row) => <VisualRow key={row.key} label={row.label} description={row.description} disabled={row.disabled}>{row.control}</VisualRow>)}
          {group.directories?.map((directory) => <VisualListRow key={directory.key} icon={directory.icon} title={directory.label}
            description={<span className='cv-directory-path'>{directory.path || t('settings.dirNotConfigured')}</span>} action={directory.action} />)}
        </VisualPanel>)}
        {!visibleGroups.length && <VisualEmpty icon={<Search theme='outline' size={22} />} title={t('settings.workspace.noResults')}
          description={t('settings.workspace.noResultsDesc')} action={<Button onClick={() => setQuery('')}>{t('settings.workspace.clearSearch')}</Button>} />}
        {error && <Alert type='error' content={<span>{error}<FeedbackButton className='ml-6px' /></span>} />}
      </div>
      <FactoryResetModal visible={factoryResetVisible} onClose={() => setFactoryResetVisible(false)} />
    </div>
  );
};

export default SystemModalContent;
