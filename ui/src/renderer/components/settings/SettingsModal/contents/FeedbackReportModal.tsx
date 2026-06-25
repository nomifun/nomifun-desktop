/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 * Based on AionUi (https://github.com/iOfficeAI/AionUi)
 */

import ModalWrapper from '@renderer/components/base/ModalWrapper';
import { openExternalUrl } from '@renderer/utils/platform';
import { Info } from '@icon-park/react';
import React, { useCallback } from 'react';
import { useTranslation } from 'react-i18next';

const OFFICIAL_SITE = 'https://nomifun.com';
const OFFICIAL_SITE_LABEL = 'nomifun.com';
const COPYRIGHT = '© 2025–2026 NomiFun · nomifun.com';

// 以下导出类型与 props 形状保持不变，以兼容现有调用方（FeedbackButton / 一键反馈入口等）。
export type PrefilledScreenshot = {
  filename: string;
  data: Uint8Array;
  type: string;
};

export type FeedbackEventTags = Record<string, string>;
export type FeedbackEventExtra = Record<string, unknown>;

type FeedbackReportModalProps = {
  visible: boolean;
  onCancel: () => void;
  defaultModule?: string;
  prefilledScreenshots?: PrefilledScreenshot[];
  feedbackTags?: FeedbackEventTags;
  feedbackExtra?: FeedbackEventExtra;
};

/**
 * “联系我们”面板：不再在客户端收集/上报反馈，仅引导用户前往官网
 * 通过群聊 / 邮箱联系我们，并展示版权信息。
 */
const FeedbackReportModal: React.FC<FeedbackReportModalProps> = ({ visible, onCancel }) => {
  const { t } = useTranslation();

  const openSite = useCallback(() => {
    void openExternalUrl(OFFICIAL_SITE).catch((e) => console.error('open official site failed', e));
  }, []);

  return (
    <ModalWrapper
      title={t('settings.contactTitle')}
      visible={visible}
      onCancel={onCancel}
      onOk={openSite}
      okText={t('settings.contactVisitWebsite')}
      cancelText={t('settings.bugReportCancel')}
      alignCenter
      className='w-[min(460px,calc(100vw-32px))] max-w-460px rd-16px'
      autoFocus={false}
      wrapStyle={{ zIndex: 1050 }}
      maskStyle={{ zIndex: 1050 }}
    >
      <div className='flex flex-col items-center gap-12px px-24px pb-12px pt-4px text-center'>
        <Info theme='outline' size='28' />
        <p className='m-0 text-13px leading-20px text-t-secondary'>
          {t('settings.contactDescription')}
        </p>
        <div className='text-14px text-t-secondary'>{OFFICIAL_SITE_LABEL}</div>
        <div className='mt-8px text-12px text-t-tertiary'>{COPYRIGHT}</div>
      </div>
    </ModalWrapper>
  );
};

export default FeedbackReportModal;
