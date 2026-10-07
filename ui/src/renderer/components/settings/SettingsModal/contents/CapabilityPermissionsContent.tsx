/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type {
  SystemPermissionEntry,
  SystemPermissionKind,
  SystemPermissionState,
} from '@/common/adapter/ipcBridge';
import NomiScrollArea from '@/renderer/components/base/NomiScrollArea';
import SegmentedTabs, { type SegmentedTabItem } from '@/renderer/components/base/SegmentedTabs';
import {
  SYSTEM_PERMISSION_AUDIT,
  computerPermissionsReady,
  permissionEntryIsReady,
  systemPermissionEntry,
  type PermissionCapabilityTab,
} from '@/renderer/hooks/system/systemPermissionModel';
import { useSystemPermissions } from '@/renderer/hooks/system/useSystemPermissions';
import { isDesktopShell } from '@/renderer/utils/platform';
import { Alert, Button, Message, Spin, Tag } from '@arco-design/web-react';
import {
  Computer,
  Earth,
  FolderOpen,
  HeadsetOne,
  Refresh,
  Remind,
  Shield,
} from '@icon-park/react';
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate, useSearchParams } from 'react-router-dom';

const isCapabilityTab = (value: string | null): value is PermissionCapabilityTab =>
  value === 'overview' ||
  value === 'voice-input' ||
  value === 'computer-use' ||
  value === 'browser-use' ||
  value === 'notifications';

const readyState = (state: SystemPermissionState): boolean =>
  state === 'granted' || state === 'not_required';

const statusColor = (state: SystemPermissionState): 'green' | 'red' | 'orange' | 'gray' => {
  if (readyState(state)) return 'green';
  if (state === 'denied' || state === 'restricted') return 'red';
  if (state === 'not_determined') return 'orange';
  return 'gray';
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
  return (
    <div className='flex min-h-68px items-center justify-between gap-24px py-12px'>
      <div className='min-w-0 flex-1'>
        <div className='flex items-center gap-8px'>
          <span className='text-14px text-t-primary'>{title}</span>
          <Tag size='small' color={statusColor(state)}>
            {t(`settings.capabilityPermissions.states.${state}`)}
          </Tag>
        </div>
        <div className='mt-4px text-12px leading-19px text-t-tertiary'>{description}</div>
        {entry?.requires_restart_after_grant && !readyState(state) && (
          <div className='mt-4px text-12px leading-19px text-warning-6'>
            {t('settings.capabilityPermissions.restartAfterGrant')}
          </div>
        )}
      </div>
      <div className='flex shrink-0 items-center gap-8px'>
        {extraAction}
        {actionsEnabled && entry?.can_request && state !== 'denied' && state !== 'restricted' && (
          <Button size='small' type='primary' loading={busy} onClick={() => onRequest?.(kind)}>
            {t('settings.capabilityPermissions.requestAccess')}
          </Button>
        )}
        {actionsEnabled && entry?.can_open_settings && !readyState(state) && (
          <Button size='small' onClick={() => onOpenSettings?.(kind)}>
            {t('settings.capabilityPermissions.openSystemSettings')}
          </Button>
        )}
      </div>
    </div>
  );
};

const SettingsCard: React.FC<{
  title: string;
  description?: string;
  children: React.ReactNode;
  action?: React.ReactNode;
}> = ({ title, description, children, action }) => (
  <section className='rounded-16px bg-2 px-24px py-18px'>
    <div className='flex items-start justify-between gap-24px'>
      <div>
        <h2 className='m-0 text-15px font-600 text-t-primary'>{title}</h2>
        {description && <p className='m-0 mt-5px text-12px leading-19px text-t-secondary'>{description}</p>}
      </div>
      {action}
    </div>
    <div className='mt-12px divide-y divide-x-0 divide-solid divide-[var(--color-border-2)]'>
      {children}
    </div>
  </section>
);

const CapabilityPermissionsContent: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [searchParams, setSearchParams] = useSearchParams();
  const desktopShell = isDesktopShell();
  const tab = isCapabilityTab(searchParams.get('tab')) ? searchParams.get('tab') as PermissionCapabilityTab : 'overview';
  const { error, loading, openSettings, refresh, request, status } = useSystemPermissions(true);
  const [notificationState, setNotificationState] = useState<SystemPermissionState>('unknown');
  const [notificationBusy, setNotificationBusy] = useState(false);
  const [microphoneBusy, setMicrophoneBusy] = useState(false);

  const microphone = systemPermissionEntry(status, 'microphone');
  const accessibility = systemPermissionEntry(status, 'accessibility');
  const screenRecording = systemPermissionEntry(status, 'screen_recording');
  const computerReady = computerPermissionsReady(status, []);
  const voiceReady = permissionEntryIsReady(microphone);

  const refreshNotification = useCallback(async () => {
    if (!desktopShell) {
      setNotificationState('unknown');
      return;
    }
    const state = await ipcBridge.notification.permissionState.invoke();
    setNotificationState(
      state === 'granted'
        ? 'granted'
        : state === 'denied'
          ? 'denied'
          : state === 'default'
            ? 'not_determined'
            : 'unknown'
    );
  }, [desktopShell]);

  useEffect(() => {
    void refreshNotification();
    const onFocus = () => void refreshNotification();
    window.addEventListener('focus', onFocus);
    return () => window.removeEventListener('focus', onFocus);
  }, [refreshNotification]);

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

  const requestNotifications = useCallback(async () => {
    setNotificationBusy(true);
    try {
      const result = await ipcBridge.notification.requestPermission.invoke();
      setNotificationState(
        result === 'granted'
          ? 'granted'
          : result === 'denied'
            ? 'denied'
            : result === 'default'
              ? 'not_determined'
              : 'unknown'
      );
    } finally {
      setNotificationBusy(false);
    }
  }, []);

  const tabItems: SegmentedTabItem[] = useMemo(() => [
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
    {
      key: 'notifications',
      label: t('settings.capabilityPermissions.tabs.notifications'),
      icon: <Remind theme='outline' size='15' strokeWidth={3} />,
      dot: notificationState === 'denied' || notificationState === 'restricted',
    },
  ], [computerReady, notificationState, status, t, voiceReady]);

  const notificationEntry: Pick<SystemPermissionEntry, 'state' | 'can_request' | 'can_open_settings' | 'requires_restart_after_grant'> = {
    state: notificationState,
    can_request: desktopShell && notificationState === 'not_determined',
    can_open_settings: desktopShell && status?.platform === 'macos',
    requires_restart_after_grant: false,
  };

  const overview = (
    <div className='grid grid-cols-2 gap-12px'>
      {[
        {
          key: 'voice-input' as const,
          icon: <HeadsetOne theme='outline' size='20' />,
          title: t('settings.capabilityPermissions.voice.title'),
          description: t('settings.capabilityPermissions.voice.summary'),
          state: voiceReady ? 'ready' : 'attention',
        },
        {
          key: 'computer-use' as const,
          icon: <Computer theme='outline' size='20' />,
          title: t('settings.capabilityPermissions.computer.title'),
          description: t('settings.capabilityPermissions.computer.summary'),
          state: computerReady ? 'ready' : 'attention',
        },
        {
          key: 'browser-use' as const,
          icon: <Earth theme='outline' size='20' />,
          title: t('settings.capabilityPermissions.browser.title'),
          description: t('settings.capabilityPermissions.browser.summary'),
          state: 'onDemand',
        },
        {
          key: 'notifications' as const,
          icon: <Remind theme='outline' size='20' />,
          title: t('settings.capabilityPermissions.notifications.title'),
          description: t('settings.capabilityPermissions.notifications.summary'),
          state: notificationState === 'granted' ? 'ready' : 'attention',
        },
      ].map((item) => (
        <button
          key={item.key}
          type='button'
          className='flex min-h-116px cursor-pointer items-start gap-12px rounded-16px border border-solid border-[var(--color-border-2)] bg-2 p-16px text-left transition-colors hover:bg-fill-1'
          onClick={() => setTab(item.key)}
        >
          <span className='flex size-38px shrink-0 items-center justify-center rounded-12px bg-primary-1 text-primary-6'>
            {item.icon}
          </span>
          <span className='min-w-0 flex-1'>
            <span className='flex items-center justify-between gap-8px'>
              <strong className='text-14px text-t-primary'>{item.title}</strong>
              <Tag size='small' color={item.state === 'ready' ? 'green' : item.state === 'onDemand' ? 'blue' : 'orange'}>
                {t(`settings.capabilityPermissions.summaryStates.${item.state}`)}
              </Tag>
            </span>
            <span className='mt-6px block text-12px leading-19px text-t-secondary'>{item.description}</span>
          </span>
        </button>
      ))}
      <div className='col-span-2 flex items-start gap-12px rounded-16px border border-solid border-[var(--color-border-2)] bg-2 p-16px'>
        <span className='flex size-38px shrink-0 items-center justify-center rounded-12px bg-fill-2 text-t-secondary'>
          <Earth theme='outline' size='20' />
        </span>
        <div>
          <div className='text-14px font-600 text-t-primary'>{t('settings.capabilityPermissions.localNetwork.title')}</div>
          <div className='mt-5px text-12px leading-19px text-t-secondary'>{t('settings.capabilityPermissions.localNetwork.description')}</div>
        </div>
        {desktopShell && status?.platform === 'macos' && (
          <Button className='ml-auto shrink-0' size='small' onClick={() => void openSettings('local_network')}>
            {t('settings.capabilityPermissions.openSystemSettings')}
          </Button>
        )}
      </div>
      <div className='col-span-2 flex items-start gap-12px rounded-16px border border-solid border-[var(--color-border-2)] bg-2 p-16px'>
        <span className='flex size-38px shrink-0 items-center justify-center rounded-12px bg-fill-2 text-t-secondary'>
          <FolderOpen theme='outline' size='20' />
        </span>
        <div>
          <div className='text-14px font-600 text-t-primary'>{t('settings.capabilityPermissions.files.title')}</div>
          <div className='mt-5px text-12px leading-19px text-t-secondary'>{t('settings.capabilityPermissions.files.description')}</div>
        </div>
        {desktopShell && status?.platform === 'macos' && (
          <Button className='ml-auto shrink-0' size='small' onClick={() => void openSettings('full_disk_access')}>
            {t('settings.capabilityPermissions.openSystemSettings')}
          </Button>
        )}
      </div>
    </div>
  );

  const voice = (
    <div className='space-y-12px'>
      <SettingsCard
        title={t('settings.capabilityPermissions.voice.title')}
        description={t('settings.capabilityPermissions.voice.description')}
        action={<Button size='small' onClick={() => navigate('/models?section=asr')}>{t('settings.capabilityPermissions.voice.configureModel')}</Button>}
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
            <Button size='small' loading={microphoneBusy} onClick={() => void testMicrophone()}>
              {t('settings.capabilityPermissions.voice.test')}
            </Button>
          }
        />
      </SettingsCard>
      <Alert type='info' showIcon content={t('settings.capabilityPermissions.voice.privacy')} />
    </div>
  );

  const computer = (
    <div className='space-y-12px'>
      <SettingsCard
        title={t('settings.capabilityPermissions.computer.title')}
        description={t('settings.capabilityPermissions.computer.description')}
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
      </SettingsCard>
      <Alert
        type={computerReady ? 'success' : 'warning'}
        showIcon
        content={t(computerReady
          ? 'settings.capabilityPermissions.computer.ready'
          : 'settings.capabilityPermissions.computer.blocked', { app: status?.app_label ?? 'NomiFun' })}
      />
    </div>
  );

  const browser = (
    <div className='space-y-12px'>
      <Alert type='success' showIcon content={t('settings.capabilityPermissions.browser.baseReady')} />
      <SettingsCard
        title={t('settings.capabilityPermissions.browser.websiteTitle')}
        description={t('settings.capabilityPermissions.browser.websiteDescription')}
      >
        {SYSTEM_PERMISSION_AUDIT.browser_use.requestedOnDemand.map((kind) => (
          <div key={kind} className='flex min-h-62px items-center justify-between gap-24px py-12px'>
            <div>
              <div className='flex items-center gap-8px text-14px text-t-primary'>
                {t(`settings.capabilityPermissions.permissions.${kind}`)}
                <Tag size='small' color='blue'>{t('settings.capabilityPermissions.summaryStates.onDemand')}</Tag>
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
      </SettingsCard>
      <section className='rounded-16px bg-2 px-24px py-18px'>
        <div className='flex items-center justify-between gap-24px'>
          <div>
            <div className='flex items-center gap-8px'>
              <h2 className='m-0 text-15px font-600 text-t-primary'>{t('settings.capabilityPermissions.localNetwork.title')}</h2>
              <Tag size='small' color='blue'>{t('settings.capabilityPermissions.summaryStates.onDemand')}</Tag>
            </div>
            <p className='m-0 mt-5px text-12px leading-19px text-t-secondary'>
              {t('settings.capabilityPermissions.localNetwork.browserDescription')}
            </p>
          </div>
          {desktopShell && status?.platform === 'macos' && (
            <Button className='shrink-0' size='small' onClick={() => void openSettings('local_network')}>
              {t('settings.capabilityPermissions.openSystemSettings')}
            </Button>
          )}
        </div>
      </section>
      <Alert type='info' showIcon content={t('settings.capabilityPermissions.browser.agentBoundary')} />
    </div>
  );

  const notifications = (
    <div className='space-y-12px'>
      <SettingsCard
        title={t('settings.capabilityPermissions.notifications.title')}
        description={t('settings.capabilityPermissions.notifications.description')}
      >
        <PermissionRow
          entry={notificationEntry}
          kind='notifications'
          title={t('settings.capabilityPermissions.permissions.notifications')}
          description={t('settings.capabilityPermissions.permissions.notificationsDesc')}
          busy={notificationBusy}
          actionsEnabled={desktopShell}
          onRequest={() => void requestNotifications()}
          onOpenSettings={openSettings}
        />
      </SettingsCard>
      <Alert type='info' showIcon content={t('settings.capabilityPermissions.notifications.preferenceHint')} />
    </div>
  );

  return (
    <div className='flex h-full w-full flex-col'>
      <div className='mb-16px flex items-start justify-between gap-24px'>
        <div>
          <h1 className='m-0 text-20px font-650 text-t-primary'>{t('settings.capabilityPermissions.title')}</h1>
          <p className='m-0 mt-6px max-w-720px text-13px leading-20px text-t-secondary'>
            {t('settings.capabilityPermissions.subtitle')}
          </p>
        </div>
        <Button size='small' icon={<Refresh theme='outline' size='14' />} loading={loading} onClick={() => void Promise.all([refresh(), refreshNotification()])}>
          {t('settings.capabilityPermissions.refresh')}
        </Button>
      </div>

      {!desktopShell && (
        <Alert className='mb-12px' type='info' showIcon content={t('settings.capabilityPermissions.desktopOnly')} />
      )}
      {error && <Alert className='mb-12px' type='error' showIcon content={t('settings.capabilityPermissions.loadFailed')} />}

      <SegmentedTabs
        items={tabItems}
        activeKey={tab}
        onChange={(key) => setTab(isCapabilityTab(key) ? key : 'overview')}
        size='sm'
        className='mb-16px'
      />

      <NomiScrollArea className='min-h-0 flex-1 pb-16px' disableOverflow>
        {loading && !status ? (
          <div className='flex min-h-180px items-center justify-center'><Spin /></div>
        ) : tab === 'voice-input' ? voice
          : tab === 'computer-use' ? computer
            : tab === 'browser-use' ? browser
              : tab === 'notifications' ? notifications
                : overview}
      </NomiScrollArea>
    </div>
  );
};

export default CapabilityPermissionsContent;
