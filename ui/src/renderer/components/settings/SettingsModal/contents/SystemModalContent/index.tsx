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
import LanguageSwitcher from '@/renderer/components/settings/LanguageSwitcher';
import { isDesktopShell } from '@/renderer/utils/platform';
import { useKeepAwake } from '@renderer/hooks/ui/useKeepAwake';
import { useThinkingDisplayPreferences } from '@renderer/hooks/config/useThinkingDisplayPreferences';
import { useConfig } from '@/renderer/hooks/config/useConfig';
import { useNotificationPermission } from '@/renderer/hooks/system/useNotificationPermission';
import { Alert, Button, Message } from '@arco-design/web-react';
import { Search } from '@icon-park/react';
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import useSWR from 'swr';
import { useLocation, useSearchParams } from 'react-router-dom';
import { VisualChoice, VisualEmpty, VisualPanel, VisualRow, VisualSearch, VisualSwitch } from '@/renderer/pages/settings/components/CodeVisualPrimitives';
import { generalSettingsHref, resolveGeneralSettingsFocus, resolveGeneralSettingsSection, type GeneralSettingsSection } from '@/renderer/pages/settings/components/generalSettingsNavigation';
import PageHeader from '@/renderer/components/layout/PageHeader';
import NotificationPermissionRow from '../NotificationPermissionRow';
import FactoryResetModal from './FactoryResetModal';
import StorageRelocationModal from './StorageRelocationModal';

/**
 * System settings content component
 *
 * Groups install-wide preferences without changing their configuration keys.
 */
const SystemModalContent: React.FC = () => {
  const { t } = useTranslation();
  const [searchParams] = useSearchParams();
  const location = useLocation();
  const pageRef = useRef<HTMLDivElement>(null);
  const groupRefs = useRef<Record<string, HTMLElement | null>>({});
  const searchScrollPosition = useRef(0);
  const [query, setQuery] = useState('');
  const [relocationVisible, setRelocationVisible] = useState(false);

  const [startOnBoot, setStartOnBoot] = useState<IStartOnBootStatus>({
    supported: false,
    enabled: false,
    isPackaged: false,
    platform: 'web',
  });
  const [notificationEnabled, setNotificationEnabled] = useState(true);
  const [cronNotificationEnabled, setCronNotificationEnabled] = useState(false);
  const [autoPreviewOfficeFiles, setAutoPreviewOfficeFiles] = useState(true);
  const [sendKey, setSendKey] = useState<'enter' | 'mod-enter'>('enter');
  const [factoryResetVisible, setFactoryResetVisible] = useState(false);
  const thinkingDisplay = useThinkingDisplayPreferences();
  const [showDecisionBasis, setShowDecisionBasis] = useConfig('chat.idmm.showDecisionBasis');
  const notificationPermission = useNotificationPermission();

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
    if (await notificationPermission.ensureGranted()) return true;
    Message.error({
      duration: 6000,
      content: (
        <span className='inline-flex items-center gap-8px'>
          <span>{t('settings.notificationPermissionDenied')}</span>
          <a className='font-600 text-primary-6 no-underline' href={'#' + generalSettingsHref('preferences', 'notifications')}>
            {t('settings.notificationPermissionManage')}
          </a>
        </span>
      ),
    });
    return false;
  }, [notificationPermission.ensureGranted, t]);

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
  const { data: systemInfo, mutate: refreshSystemInfo } = useSWR('system.dir.info', () => ipcBridge.application.systemInfo.invoke());
  const { data: relocationStatus, mutate: setRelocationStatus } = useSWR('system.storage-relocation', () => ipcBridge.storageRelocation.status.invoke());

  const handleOpenLogDir = useCallback(() => {
    if (!systemInfo?.logDir) return;
    void ipcBridge.shell.openFolderWith
      .invoke({ folder_path: systemInfo.logDir, tool: 'explorer' })
      .catch((caughtError) => {
        console.error('[SystemModalContent] Failed to open log directory:', caughtError);
      });
  }, [systemInfo?.logDir]);

  const switchControl = (label: string, checked: boolean, onChange: (checked: boolean) => void | Promise<void>, disabled = false) =>
    <VisualSwitch label={label} checked={checked} onChange={onChange} disabled={disabled} />;

  const groups: Array<{
    key: string; category: GeneralSettingsSection; title: string; description: string;
    rows: Array<{ key: string; label: string; description?: string; disabled?: boolean; disabledHint?: string; control: React.ReactNode }>;
    directories?: Array<{ key: string; label: string; path?: string; action: React.ReactNode }>;
    permissionRow?: boolean;
  }> = [
    {
      key: 'preferences', category: 'preferences', title: t('settings.workspace.generalGroup'), description: t('settings.workspace.preferencesDesc'),
      rows: [
        { key: 'language', label: t('settings.language'), description: t('settings.languagePreferenceDesc'), control: <LanguageSwitcher /> },
        { key: 'startOnBoot', label: t('settings.startOnBoot'), description: startOnBoot.supported ? t('settings.startOnBootDesc') : t('settings.startOnBootUnsupported'), control: switchControl(t('settings.startOnBoot'), startOnBoot.enabled, handleStartOnBootChange, !startOnBoot.supported) },
        { key: 'keepAwake', label: t('settings.keepAwake'), description: t('settings.keepAwakeDesc'), control: switchControl(t('settings.keepAwake'), keepAwake, handleKeepAwakeChange) },
      ],
    },
    {
      key: 'conversation', category: 'preferences', title: t('settings.workspace.conversation'), description: t('settings.workspace.conversationDesc'),
      rows: [
        { key: 'sendKey', label: t('settings.sendKey'), description: t('settings.sendKeyDesc'), control:
          <VisualChoice label={t('settings.sendKey')} value={sendKey} onChange={handleSendKeyChange} options={[
            { value: 'enter', label: t('settings.sendKeyEnter') }, { value: 'mod-enter', label: t('settings.sendKeyModEnter') },
          ]} /> },
        { key: 'thinkingVisible', label: t('settings.thinkingProcessVisible'), description: t('settings.thinkingProcessVisibleDesc'), control: switchControl(t('settings.thinkingProcessVisible'), thinkingDisplay.visible, handleThinkingVisibleChange) },
        { key: 'thinkingContentLength', label: t('settings.thinkingContentLength'), description: t('settings.thinkingContentLengthDesc'), disabled: !thinkingDisplay.visible, disabledHint: t('settings.workspace.enableThinkingFirst'), control:
          <VisualChoice<ThinkingContentDisplayLength> label={t('settings.thinkingContentLength')} value={thinkingDisplay.contentLength} disabled={!thinkingDisplay.visible} onChange={handleThinkingContentLengthChange} options={[
            { value: 'compact', label: t('settings.thinkingContentCompact') }, { value: 'full', label: t('settings.thinkingContentFull') },
          ]} /> },
        { key: 'thinkingSummaryLength', label: t('settings.thinkingSummaryLength'), description: t('settings.thinkingSummaryLengthDesc'), disabled: !thinkingDisplay.visible, disabledHint: t('settings.workspace.enableThinkingFirst'), control:
          <VisualChoice<ThinkingSummaryDisplayLength> label={t('settings.thinkingSummaryLength')} value={thinkingDisplay.summaryLength} disabled={!thinkingDisplay.visible} onChange={handleThinkingSummaryLengthChange} options={[
            { value: 'hidden', label: t('settings.thinkingSummaryHidden') }, { value: 'shown', label: t('settings.thinkingSummaryShown') },
          ]} /> },
        { key: 'idmmDecisionBasis', label: t('settings.idmmDecisionBasis'), description: t('settings.idmmDecisionBasisDesc'), control: switchControl(t('settings.idmmDecisionBasis'), showDecisionBasis === true, handleDecisionBasisChange) },
      ],
    },
    {
      key: 'notifications', category: 'preferences', title: t('settings.notification'),
      description: t('settings.workspace.notificationsDesc') + ' ' + t('settings.capabilityPermissions.notifications.description'),
      permissionRow: true,
      rows: [
        { key: 'notification', label: t('settings.receiveDesktopNotifications'), description: t('settings.receiveDesktopNotificationsDesc'), control: switchControl(t('settings.receiveDesktopNotifications'), notificationEnabled, handleNotificationEnabledChange) },
        { key: 'cronNotificationEnabled', label: t('settings.cronNotificationEnabled'), description: t('settings.workspace.cronNotificationDesc'), disabled: !notificationEnabled, disabledHint: t('settings.workspace.enableNotificationsFirst'), control: switchControl(t('settings.cronNotificationEnabled'), cronNotificationEnabled, handleCronNotificationEnabledChange, !notificationEnabled) },
      ],
    },
    {
      key: 'files', category: 'data', title: t('settings.workspace.fileProcessing'), description: t('settings.workspace.files') + ' ' + t('settings.workspace.filesDesc'),
      rows: [
        { key: 'autoPreviewOfficeFiles', label: t('settings.autoPreviewOfficeFiles'), description: t('settings.autoPreviewOfficeFilesDesc'), control: switchControl(t('settings.autoPreviewOfficeFiles'), autoPreviewOfficeFiles, handleAutoPreviewOfficeFilesChange) },
      ],
    },
    {
      key: 'storage', category: 'data', title: t('settings.workspace.storageAndLogs'), description: t('settings.workspace.filesDesc'),
      rows: [],
      directories: [
        { key: 'dataDir', label: t('settings.storageRelocation.location'), path: relocationStatus?.current_dir || systemInfo?.dataDir, action:
          <Button size='small' disabled={!systemInfo} onClick={() => setRelocationVisible(true)}>{t('settings.workspace.changeDirectory')}</Button> },
        { key: 'logDir', label: t('settings.logDir'), path: systemInfo?.logDir, action:
          <Button size='small' disabled={!systemInfo?.logDir} onClick={handleOpenLogDir}>{t('settings.workspace.openDirectory')}</Button> },
      ],
    },
    {
      key: 'reset', category: 'data', title: t('settings.workspace.resetGroup'), description: t('settings.workspace.dataDesc'),
      rows: [{ key: 'factoryReset', label: t('settings.factoryReset.button'), description: t('settings.factoryReset.rowDesc'), control:
        <Button status='danger' onClick={() => setFactoryResetVisible(true)}>{t('settings.factoryReset.button')}</Button> }],
    },
  ];
  const requestedSection = searchParams.get('section');
  const activeSection = resolveGeneralSettingsSection(requestedSection);
  const focusedGroup = resolveGeneralSettingsFocus(requestedSection, searchParams.get('focus'));
  const search = query.trim().toLocaleLowerCase();
  const matches = (label: string, description?: string) => !search || (label + ' ' + (description ?? '')).toLocaleLowerCase().includes(search);
  const visibleGroups = groups.filter((group) => search || group.category === activeSection).map((group) => {
    const categoryTitle = group.category === 'data' ? t('settings.workspace.data') : t('settings.workspace.preferences');
    const groupMatches = Boolean(search && matches(group.title + ' ' + categoryTitle, group.description));
    return { ...group, rows: group.rows.filter((row) => groupMatches || matches(row.label, row.description)),
      directories: group.directories?.filter((row) => groupMatches || matches(row.label, row.path)) };
  }).filter((group) => group.rows.length || group.directories?.length);

  useEffect(() => {
    searchScrollPosition.current = 0;
    setQuery('');
  }, [location.key]);
  useEffect(() => {
    if (search) return;
    const wrapper = pageRef.current?.closest('.settings-page-wrapper');
    const group = focusedGroup && groupRefs.current[focusedGroup];
    if (group) group.scrollIntoView?.({ block: 'start' });
    else if (wrapper) wrapper.scrollTop = searchScrollPosition.current;
  }, [activeSection, focusedGroup, location.key, Boolean(search)]);

  const handleSearch = (value: string) => {
    const wrapper = pageRef.current?.closest('.settings-page-wrapper');
    if (!search && value.trim()) searchScrollPosition.current = wrapper?.scrollTop ?? 0;
    setQuery(value);
  };

  return (
    <div ref={pageRef} className='general-settings-page'>
      <PageHeader title={search ? t('settings.workspace.searchResults') : t(activeSection === 'data' ? 'settings.workspace.data' : 'settings.workspace.preferences')}
        actions={<VisualSearch value={query} onChange={handleSearch} label={t('settings.workspace.search')} clearLabel={t('settings.workspace.clearSearch')} />} />
      <div className='cv-settings-groups'>
        {visibleGroups.map((group) => <section key={group.key} className='cv-settings-group'
          ref={(node) => { groupRefs.current[group.key] = node; }} aria-labelledby={'settings-group-' + group.key}>
          {search && <p className='cv-settings-group__context'>{t(group.category === 'data' ? 'settings.workspace.data' : 'settings.workspace.preferences')}</p>}
          <h2 id={'settings-group-' + group.key} className='cv-settings-group__title'>{group.title}</h2>
          <VisualPanel label={group.title}>
            {group.permissionRow && <NotificationPermissionRow permission={notificationPermission} />}
            {group.rows.map((row) => <VisualRow key={row.key} label={row.label}
              description={row.disabled && row.disabledHint ? <>{row.description}<span className='cv-row__note'>{row.disabledHint}</span></> : row.description}
              disabled={row.disabled}>{row.control}</VisualRow>)}
            {group.directories?.map((directory) => <VisualRow key={directory.key} label={directory.label}
              description={<span className='cv-directory-path'>{directory.path || t('settings.dirNotConfigured')}</span>}>{directory.action}</VisualRow>)}
          </VisualPanel>
        </section>)}
        {!visibleGroups.length && <VisualEmpty icon={<Search theme='outline' size={22} />} title={t('settings.workspace.noResults')}
          description={t('settings.workspace.noResultsDesc')} action={<Button onClick={() => handleSearch('')}>{t('settings.workspace.clearSearch')}</Button>} />}
        {relocationStatus?.restart_required && <Alert type='info' content={<div className='flex items-center justify-between gap-12px'>
          <span>{relocationStatus.cancellable
            ? t('settings.storageRelocation.pending', { path: relocationStatus.target_dir })
            : t('settings.storageRelocation.recoveryRequired')}</span>
          <Button size='small' onClick={() => setRelocationVisible(true)}>{t('settings.storageRelocation.manage')}</Button>
        </div>} />}
      </div>
      <FactoryResetModal visible={factoryResetVisible} onClose={() => setFactoryResetVisible(false)} />
      <StorageRelocationModal visible={relocationVisible} currentDir={systemInfo?.dataDir ?? ''} status={relocationStatus}
        onStatusChange={(next) => {
          void setRelocationStatus(next, false);
          if (next.phase === 'completed') void refreshSystemInfo();
        }} onClose={() => setRelocationVisible(false)} />
    </div>
  );
};

export default SystemModalContent;
