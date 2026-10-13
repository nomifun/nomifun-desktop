import { systemPermissionNoticeState, type SystemPermissionNotice as Notice } from '../systemPermissionNoticeModel';
import { useSystemPermissions } from '@/renderer/hooks/system/useSystemPermissions';
import { Computer } from '@icon-park/react';
import React, { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import styles from './SystemPermissionNotice.module.css';

const SystemPermissionNotice: React.FC<{ notice: Notice }> = ({ notice }) => {
  const { t, i18n } = useTranslation();
  const navigate = useNavigate();
  const [dismissed, setDismissed] = useState(false);
  const [resolvedIdentity, setResolvedIdentity] = useState<string>();
  const resolved = resolvedIdentity === notice.refusal_id;
  const permissions = useSystemPermissions(!dismissed && !resolved, notice.refusal_id);
  const state = systemPermissionNoticeState(notice.request, permissions.status);
  const checkFailed = Boolean(permissions.error) || state.kind === 'check_failed';

  useEffect(() => {
    if (!permissions.loading && !permissions.error && state.kind === 'ready') setResolvedIdentity(notice.refusal_id);
  }, [notice.refusal_id, permissions.error, permissions.loading, state.kind]);

  // An OS grant is readiness for a new invocation, never authority to replay
  // the old physical input or to rewrite its not_executed receipt.
  if (dismissed || resolved || (!permissions.loading && !checkFailed && state.kind === 'ready')) return null;

  const names = state.missing.map(kind => t(kind === 'accessibility'
    ? 'messages.systemPermission.accessibility' : 'messages.systemPermission.screenRecording'));
  const permissionList = new Intl.ListFormat(i18n.resolvedLanguage ?? i18n.language,
    { style: 'short', type: 'conjunction' }).format(names);

  return <div className={styles.notice} data-testid='system-permission-notice'>
    <p className={styles.context}>{t('messages.systemPermission.context')}</p>
    <div className={styles.card} role='status' aria-live='polite' aria-label={t('messages.systemPermission.title')}>
      <span className={styles.icon} aria-hidden='true'><Computer size={22} fill='currentColor' /></span>
      <div className={styles.copy}>
        <div className={styles.title}>{t(checkFailed ? 'messages.systemPermission.checkFailedTitle' : 'messages.systemPermission.title')}</div>
        <div className={styles.detail}>{t(checkFailed ? 'messages.systemPermission.checkFailed'
          : 'messages.systemPermission.missing', { permissions: permissionList })}</div>
      </div>
      <div className={styles.actions}>
        {checkFailed ? <button type='button' className={styles.primary} disabled={permissions.loading}
          onClick={() => void permissions.refresh()}>{t('messages.systemPermission.retryCheck')}</button>
          : <button type='button' className={styles.primary}
            onClick={() => { void navigate('/settings/permissions?tab=computer-use'); }}>{t('messages.systemPermission.grant')}</button>}
        <button type='button' className={styles.later} onClick={() => setDismissed(true)}>{t('messages.systemPermission.later')}</button>
      </div>
    </div>
  </div>;
};

export default SystemPermissionNotice;
