/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ConversationId } from '@/common/types/ids';
import type { AgentResolvedSnapshot } from '@/common/types/agentPlatform';
import {
  computerPermissionKindsForActions,
  missingComputerPermissionKinds,
} from '@/renderer/hooks/system/systemPermissionModel';
import { useSystemPermissions } from '@/renderer/hooks/system/useSystemPermissions';
import { Button, Popover } from '@arco-design/web-react';
import { Attention } from '@icon-park/react';
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import {
  capabilityHeaderButtonClass,
  capabilityHeaderButtonStyle,
} from './CapabilityHeaderButton';

type Props = {
  conversationId: ConversationId;
  snapshot?: AgentResolvedSnapshot | null;
};

const WARNING_COLOR = 'rgb(var(--orange-6))';

/**
 * A non-modal conversation reminder for frozen Agent capabilities whose live
 * host permission has been revoked or was never granted. The chip remains in
 * the header after the auto-open bubble is dismissed, and the shared permission
 * hook rechecks when the user returns from System Settings.
 */
const SystemPermissionReminder: React.FC<Props> = ({ conversationId, snapshot }) => {
  const { t, i18n } = useTranslation();
  const navigate = useNavigate();
  const computerEnabled = snapshot?.enabled_capabilities.includes('computer') === true;
  const computerActions = useMemo(
    () => snapshot?.enabled_capability_actions.computer ?? [],
    [snapshot?.enabled_capability_actions.computer]
  );
  const requiredKinds = useMemo(
    () => computerEnabled ? computerPermissionKindsForActions(computerActions) : [],
    [computerActions, computerEnabled]
  );
  const shouldCheck = requiredKinds.length > 0;
  const permissions = useSystemPermissions(shouldCheck);
  const missingKinds = useMemo(
    () => shouldCheck && permissions.status
      ? missingComputerPermissionKinds(permissions.status, computerActions)
      : [],
    [computerActions, permissions.status, shouldCheck]
  );
  const issue = !permissions.loading && shouldCheck
    ? permissions.error
      ? 'check_failed'
      : missingKinds.length > 0
        ? 'missing'
        : null
    : null;
  const issueSignature = issue === 'missing'
    ? `${conversationId}:${missingKinds.join(',')}`
    : issue === 'check_failed'
      ? `${conversationId}:check_failed`
      : '';
  const shownSignature = useRef('');
  const [bubbleVisible, setBubbleVisible] = useState(false);

  useEffect(() => {
    // A focus-triggered refresh is not a resolved state change. Preserve the
    // dismissal signature so the same bubble does not reopen every time the
    // user switches back to this window.
    if (permissions.loading) return;
    if (!issueSignature) {
      shownSignature.current = '';
      setBubbleVisible(false);
      return;
    }
    if (shownSignature.current !== issueSignature) {
      shownSignature.current = issueSignature;
      setBubbleVisible(true);
    }
  }, [issueSignature, permissions.loading]);

  if (!issue) return null;

  const permissionNames = missingKinds.map((kind) => t(
    kind === 'accessibility'
      ? 'settings.capabilityPermissions.permissions.accessibility'
      : 'settings.capabilityPermissions.permissions.screenRecording'
  ));
  const permissionList = new Intl.ListFormat(i18n.resolvedLanguage ?? i18n.language, {
    style: 'short',
    type: 'conjunction',
  }).format(permissionNames);
  const panel = (
    <div
      className='flex max-w-320px flex-col gap-8px'
      role='dialog'
      aria-label={t('conversation.systemPermissions.title')}
    >
      <div className='flex items-center gap-7px text-13px font-600 text-t-primary'>
        <Attention theme='outline' size='16' fill={WARNING_COLOR} />
        {t('conversation.systemPermissions.title')}
      </div>
      <div className='text-12px leading-19px text-t-secondary'>
        {issue === 'missing'
          ? t('conversation.systemPermissions.missing', { permissions: permissionList })
          : t('conversation.systemPermissions.checkFailed')}
      </div>
      <div className='flex items-center justify-end gap-8px pt-2px'>
        <Button size='mini' type='text' onClick={() => setBubbleVisible(false)}>
          {t('conversation.systemPermissions.later')}
        </Button>
        <Button
          size='mini'
          type='primary'
          onClick={() => {
            setBubbleVisible(false);
            void navigate('/settings/permissions?tab=computer-use');
          }}
        >
          {t('conversation.systemPermissions.openSettings')}
        </Button>
      </div>
    </div>
  );

  return (
    <Popover
      trigger='click'
      position='br'
      content={panel}
      popupVisible={bubbleVisible}
      onVisibleChange={setBubbleVisible}
    >
      <Button
        size='mini'
        shape='round'
        type='secondary'
        data-testid='system-permission-reminder'
        aria-label={t('conversation.systemPermissions.label')}
        aria-haspopup='dialog'
        aria-expanded={bubbleVisible}
        className={capabilityHeaderButtonClass(true, 'shrink-0')}
        style={capabilityHeaderButtonStyle(WARNING_COLOR)}
      >
        <span className='inline-flex items-center gap-6px leading-none'>
          <Attention theme='outline' size='14' fill={WARNING_COLOR} />
          <span className='text-12px'>{t('conversation.systemPermissions.label')}</span>
        </span>
      </Button>
    </Popover>
  );
};

export default SystemPermissionReminder;
