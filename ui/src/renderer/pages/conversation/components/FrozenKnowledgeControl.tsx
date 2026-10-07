/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { TypedResourceBinding } from '@/common/types/agentPlatform';
import { CAPABILITY_COLORS } from '@/renderer/components/capability/CapabilityIcon';
import { Button, Popover } from '@arco-design/web-react';
import { BookOne } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import {
  capabilityHeaderButtonClass,
  capabilityHeaderButtonStyle,
} from './CapabilityHeaderButton';

type Props = {
  resources: readonly TypedResourceBinding[];
};

const FrozenKnowledgeControl: React.FC<Props> = ({ resources }) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const mounted = resources.length > 0;
  const color = mounted ? CAPABILITY_COLORS.primary : CAPABILITY_COLORS.off;
  const status = mounted
    ? t('knowledge.control.mounted', { count: resources.length })
    : t('knowledge.control.off');

  const panel = (
    <div className='box-border flex w-340px max-h-480px flex-col gap-10px overflow-hidden p-12px'>
      <div className='flex items-center justify-between gap-10px'>
        <span className='inline-flex min-w-0 items-center gap-8px'>
          <span className='inline-flex h-24px w-24px shrink-0 items-center justify-center rounded-6px bg-[rgba(var(--primary-6),0.1)] text-primary-6'>
            <BookOne theme='outline' size='15' fill='currentColor' />
          </span>
          <span className='truncate text-13px font-600 text-t-primary'>
            {t('knowledge.control.label')}
          </span>
        </span>
        <span className='inline-flex shrink-0 items-center gap-5px rounded-full border border-solid border-[var(--color-border-2)] px-7px py-3px text-11px text-[var(--color-text-1)]'>
          <span className='h-6px w-6px rounded-full' style={{ backgroundColor: color }} />
          {status}
        </span>
      </div>

      <p className='m-0 text-11px leading-16px text-[var(--color-text-2)]'>
        {t('conversation.chat.frozenSessionConfigHint')}
      </p>

      <div className='flex min-h-0 flex-col gap-6px overflow-y-auto rounded-12px border border-solid border-[var(--color-border-2)] p-10px'>
        <span className='text-11px font-600 text-[var(--color-text-1)]'>
          {t('knowledge.control.basesLabel', { defaultValue: '挂载的知识库' })}
        </span>
        {resources.length === 0 ? (
          <span className='py-4px text-11px text-[var(--color-text-2)]'>
            {t('agentSettings.resources.emptyOptional')}
          </span>
        ) : resources.map((resource) => {
          const parameters = resource.typed_parameters ?? {};
          const name = parameters.knowledge_name?.trim() || resource.resource_id;
          const description = parameters.knowledge_description?.trim();
          return (
            <div
              key={resource.binding_id}
              className='rounded-8px border border-solid border-[rgba(var(--primary-6),0.32)] bg-[rgba(var(--primary-6),0.06)] px-9px py-8px'
            >
              <div className='truncate text-12px font-600 text-[var(--color-text-1)]' title={name}>
                {name}
              </div>
              {description ? (
                <div className='mt-2px line-clamp-2 text-11px leading-15px text-[var(--color-text-2)]'>
                  {description}
                </div>
              ) : null}
            </div>
          );
        })}
      </div>

      <button
        type='button'
        className='self-start border-0 bg-transparent p-0 text-11px font-600 text-primary-6 cursor-pointer hover:underline'
        onClick={() => navigate('/knowledge')}
      >
        {t('knowledge.mount.manage', { defaultValue: '管理知识库 ›' })}
      </button>
    </div>
  );

  return (
    <Popover
      className='knowledge-control-popover'
      trigger='click'
      position='br'
      content={panel}
    >
      <Button
        size='mini'
        shape='round'
        type='secondary'
        className={capabilityHeaderButtonClass(mounted, 'shrink-0')}
        style={capabilityHeaderButtonStyle(color)}
      >
        <span className='inline-flex items-center gap-6px leading-none'>
          <BookOne theme='outline' size='14' fill={color} className='block' />
          <span className='text-12px'>{t('knowledge.control.label')}</span>
        </span>
      </Button>
    </Popover>
  );
};

export default FrozenKnowledgeControl;
