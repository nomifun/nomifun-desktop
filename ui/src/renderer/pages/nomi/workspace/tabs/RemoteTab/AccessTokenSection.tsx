/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button, Message, Modal, Spin, Tag } from '@arco-design/web-react';
import { Key } from '@icon-park/react';
import React, { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { webui } from '@/common/adapter/ipcBridge';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type { CompanionId } from '@/common/types/ids';
import CopyIconButton from '@/renderer/components/base/CopyIconButton';
import { NomiSettingList, NomiSettingRow, NomiSettingSection } from '@/renderer/components/base/NomiSettingLayout';

interface AccessTokenSectionProps {
  companionId: CompanionId;
  companionName: string;
  /** From companion.status; null while the status is still loading. */
  modelConfigured: boolean | null;
}

/**
 * 「远程访问」节：本伙伴的访问令牌（生成 / 重新生成 / 吊销）。
 *
 * The plaintext token is returned by the backend exactly ONCE, at mint time —
 * it lives only in this component's state, is never re-fetched, and is dropped
 * as soon as the companion changes or the token is revoked. That "treasure
 * shown once" behaviour is the security contract of
 * `/api/webui/companions/{id}/access-token`, so the row shows only
 * configured/not-configured afterwards.
 */
const AccessTokenSection: React.FC<AccessTokenSectionProps> = ({ companionId, companionName, modelConfigured }) => {
  const { t } = useTranslation();

  const [configured, setConfigured] = useState<boolean | null>(null);
  const [statusLoading, setStatusLoading] = useState(false);
  const [minting, setMinting] = useState(false);
  const [revoking, setRevoking] = useState(false);
  const [plaintext, setPlaintext] = useState<string | null>(null);
  const [warning, setWarning] = useState<string | null>(null);

  // Status per companion. The one-time plaintext and the mint-time warning are
  // session- AND companion-scoped: clear them so a token never trails onto a
  // different companion after a switch.
  useEffect(() => {
    setPlaintext(null);
    setWarning(null);
    setConfigured(null);
    let alive = true;
    setStatusLoading(true);
    webui.companionAccessToken.status
      .invoke({ companionId })
      .then((res) => {
        if (alive) setConfigured(res.configured);
      })
      .catch((error) => {
        console.error('[RemoteTab] Companion access-token status failed:', error);
        if (alive) setConfigured(null);
      })
      .finally(() => {
        if (alive) setStatusLoading(false);
      });
    return () => {
      alive = false;
    };
  }, [companionId]);

  const friendlyError = useCallback(
    (error: unknown, fallback: string) =>
      isBackendHttpError(error) && error.backendMessage ? error.backendMessage : fallback,
    []
  );

  const mint = useCallback(async () => {
    setMinting(true);
    try {
      const res = await webui.companionAccessToken.mint.invoke({ companionId });
      setPlaintext(res.token);
      setWarning(res.warning ?? null);
      setConfigured(true);
      Message.success(t('settings.webui.companionToken.minted'));
    } catch (error) {
      console.error('[RemoteTab] Mint companion access token failed:', error);
      Message.error(friendlyError(error, t('settings.webui.companionToken.mintFailed')));
    } finally {
      setMinting(false);
    }
  }, [companionId, friendlyError, t]);

  // Regenerating silently invalidates the token external clients already hold —
  // confirm that; minting the FIRST token needs no ceremony. `configured === null`
  // means the status probe failed, so we must not treat it as "no token yet":
  // that would turn this button into a silent revoke of a live token.
  const handleMint = useCallback(() => {
    if (configured === false) {
      void mint();
      return;
    }
    Modal.confirm({
      title: t('nomi.remote.accessRegenerateTitle', { defaultValue: '重新生成访问令牌' }),
      content:
        configured === true
          ? t('nomi.remote.accessRegenerateConfirm', {
              defaultValue: '重新生成会立即使旧令牌失效，正在使用它的外部客户端会断开连接。确定继续吗？',
            })
          : t('nomi.remote.accessRegenerateUnknownConfirm', {
              defaultValue:
                '暂时无法确认这只伙伴是否已有访问令牌。如果已有，生成新令牌会立即使旧令牌失效，正在使用它的外部客户端会断开连接。确定继续吗？',
            }),
      onOk: () => mint(),
    });
  }, [configured, mint, t]);

  const confirmRevoke = useCallback(() => {
    Modal.confirm({
      title: t('nomi.remote.accessRevokeTitle', { defaultValue: '吊销访问令牌' }),
      content: t('settings.webui.companionToken.revokeConfirm'),
      okButtonProps: { status: 'danger' },
      onOk: async () => {
        setRevoking(true);
        try {
          await webui.companionAccessToken.revoke.invoke({ companionId });
          setConfigured(false);
          setPlaintext(null);
          setWarning(null);
          Message.success(t('settings.webui.companionToken.revoked'));
        } catch (error) {
          console.error('[RemoteTab] Revoke companion access token failed:', error);
          Message.error(friendlyError(error, t('settings.webui.companionToken.revokeFailed')));
        } finally {
          setRevoking(false);
        }
      },
    });
  }, [companionId, friendlyError, t]);

  const footerNodes: React.ReactNode[] = [];
  if (plaintext) {
    footerNodes.push(
      <div
        key='token'
        className='flex min-w-0 items-center gap-8px rd-full border border-solid border-[rgba(var(--primary-6),0.28)] bg-[rgba(var(--primary-6),0.08)] px-12px py-6px'
      >
        <Key theme='outline' size='14' fill='currentColor' strokeWidth={3} className='shrink-0 text-primary-6' />
        <span className='min-w-0 flex-1 truncate font-mono text-13px text-t-primary'>{plaintext}</span>
        <CopyIconButton text={plaintext} size={14} className='h-22px w-22px shrink-0' />
      </div>,
      <div key='once' className='text-12px leading-18px text-warning'>
        {t('settings.webui.companionToken.shownOnceHint')}
      </div>
    );
  }
  if (warning) {
    footerNodes.push(
      <div key='warning' className='text-12px leading-18px text-warning'>
        {warning}
      </div>
    );
  } else if (modelConfigured === false && configured === false) {
    footerNodes.push(
      <div key='no-model' className='text-12px leading-18px text-t-tertiary'>
        {t('settings.webui.companionToken.noModelHint')}
      </div>
    );
  }

  return (
    <NomiSettingSection
      title={t('nomi.remote.accessTitle', { defaultValue: '远程访问' })}
      description={t('nomi.remote.accessHint', {
        defaultValue: '桌面应用之外接入 {{companionName}}：外部 MCP / REST 客户端凭访问令牌以它的身份连接。',
        companionName,
      })}
    >
      <NomiSettingList>
        <NomiSettingRow
          leading={
            <Key theme='outline' size='16' fill='currentColor' strokeWidth={3} className='shrink-0 text-primary-6' />
          }
          title={
            <div className='flex min-w-0 flex-wrap items-center gap-6px'>
              <span className='truncate'>{t('nomi.remote.accessTokenRow', { defaultValue: '访问令牌' })}</span>
              {statusLoading ? (
                <Spin size={12} />
              ) : configured === null ? null : (
                <Tag size='small' color={configured ? 'green' : 'gray'}>
                  {configured
                    ? t('settings.webui.companionToken.statusActive')
                    : t('settings.webui.companionToken.statusNone')}
                </Tag>
              )}
            </div>
          }
          description={t('nomi.remote.accessTokenGrant', {
            defaultValue:
              '持有令牌者会以 {{companionName}} 的身份接入远程接口：读写它的记忆、调用它的技能、代它发起对话，以及远程接口开放的其他能力 —— 等同于交出这只伙伴的完整操作权限。请当作密码保管，泄露后立即吊销。',
            companionName,
          })}
          controls={
            <>
              <Button
                size='small'
                type='primary'
                loading={minting}
                disabled={revoking || statusLoading}
                onClick={handleMint}
              >
                {configured
                  ? t('settings.webui.companionToken.regenerate')
                  : t('settings.webui.companionToken.generate')}
              </Button>
              {configured && (
                <Button size='small' status='danger' loading={revoking} disabled={minting} onClick={confirmRevoke}>
                  {t('settings.webui.companionToken.revoke')}
                </Button>
              )}
            </>
          }
          footer={footerNodes.length > 0 ? <div className='flex flex-col gap-6px'>{footerNodes}</div> : undefined}
        />
      </NomiSettingList>
    </NomiSettingSection>
  );
};

export default AccessTokenSection;
