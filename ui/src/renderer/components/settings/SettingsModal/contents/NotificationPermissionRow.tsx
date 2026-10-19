import type { NotificationPermission } from '@/renderer/hooks/system/useNotificationPermission';
import { VisualRow } from '@/renderer/pages/settings/components/CodeVisualPrimitives';
import { Button } from '@arco-design/web-react';
import React from 'react';
import { useTranslation } from 'react-i18next';

/** A standard settings row; notification preferences remain separate controls. */
const NotificationPermissionRow: React.FC<{ permission: NotificationPermission }> = ({ permission }) => {
  const { t } = useTranslation();
  const { state, error, desktopShell, canOpenSettings, loading, busy } = permission;
  const status = !desktopShell || error || state === 'unknown' ? 'unavailable'
    : state === 'granted' || state === 'not_required' ? 'enabled'
    : state === 'not_determined' ? 'pending' : 'denied';
  return <VisualRow label={t(`settings.notificationPermission.${status}Title`)}
    description={!desktopShell ? t('settings.capabilityPermissions.desktopOnly')
      : error ? t('settings.capabilityPermissions.loadFailed')
      : t(`settings.notificationPermission.${status}Desc`)}>
    {desktopShell && (error || state === 'unknown'
      ? <Button size='small' loading={loading} onClick={() => void permission.refresh()}>{t('settings.workspace.retry')}</Button>
      : state === 'not_determined'
        ? <Button size='small' loading={busy} onClick={() => void permission.request()}>{t('settings.capabilityPermissions.requestAccess')}</Button>
        : canOpenSettings && <Button size='small' onClick={() => void permission.openSettings()}>{t('settings.capabilityPermissions.openSystemSettings')}</Button>)}
  </VisualRow>;
};

export default NotificationPermissionRow;
