/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Tooltip } from '@arco-design/web-react';
import { BookOne } from '@icon-park/react';
import classNames from 'classnames';
import type { SiderTooltipProps } from '@renderer/utils/ui/siderTooltip';

interface SiderKnowledgeEntryProps {
  isMobile: boolean;
  isActive: boolean;
  collapsed: boolean;
  siderTooltipProps: SiderTooltipProps;
  onClick: () => void;
  /** Red dot — set when there are unreviewed knowledge write-back proposals. */
  dot?: boolean;
}

/** Small red badge dot for the "unreviewed proposals" signal. */
const RedDot: React.FC = () => (
  <span
    className='absolute rounded-full bg-red-500'
    style={{ width: 7, height: 7, top: -1, right: -1 }}
  />
);

const SiderKnowledgeEntry: React.FC<SiderKnowledgeEntryProps> = ({
  isMobile,
  isActive,
  collapsed,
  siderTooltipProps,
  onClick,
  dot = false,
}) => {
  const { t } = useTranslation();

  if (collapsed) {
    return (
      <Tooltip {...siderTooltipProps} content={t('knowledge.title')} position='right'>
        <div
          className={classNames(
            'w-full h-34px flex items-center justify-center cursor-pointer transition-colors rd-8px text-t-primary',
            isActive ? '!bg-primary-1 !text-primary-6' : 'hover:bg-fill-2 active:bg-fill-3'
          )}
          onClick={onClick}
        >
          <span className='relative block leading-none shrink-0' style={{ lineHeight: 0 }}>
            <BookOne theme='outline' size='20' fill='currentColor' className='block leading-none' />
            {dot && <RedDot />}
          </span>
        </div>
      </Tooltip>
    );
  }

  return (
    <Tooltip {...siderTooltipProps} content={t('knowledge.title')} position='right'>
      <div
        className={classNames(
          'box-border group h-34px w-full flex items-center justify-start gap-8px pl-10px pr-8px rd-0.5rem cursor-pointer shrink-0 transition-all text-t-primary',
          isMobile && 'sider-action-btn-mobile',
          isActive ? '!bg-primary-1 !text-primary-6' : 'hover:bg-fill-2 active:bg-fill-3'
        )}
        onClick={onClick}
      >
        <span className='relative size-22px flex items-center justify-center shrink-0'>
          <BookOne
            theme='outline'
            size='16'
            fill='currentColor'
            className='block leading-none'
            style={{ lineHeight: 0 }}
          />
          {dot && <RedDot />}
        </span>
        <span className='collapsed-hidden text-14px font-[500] leading-24px'>
          {t('knowledge.title')}
        </span>
      </div>
    </Tooltip>
  );
};

export default SiderKnowledgeEntry;
