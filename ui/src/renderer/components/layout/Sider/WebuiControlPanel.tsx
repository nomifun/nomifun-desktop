/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { WEBUI_DEFAULT_PORT } from '@/common/config/constants';
import { webui } from '@/common/adapter/ipcBridge';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import NomiModal from '@/renderer/components/base/NomiModal';
import { fetchCurrentUser } from '@/renderer/hooks/context/AuthContext';
import { useWebuiServer } from '@/renderer/hooks/context/WebuiServerContext';
import { copyText } from '@/renderer/utils/ui/clipboard';
import { Alert, Button, Form, Input, Message, Switch, Tooltip } from '@arco-design/web-react';
import { Copy, Earth, EditTwo, PreviewCloseOne, PreviewOpen } from '@icon-park/react';
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { buildWebuiQrLoginUrl, getWebuiQrBaseUrls } from './webuiQrLinks';
import WebuiAccessAddresses from './WebuiAccessAddresses';
import ConnectionHint from './ConnectionHint';
import WebuiQrLoginPanel from './WebuiQrLoginPanel';

/**
 * WebuiControlPanel — the full WebUI remote-access control body, shared by the
 * Open Capabilities page and the compact sider footer popover.
 *
 * Consumes shared listener state and adds account dialogs and one-time login.
 * Browser sessions read their authenticated account separately from the
 * desktop listener's status.
 */
interface WebuiControlPanelProps {
  mode?: 'popover' | 'page';
}

const CREDENTIAL_PILL_CLASS =
  'inline-flex min-w-0 items-center rd-100px border border-solid border-arco-2 bg-fill-1 px-8px py-2px';
const CREDENTIAL_ACTIONS_CLASS = 'ml-6px inline-flex shrink-0 items-center gap-0';
const CREDENTIAL_ICON_BUTTON_CLASS =
  'rd-100px !h-20px !w-20px !min-w-20px !p-0 inline-flex items-center justify-center';

const WebuiControlPanel: React.FC<WebuiControlPanelProps> = ({ mode = 'popover' }) => {
  const { t } = useTranslation();

  // Shared WebUI server state (single subscription, single start/stop path).
  const { status, running, starting, lifecycleSupported, initialPassword, accessUrls, start, stop, refresh, clearInitialPassword } = useWebuiServer();
  const port = WEBUI_DEFAULT_PORT;
  const availablePassword = lifecycleSupported ? initialPassword : null;
  const [passwordVisible, setPasswordVisible] = useState(false);

  useEffect(() => setPasswordVisible(false), [availablePassword]);

  // 用户名即时覆盖（改密后调 clearInitialPassword 收起明文）。
  // Optimistic username override for immediate display after a change.
  const [usernameOverride, setUsernameOverride] = useState<string | null>(null);
  const [browserUsername, setBrowserUsername] = useState<string | null>(null);
  const [browserUsernameLoading, setBrowserUsernameLoading] = useState(!lifecycleSupported);

  useEffect(() => {
    if (lifecycleSupported) return;
    const controller = new AbortController();
    setBrowserUsernameLoading(true);
    void fetchCurrentUser(controller.signal).then((result) => {
      if (controller.signal.aborted) return;
      setBrowserUsername(result.kind === 'user' ? result.user.username : null);
      setBrowserUsernameLoading(false);
    });
    return () => controller.abort();
  }, [lifecycleSupported]);

  // 设置新密码 / 用户名弹窗 / Password & username modals
  const [setPasswordModalVisible, setSetPasswordModalVisible] = useState(false);
  const [passwordLoading, setPasswordLoading] = useState(false);
  const [setUsernameModalVisible, setSetUsernameModalVisible] = useState(false);
  const [usernameLoading, setUsernameLoading] = useState(false);
  const [form] = Form.useForm();
  const [usernameForm] = Form.useForm();

  // 二维码登录相关状态 / QR code login related state
  const [qrToken, setQrToken] = useState<string | null>(null);
  const [qrExpiresAt, setQrExpiresAt] = useState<number | null>(null);
  const [qrLoading, setQrLoading] = useState(false);
  const [selectedQrBaseUrl, setSelectedQrBaseUrl] = useState<string | null>(null);
  const qrRefreshTimerRef = useRef<NodeJS.Timeout | null>(null);

  // 复制内容 / Copy content
  const handleCopy = (text: string) => {
    copyText(text)
      .then(() => Message.success(t('common.copySuccess')))
      .catch(() => Message.error(t('common.copyFailed')));
  };

  // Browser lifecycle status is unavailable; it cannot establish an account.
  const displayUsername = usernameOverride || (lifecycleSupported ? status?.adminUsername : browserUsername) || null;
  const usernamePending = lifecycleSupported ? !status : browserUsernameLoading;
  const qrBaseUrls = useMemo(() => getWebuiQrBaseUrls(status, accessUrls, port), [status, accessUrls, port]);
  const qrUrl = useMemo(() => {
    if (!qrToken || !selectedQrBaseUrl) return null;
    return buildWebuiQrLoginUrl(selectedQrBaseUrl, qrToken);
  }, [qrToken, selectedQrBaseUrl]);
  const selectedQrDisplayUrl = selectedQrBaseUrl ?? qrBaseUrls[0] ?? '';

  // 打开设置新密码弹窗 / Open set new password modal
  const handleResetPassword = () => {
    form.resetFields();
    setSetPasswordModalVisible(true);
  };

  const handleResetUsername = () => {
    if (!displayUsername) return;
    usernameForm.setFieldsValue({ newUsername: displayUsername });
    setSetUsernameModalVisible(true);
  };

  // 提交新密码 / Submit new password
  //
  // Desktop shell: possession-based `/api/webui/change-password` (no current
  // password). Browser (docker WebUI): authenticated `/api/auth/change-password`
  // with current-password verification; the backend rotates the JWT secret, so
  // every session dies and we route the user back to the login screen.
  const handleSetNewPassword = async () => {
    try {
      const values = await form.validate();
      setPasswordLoading(true);

      if (lifecycleSupported) {
        await webui.changePassword.invoke({
          newPassword: values.newPassword,
        });
      } else {
        await webui.account.changePassword.invoke({
          currentPassword: values.currentPassword,
          newPassword: values.newPassword,
        });
      }
      setSetPasswordModalVisible(false);
      form.resetFields();
      // 改密后清除一次性明文（标题栏抽屉与本页同时收起）。
      // After a change, forget the one-time plaintext (hides it everywhere).
      clearInitialPassword();
      if (lifecycleSupported) {
        void refresh();
        Message.success(t('settings.webui.passwordChanged'));
      } else {
        // All sessions (this one included) are now invalid — send the user to
        // sign back in with the new password instead of letting the next API
        // call bounce them with an opaque auth error.
        Message.success(t('settings.webui.passwordChangedRelogin'));
        setTimeout(() => {
          window.location.hash = '/login';
        }, 1200);
      }
    } catch (error) {
      console.error('Set new password error:', error);
      const errorCodeMap: Record<string, string> = {
        PASSWORD_TOO_SHORT: t('settings.webui.passwordTooShort'),
        PASSWORD_TOO_LONG: t('settings.webui.passwordTooLong'),
        PASSWORD_TOO_COMMON: t('settings.webui.passwordTooCommon'),
      };
      if (isBackendHttpError(error) && error.status === 401) {
        Message.error(t('settings.webui.currentPasswordIncorrect'));
        return;
      }
      const rawMsg =
        isBackendHttpError(error) && error.backendMessage
          ? error.backendMessage
          : error instanceof Error
            ? error.message
            : '';
      const codes = rawMsg.split('; ');
      const translated = codes.map((code) => errorCodeMap[code]).filter(Boolean);
      Message.error(translated.length > 0 ? translated.join('; ') : rawMsg || t('settings.webui.passwordChangeFailed'));
    } finally {
      setPasswordLoading(false);
    }
  };

  const handleSetNewUsername = async () => {
    try {
      const values = await usernameForm.validate();
      setUsernameLoading(true);

      // HTTP bridge: both variants return { username: string } directly;
      // httpBridge throws BackendHttpError on 4xx/5xx — caught below.
      const result = lifecycleSupported
        ? await webui.changeUsername.invoke({
            newUsername: values.newUsername,
          })
        : await webui.account.changeUsername.invoke({
            currentPassword: values.currentPassword,
            newUsername: values.newUsername,
          });
      const nextUsername = result?.username ?? values.newUsername.trim();
      Message.success(t('settings.webui.usernameChanged'));
      setSetUsernameModalVisible(false);
      usernameForm.resetFields();
      setUsernameOverride(nextUsername);
    } catch (error) {
      console.error('Set new username error:', error);
      if (isBackendHttpError(error) && error.status === 401) {
        Message.error(t('settings.webui.currentPasswordIncorrect'));
        return;
      }
      const fallback = t('settings.webui.usernameChangeFailed');
      const msg = isBackendHttpError(error) && error.backendMessage ? error.backendMessage : fallback;
      Message.error(msg);
    } finally {
      setUsernameLoading(false);
    }
  };

  // 生成二维码 / Generate QR code
  const generateQRCode = useCallback(async () => {
    if (!status?.running) return;

    setQrLoading(true);
    try {
      // Backend returns only { token, expires_at_ms }; the scannable URL is
      // composed here from the selected access URL so multi-homed/VPN hosts can
      // switch the QR code to whichever LAN address a phone can actually reach.
      const qrData = await webui.generateQRToken.invoke();

      if (qrData) {
        setQrToken(qrData.token);
        setSelectedQrBaseUrl((current) => (current && qrBaseUrls.includes(current) ? current : (qrBaseUrls[0] ?? null)));
        setQrExpiresAt(qrData.expires_at_ms);

        // 设置自动刷新定时器（4分钟后自动刷新，因为 token 5分钟过期）
        // Set auto-refresh timer (refresh after 4 minutes, as token expires in 5 minutes)
        if (qrRefreshTimerRef.current) {
          clearTimeout(qrRefreshTimerRef.current);
        }
        qrRefreshTimerRef.current = setTimeout(
          () => {
            void generateQRCode();
          },
          4 * 60 * 1000
        );
      } else {
        console.error('Generate QR code failed: no data returned');
        Message.error(t('settings.webui.qrGenerateFailed'));
      }
    } catch (error) {
      console.error('Generate QR code error:', error);
      Message.error(t('settings.webui.qrGenerateFailed'));
    } finally {
      setQrLoading(false);
    }
  }, [status?.running, qrBaseUrls, t]);

  useEffect(() => {
    if (qrBaseUrls.length === 0) {
      setSelectedQrBaseUrl(null);
      return;
    }
    setSelectedQrBaseUrl((current) => (current && qrBaseUrls.includes(current) ? current : qrBaseUrls[0]));
  }, [qrBaseUrls]);

  // 当服务器启动且允许远程访问时自动生成二维码 / Auto-generate QR code when server starts and remote access is allowed
  useEffect(() => {
    if (status?.running && status.allowRemote && !qrToken) {
      void generateQRCode();
    }
  }, [status?.allowRemote, status?.running, generateQRCode, qrToken]);

  // 清理定时器 / Cleanup timer
  useEffect(() => {
    return () => {
      if (qrRefreshTimerRef.current) {
        clearTimeout(qrRefreshTimerRef.current);
      }
    };
  }, []);

  // 服务器停止或关闭远程访问时清除二维码 / Clear QR code when server stops or remote access is disabled
  useEffect(() => {
    if (!status?.running || !status.allowRemote) {
      setQrToken(null);
      setQrExpiresAt(null);
      setSelectedQrBaseUrl(null);
      if (qrRefreshTimerRef.current) {
        clearTimeout(qrRefreshTimerRef.current);
        qrRefreshTimerRef.current = null;
      }
    }
  }, [status?.allowRemote, status?.running]);

  // Only the native first-enable result supplies a password that can be revealed.
  // Persisted credentials are hashes; show their configured state truthfully.
  const passwordIsSet = status?.passwordSet ?? true;
  const displayPassword = availablePassword
    ? passwordVisible ? availablePassword : t('settings.webui.passwordHidden')
    : passwordIsSet ? t('settings.webui.passwordConfigured') : t('settings.webui.passwordNotSet');
  const containerClass =
    mode === 'page'
      ? 'w-full overflow-visible flex flex-col gap-12px'
      : 'w-320px max-h-[min(560px,70vh)] overflow-y-auto -mx-4px px-4px py-4px flex flex-col gap-12px';

  return (
    <div className={containerClass}>
      {mode === 'popover' && <>
        <div className='flex items-center gap-8px px-2px'>
          <Earth theme='outline' size='16' className='text-primary-6 shrink-0' />
          <span className='text-14px font-500 text-t-primary'>{t('settings.webui')}</span>
        </div>
        <div className='rd-10px border border-solid border-arco-2 bg-fill-1 px-10px py-8px text-12px text-t-secondary leading-relaxed'>
          {t('settings.webui.featureRemoteDesc')}
        </div>
      </>}

      {/* A browser is already served by its host and cannot control its listener. */}
      {!lifecycleSupported && (
        <div className='rd-10px border border-solid border-arco-2 bg-fill-1 px-10px py-8px text-12px text-warning leading-relaxed'>
          {t('settings.webui.desktopLifecycleUnavailable')}
        </div>
      )}

      {/* 启用 WebUI / Enable WebUI */}
      {lifecycleSupported && <div className='flex items-center justify-between gap-12px'>
        <div className='min-w-0 flex items-center gap-8px'>
          <span className='text-14px text-t-primary'>{t('settings.webui.enable')}</span>
          {starting ? (
            <span className='text-12px text-warning'>{t('settings.webui.starting')}</span>
          ) : running ? (
            <span className='text-12px text-success'>✓ {t('settings.webui.running')}</span>
          ) : null}
        </div>
        <Switch
          checked={running}
          loading={starting}
          disabled={!lifecycleSupported}
          onChange={(checked) => (checked ? void start() : void stop())}
        />
      </div>}

      {mode === 'popover' && <WebuiAccessAddresses />}

      {/* 登录信息 / Login Info */}
      <div className='flex flex-col gap-4px'>
        <div className='text-12px font-500 text-t-tertiary px-2px'>{t('settings.webui.loginInfo')}</div>

        {/* 账号 / Account */}
        <div className='flex items-center justify-between gap-12px'>
          <span className='text-13px text-t-secondary shrink-0'>{t('settings.webui.username')}:</span>
          <div className={CREDENTIAL_PILL_CLASS}>
            <span className='text-13px text-t-primary truncate'>{displayUsername || t(usernamePending ? 'common.loading' : 'common.unknownUser')}</span>
            <div className={CREDENTIAL_ACTIONS_CLASS}>
              <Tooltip content={t('common.copy')}>
                <Button
                  type='text'
                  size='mini'
                  className={CREDENTIAL_ICON_BUTTON_CLASS}
                  aria-label={t('common.copy')}
                  disabled={!displayUsername}
                  onClick={() => { if (displayUsername) handleCopy(displayUsername); }}
                >
                  <Copy size={13} />
                </Button>
              </Tooltip>
              <Tooltip content={t('settings.webui.editUsernameTooltip')}>
                <Button
                  type='text'
                  size='mini'
                  className={CREDENTIAL_ICON_BUTTON_CLASS}
                  aria-label={t('settings.webui.editUsernameTooltip')}
                  disabled={!displayUsername}
                  onClick={handleResetUsername}
                >
                  <EditTwo size={13} />
                </Button>
              </Tooltip>
            </div>
          </div>
        </div>

        {/* 密码 / Password */}
        <div className='flex items-center justify-between gap-12px'>
          <div className='flex shrink-0 items-center gap-4px'>
            <span className='text-13px text-t-secondary'>{t('settings.webui.password')}:</span>
            {(availablePassword || passwordIsSet) && <ConnectionHint
              label={t('settings.webui.password')}
              content={t(availablePassword ? 'settings.webui.initialPasswordHint' : 'settings.webui.passwordUnavailableHint')}
            />}
          </div>
          <div className={CREDENTIAL_PILL_CLASS}>
            <span className='text-13px text-t-primary truncate'>{displayPassword}</span>
            <div className={CREDENTIAL_ACTIONS_CLASS}>
              {availablePassword && <>
                <Tooltip content={t(passwordVisible ? 'settings.webui.hidePassword' : 'settings.webui.showPassword')} trigger={['hover', 'focus']}>
                  <Button
                    type='text'
                    size='mini'
                    className={CREDENTIAL_ICON_BUTTON_CLASS}
                    aria-label={t(passwordVisible ? 'settings.webui.hidePassword' : 'settings.webui.showPassword')}
                    aria-pressed={passwordVisible}
                    onClick={() => setPasswordVisible((visible) => !visible)}
                  >
                    {passwordVisible ? <PreviewCloseOne size={13} /> : <PreviewOpen size={13} />}
                  </Button>
                </Tooltip>
                <Tooltip content={t('settings.webui.copyPasswordTooltip')} trigger={['hover', 'focus']}>
                  <Button type='text' size='mini' className={CREDENTIAL_ICON_BUTTON_CLASS} aria-label={t('settings.webui.copyPasswordTooltip')} onClick={() => handleCopy(availablePassword)}>
                    <Copy size={13} />
                  </Button>
                </Tooltip>
              </>}
              <Tooltip content={t('settings.webui.changePassword')}>
                <Button
                  type='text'
                  size='mini'
                  className={CREDENTIAL_ICON_BUTTON_CLASS}
                  aria-label={t('settings.webui.changePassword')}
                  onClick={handleResetPassword}
                >
                  <EditTwo size={13} />
                </Button>
              </Tooltip>
            </div>
          </div>
        </div>
      </div>

      {lifecycleSupported && status?.running && status.allowRemote && <WebuiQrLoginPanel
        baseUrls={qrBaseUrls}
        selectedBaseUrl={selectedQrDisplayUrl}
        onBaseUrlChange={setSelectedQrBaseUrl}
        qrUrl={qrUrl}
        expiresAt={qrExpiresAt}
        loading={qrLoading}
        onRefresh={() => void generateQRCode()}
      />}

      {/* 设置新用户名弹窗 / Set New Username Modal */}
      <NomiModal
        visible={setUsernameModalVisible}
        onCancel={() => setSetUsernameModalVisible(false)}
        onOk={handleSetNewUsername}
        confirmLoading={usernameLoading}
        header={{ title: t('settings.webui.setNewUsername'), showClose: true }}
        size='small'
      >
        <Form form={usernameForm} layout='vertical' className='pt-12px'>
          {!lifecycleSupported && (
            <Form.Item
              label={t('settings.webui.currentPassword')}
              field='currentPassword'
              rules={[{ required: true, message: t('settings.webui.currentPasswordRequired') }]}
            >
              <Input.Password placeholder={t('settings.webui.currentPasswordPlaceholder')} />
            </Form.Item>
          )}
          <Form.Item
            label={t('settings.webui.newUsername')}
            field='newUsername'
            rules={[
              { required: true, message: t('settings.webui.newUsernameRequired') },
              {
                validator: (value, callback) => {
                  if (typeof value !== 'string') {
                    callback();
                    return;
                  }

                  const trimmed = value.trim();
                  if (trimmed.length < 3) {
                    callback(t('settings.webui.usernameMinLength'));
                    return;
                  }

                  if (trimmed.length > 32) {
                    callback(t('settings.webui.usernameMaxLength'));
                    return;
                  }

                  if (!/^[a-zA-Z0-9_-]+$/.test(trimmed)) {
                    callback(t('settings.webui.usernameFormatError'));
                    return;
                  }

                  if (/^[_-]|[_-]$/.test(trimmed)) {
                    callback(t('settings.webui.usernameEdgeError'));
                    return;
                  }

                  callback();
                },
              },
            ]}
          >
            <Input placeholder={t('settings.webui.newUsernamePlaceholder')} />
          </Form.Item>
        </Form>
      </NomiModal>

      {/* 设置新密码弹窗 / Set New Password Modal */}
      <NomiModal
        visible={setPasswordModalVisible}
        onCancel={() => setSetPasswordModalVisible(false)}
        onOk={handleSetNewPassword}
        confirmLoading={passwordLoading}
        header={{ title: t('settings.webui.setNewPassword'), showClose: true }}
        size='small'
      >
        <Form form={form} layout='vertical' className='pt-12px'>
          {!lifecycleSupported && (
            <Alert className='mb-16px' type='warning' showIcon content={t('settings.deviceConnections.passwordChangeSignsOutDevices')} />
          )}
          {!lifecycleSupported && (
            <Form.Item
              label={t('settings.webui.currentPassword')}
              field='currentPassword'
              rules={[{ required: true, message: t('settings.webui.currentPasswordRequired') }]}
            >
              <Input.Password placeholder={t('settings.webui.currentPasswordPlaceholder')} />
            </Form.Item>
          )}
          <Form.Item
            label={t('settings.webui.newPassword')}
            field='newPassword'
            rules={[
              { required: true, message: t('settings.webui.newPasswordRequired') },
              { minLength: 8, message: t('settings.webui.passwordMinLength') },
            ]}
          >
            <Input.Password placeholder={t('settings.webui.newPasswordPlaceholder')} />
          </Form.Item>
          <Form.Item
            label={t('settings.webui.confirmPassword')}
            field='confirmPassword'
            rules={[
              { required: true, message: t('settings.webui.confirmPasswordRequired') },
              {
                validator: (value, callback) => {
                  if (value !== form.getFieldValue('newPassword')) {
                    callback(t('settings.webui.passwordMismatch'));
                  } else {
                    callback();
                  }
                },
              },
            ]}
          >
            <Input.Password placeholder={t('settings.webui.confirmPasswordPlaceholder')} />
          </Form.Item>
        </Form>
      </NomiModal>
    </div>
  );
};

export default WebuiControlPanel;
