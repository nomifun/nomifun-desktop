/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  SystemPermissionEntry,
  SystemPermissionKind,
  SystemPermissionState,
} from '@/common/adapter/ipcBridge';
import { VisualListRow, VisualPanel, VisualRow, VisualStatus, VisualTabs, type VisualTabItem, type VisualTone } from '@/renderer/pages/settings/components/CodeVisualPrimitives';
import PageHeader from '@/renderer/components/layout/PageHeader';
import {
  SYSTEM_PERMISSION_AUDIT,
  computerPermissionsReady,
  permissionEntryIsReady,
  systemPermissionEntry,
  type PermissionCapabilityTab,
} from '@/renderer/hooks/system/systemPermissionModel';
import { useSystemPermissions } from '@/renderer/hooks/system/useSystemPermissions';
import { isDesktopShell } from '@/renderer/utils/platform';
import { Alert, Button, Message, Spin } from '@arco-design/web-react';
import {
  Computer,
  Earth,
  HeadsetOne,
  Refresh,
  Shield,
} from '@icon-park/react';
import React, { useCallback, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Navigate, useNavigate, useSearchParams } from 'react-router-dom';
import { computerTools } from '@/common/chat/computerSurface';
import ComputerDriverStatus from './ComputerDriverStatus';

const isCapabilityTab = (value: string | null): value is PermissionCapabilityTab =>
  value === 'overview' ||
  value === 'voice-input' ||
  value === 'computer-use' ||
  value === 'browser-use';

const readyState = (state: SystemPermissionState): boolean =>
  state === 'granted' || state === 'not_required';

const statusTone = (state: SystemPermissionState): VisualTone => {
  if (readyState(state)) return 'success';
  if (state === 'denied' || state === 'restricted') return 'danger';
  if (state === 'not_determined' || state === 'not_granted') return 'warning';
  return 'neutral';
};

type PermissionRowProps = {
  entry: Pick<SystemPermissionEntry, 'state' | 'can_request' | 'can_open_settings' | 'requires_restart_after_grant'> | undefined;
  kind: SystemPermissionKind;
  title: string;
  description: string;
  busy?: boolean;
  onRequest?: (kind: SystemPermissionKind) => void;
  onOpenSettings?: (kind: SystemPermissionKind) => void;
  extraAction?: React.ReactNode;
  actionsEnabled?: boolean;
};

const PermissionRow: React.FC<PermissionRowProps> = ({
  entry,
  kind,
  title,
  description,
  busy,
  onRequest,
  onOpenSettings,
  extraAction,
  actionsEnabled = true,
}) => {
  const { t } = useTranslation();
  const state = entry?.state ?? 'unknown';
  return <VisualRow
    label={<><span>{title}</span><VisualStatus tone={statusTone(state)}>{t(`settings.capabilityPermissions.states.${state}`)}</VisualStatus></>}
    description={<>{description}
      {entry?.requires_restart_after_grant && !readyState(state) && <span className='cv-row__note'>
        {t('settings.capabilityPermissions.restartAfterGrant')}
      </span>}
    </>}
  >
    {extraAction}
    {actionsEnabled && entry?.can_request && state !== 'denied' && state !== 'restricted' && (
      <Button size='small' type='primary' loading={busy} onClick={() => onRequest?.(kind)}>{t('settings.capabilityPermissions.requestAccess')}</Button>
    )}
    {actionsEnabled && entry?.can_open_settings && !readyState(state) && (
      <Button size='small' onClick={() => onOpenSettings?.(kind)}>{t('settings.capabilityPermissions.openSystemSettings')}</Button>
    )}
  </VisualRow>;
};

const CapabilityPermissionsContent: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [searchParams, setSearchParams] = useSearchParams();
  const desktopShell = isDesktopShell();
  const tab = isCapabilityTab(searchParams.get('tab')) ? searchParams.get('tab') as PermissionCapabilityTab : 'overview';
  const notificationRedirect = searchParams.get('tab') === 'notifications';
  const { error, loading, openSettings, refresh, request, status } = useSystemPermissions(!notificationRedirect);
  const [microphoneBusy, setMicrophoneBusy] = useState(false);

  const microphone = systemPermissionEntry(status, 'microphone');
  const accessibility = systemPermissionEntry(status, 'accessibility');
  const screenRecording = systemPermissionEntry(status, 'screen_recording');
  const computerReady = status?.platform !== 'macos' || computerPermissionsReady(status, computerTools.map(tool => tool.action_id));
  const voiceReady = permissionEntryIsReady(microphone);

  const setTab = useCallback((next: PermissionCapabilityTab) => {
    setSearchParams((previous) => {
      const updated = new URLSearchParams(previous);
      if (next === 'overview') updated.delete('tab');
      else updated.set('tab', next);
      return updated;
    }, { replace: true });
  }, [setSearchParams]);

  const requestPermission = useCallback(async (kind: SystemPermissionKind) => {
    const next = await request(kind);
    const entry = systemPermissionEntry(next, kind);
    if (
      (kind === 'accessibility' || kind === 'screen_recording') &&
      entry &&
      !permissionEntryIsReady(entry)
    ) {
      await openSettings(kind);
    }
  }, [openSettings, request]);

  const testMicrophone = useCallback(async () => {
    setMicrophoneBusy(true);
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      stream.getTracks().forEach((track) => track.stop());
      await refresh();
      Message.success(t('settings.capabilityPermissions.voice.testPassed'));
    } catch {
      await refresh();
      Message.error(t('settings.capabilityPermissions.voice.testFailed'));
    } finally {
      setMicrophoneBusy(false);
    }
  }, [refresh, t]);

  const tabItems: VisualTabItem[] = useMemo(() => [
    {
      key: 'overview',
      label: t('settings.capabilityPermissions.tabs.overview'),
      icon: <Shield theme='outline' size='15' strokeWidth={3} />,
    },
    {
      key: 'voice-input',
      label: t('settings.capabilityPermissions.tabs.voice'),
      icon: <HeadsetOne theme='outline' size='15' strokeWidth={3} />,
      dot: Boolean(status && !voiceReady),
    },
    {
      key: 'computer-use',
      label: t('settings.capabilityPermissions.tabs.computer'),
      icon: <Computer theme='outline' size='15' strokeWidth={3} />,
      dot: Boolean(status && !computerReady),
    },
    {
      key: 'browser-use',
      label: t('settings.capabilityPermissions.tabs.browser'),
      icon: <Earth theme='outline' size='15' strokeWidth={3} />,
    },
  ], [computerReady, status, t, voiceReady]);

  const overview = (
    <div className='cv-stack'>
      <VisualPanel label={t('settings.capabilityPermissions.tabs.overview')}>
        {[
          { key: 'voice-input' as const, icon: <HeadsetOne theme='outline' size={18} />, title: t('settings.capabilityPermissions.voice.title'), description: t('settings.capabilityPermissions.voice.summary'), state: !microphone ? 'unknown' : voiceReady ? 'ready' : 'attention' },
          { key: 'computer-use' as const, icon: <Computer theme='outline' size={18} />, title: t('settings.capabilityPermissions.computer.title'), description: t('settings.capabilityPermissions.computer.summary'), state: !status ? 'unknown' : computerReady ? 'ready' : 'attention' },
          { key: 'browser-use' as const, icon: <Earth theme='outline' size={18} />, title: t('settings.capabilityPermissions.browser.title'), description: t('settings.capabilityPermissions.browser.summary'), state: 'onDemand' },
        ].map((item) => <VisualListRow key={item.key} icon={item.icon} title={item.title} description={item.description} onClick={() => setTab(item.key)}
          action={<VisualStatus tone={item.state === 'ready' ? 'success' : item.state === 'attention' ? 'warning' : 'neutral'}>
            {item.state === 'unknown' ? t('settings.capabilityPermissions.states.unknown') : t(`settings.capabilityPermissions.summaryStates.${item.state}`)}
          </VisualStatus>} />)}
      </VisualPanel>
      <VisualPanel title={t('settings.workspace.systemAccess')}>
        <PermissionRow kind='local_network' entry={systemPermissionEntry(status, 'local_network')}
          title={t('settings.capabilityPermissions.localNetwork.title')} description={t('settings.capabilityPermissions.localNetwork.description')}
          actionsEnabled={desktopShell} onOpenSettings={openSettings} />
        <PermissionRow kind='full_disk_access' entry={systemPermissionEntry(status, 'full_disk_access')}
          title={t('settings.capabilityPermissions.files.title')} description={t('settings.capabilityPermissions.files.description')}
          actionsEnabled={desktopShell} onOpenSettings={openSettings} />
        <PermissionRow kind='automation' entry={systemPermissionEntry(status, 'automation')}
          title={t('settings.capabilityPermissions.automation.title')} description={t('settings.capabilityPermissions.automation.description')}
          actionsEnabled={desktopShell} onOpenSettings={openSettings} />
      </VisualPanel>
    </div>
  );

  const voice = (
    <div className='space-y-12px'>
      <VisualPanel
        label={t('settings.capabilityPermissions.voice.title')}
      >
        <PermissionRow
          entry={microphone}
          kind='microphone'
          title={t('settings.capabilityPermissions.permissions.microphone')}
          description={t('settings.capabilityPermissions.permissions.microphoneDesc', { app: status?.app_label ?? 'NomiFun' })}
          busy={loading}
          actionsEnabled={desktopShell}
          onRequest={requestPermission}
          onOpenSettings={openSettings}
          extraAction={
            <><Button size='small' onClick={() => navigate('/models?section=asr')}>{t('settings.capabilityPermissions.voice.configureModel')}</Button>
            <Button size='small' loading={microphoneBusy} onClick={() => void testMicrophone()}>
              {t('settings.capabilityPermissions.voice.test')}
            </Button></>
          }
        />
      </VisualPanel>
      <Alert type='info' showIcon content={t('settings.capabilityPermissions.voice.privacy')} />
    </div>
  );

  const computer = (
    <div className='space-y-12px'>
      <ComputerDriverStatus />
      {status?.platform === 'macos' && <VisualPanel
        label={t('settings.capabilityPermissions.computer.title')}
      >
        <PermissionRow
          entry={accessibility}
          kind='accessibility'
          title={t('settings.capabilityPermissions.permissions.accessibility')}
          description={t('settings.capabilityPermissions.permissions.accessibilityDesc')}
          busy={loading}
          actionsEnabled={desktopShell}
          onRequest={requestPermission}
          onOpenSettings={openSettings}
        />
        <PermissionRow
          entry={screenRecording}
          kind='screen_recording'
          title={t('settings.capabilityPermissions.permissions.screenRecording')}
          description={t('settings.capabilityPermissions.permissions.screenRecordingDesc')}
          busy={loading}
          actionsEnabled={desktopShell}
          onRequest={requestPermission}
          onOpenSettings={openSettings}
        />
      </VisualPanel>}
      <Alert
        type={computerReady ? 'info' : 'warning'}
        showIcon
        content={t(status?.platform !== 'macos' ? 'settings.capabilityPermissions.computer.platformLimits' : computerReady
          ? 'settings.capabilityPermissions.computer.ready'
          : 'settings.capabilityPermissions.computer.blocked', { app: status?.app_label ?? 'NomiFun' })}
      />
    </div>
  );

  const browser = (
    <div className='space-y-12px'>
      <Alert type='success' showIcon content={t('settings.capabilityPermissions.browser.baseReady')} />
      <VisualPanel title={t('settings.capabilityPermissions.browser.hostMediaTitle')}
        description={t('settings.capabilityPermissions.browser.hostMediaDescription')}>
        {(['camera', 'microphone'] as const).map(kind => <PermissionRow key={kind} kind={kind}
          entry={systemPermissionEntry(status, kind)} title={t(`settings.capabilityPermissions.permissions.${kind}`)}
          description={t('settings.capabilityPermissions.browser.siteKinds.' + kind)}
          busy={loading} actionsEnabled={desktopShell} onRequest={requestPermission} onOpenSettings={openSettings} />)}
      </VisualPanel>
      <VisualPanel
        title={t('settings.capabilityPermissions.browser.websiteTitle')}
        description={t('settings.capabilityPermissions.browser.websiteDescription')}
      >
        {SYSTEM_PERMISSION_AUDIT.browser_use.requestedOnDemand
          .filter(kind => status?.platform !== 'macos' || kind === 'camera' || kind === 'microphone')
          .map((kind) => (
          <div key={kind} className='cv-row'>
            <div>
              <div className='flex items-center gap-8px text-14px text-t-primary'>
                {t(`settings.capabilityPermissions.permissions.${kind}`)}
                <VisualStatus tone='info'>{t('settings.capabilityPermissions.summaryStates.onDemand')}</VisualStatus>
              </div>
              <div className='mt-4px text-12px leading-19px text-t-tertiary'>
                {t(`settings.capabilityPermissions.browser.siteKinds.${kind}`)}
              </div>
            </div>
            {desktopShell && status?.platform === 'macos' && (
              <Button size='small' onClick={() => void openSettings(kind)}>
                {t('settings.capabilityPermissions.openSystemSettings')}
              </Button>
            )}
          </div>
        ))}
      </VisualPanel>
      {status?.platform === 'macos' && <Alert type='info' showIcon
        content={t('settings.capabilityPermissions.browser.macosPermissionLimit')} />}
      <VisualPanel title={t('settings.capabilityPermissions.localNetwork.title')}>
        <VisualRow label={t('settings.capabilityPermissions.summaryStates.onDemand')} description={t('settings.capabilityPermissions.localNetwork.browserDescription')}>
          {desktopShell && status?.platform === 'macos' && <Button size='small' onClick={() => void openSettings('local_network')}>{t('settings.capabilityPermissions.openSystemSettings')}</Button>}
        </VisualRow>
      </VisualPanel>
      <Alert type='info' showIcon content={t('settings.capabilityPermissions.browser.agentBoundary')} />
    </div>
  );

  if (notificationRedirect) return <Navigate replace to='/settings/system?section=preferences&focus=notifications' />;

  return (
    <div>
      <PageHeader title={t('settings.capabilityPermissions.title')}
        actions={<Button size='small' icon={<Refresh theme='outline' size={14} />} loading={loading} onClick={() => void refresh().catch(() => Message.error(t('settings.capabilityPermissions.loadFailed')))}>{t('settings.capabilityPermissions.refresh')}</Button>} />
      {!desktopShell && <Alert className='mb-16px' type='info' showIcon content={t('settings.capabilityPermissions.desktopOnly')} />}
      {error && <Alert className='mb-16px' type='error' showIcon content={t('settings.capabilityPermissions.loadFailed')} />}
      <VisualTabs id='permissions' label={t('settings.capabilityPermissions.title')} items={tabItems} activeKey={tab}
        onChange={(key) => setTab(isCapabilityTab(key) ? key : 'overview')} />
      <div id='permissions-panel' role='tabpanel' aria-labelledby={'permissions-tab-' + tab} aria-busy={loading}>
        {loading && !status ? <div className='flex min-h-180px items-center justify-center'><Spin /></div>
          : tab === 'voice-input' ? voice : tab === 'computer-use' ? computer : tab === 'browser-use' ? browser : overview}
      </div>
    </div>
  );
};

export default CapabilityPermissionsContent;
