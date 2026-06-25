/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * TypeRail — Left rail of the CreateStudio dialog.
 *
 * Three groups: Regular (blank / local / web), Third-party (feishu-disabled /
 * notion-disabled), Migration (import). Selected item uses primary theme; hover
 * uses fill-2. Disabled items carry an availability badge.
 *
 * Theme variables only; no hard-coded semantic colors.
 */
import React from 'react';
import { useTranslation } from 'react-i18next';
import { AllApplication, BookOne, Earth, FolderOpen, LarkOne, Upload } from '@icon-park/react';
import { FEISHU_KNOWLEDGE_CREATION_ENABLED, type StudioSourceType } from './sourceTypes';

// ─── Rail item definition ────────────────────────────────────────────────────

interface RailItem {
  key: string;
  icon: React.ReactNode;
  title: string;
  description: string;
  disabled?: boolean;
  badge?: string;
}

interface RailGroup {
  label: string;
  items: RailItem[];
}

// ─── Props ───────────────────────────────────────────────────────────────────

export interface TypeRailProps {
  value: StudioSourceType;
  onChange: (value: StudioSourceType) => void;
}

// ─── Component ───────────────────────────────────────────────────────────────

const TypeRail: React.FC<TypeRailProps> = ({ value, onChange }) => {
  const { t } = useTranslation();

  const groups: RailGroup[] = [
    {
      label: t('knowledge.studio.groupRegular', { defaultValue: '常规' }),
      items: [
        {
          key: 'blank',
          icon: <BookOne theme='outline' size='16' />,
          title: t('knowledge.studio.typeBlank', { defaultValue: '空白知识库' }),
          description: t('knowledge.studio.typeBlankDesc', { defaultValue: '从零开始，最简单' }),
        },
        {
          key: 'local',
          icon: <FolderOpen theme='outline' size='16' />,
          title: t('knowledge.studio.typeLocal', { defaultValue: '本地文件夹' }),
          description: t('knowledge.studio.typeLocalDesc', { defaultValue: '引用电脑上已有目录' }),
        },
        {
          key: 'web',
          icon: <Earth theme='outline' size='16' />,
          title: t('knowledge.studio.typeWeb', { defaultValue: '网页 / URL' }),
          description: t('knowledge.studio.typeWebDesc', { defaultValue: '抓取一组网址' }),
        },
      ],
    },
    {
      label: t('knowledge.studio.groupThirdParty', { defaultValue: '连接第三方' }),
      items: [
        {
          key: 'feishu',
          icon: <LarkOne theme='outline' size='16' />,
          title: t('knowledge.studio.typeFeishu', { defaultValue: '飞书知识空间' }),
          description: t('knowledge.studio.typeFeishuDesc', { defaultValue: '同步 Wiki 文档' }),
          disabled: !FEISHU_KNOWLEDGE_CREATION_ENABLED,
          badge: !FEISHU_KNOWLEDGE_CREATION_ENABLED
            ? t('knowledge.studio.temporarilyDisabled', { defaultValue: '暂不可用' })
            : undefined,
        },
        {
          key: 'notion',
          icon: <AllApplication theme='outline' size='16' />,
          title: t('knowledge.studio.typeNotion', { defaultValue: 'Notion / 更多' }),
          description: '',
          disabled: true,
          badge: t('knowledge.studio.comingSoon', { defaultValue: '即将支持' }),
        },
      ],
    },
    {
      label: t('knowledge.studio.groupMigration', { defaultValue: '迁移' }),
      items: [
        {
          key: 'import',
          icon: <Upload theme='outline' size='16' />,
          title: t('knowledge.studio.typeImport', { defaultValue: '导入 .zip 包' }),
          description: t('knowledge.studio.typeImportDesc', { defaultValue: '从导出的备份还原' }),
        },
      ],
    },
  ];

  return (
    <div className='flex flex-col gap-0 overflow-y-auto border-r border-r-[var(--color-border)] bg-[var(--color-bg-1)] p-16px pr-12px'>
      {groups.map((group, gi) => (
        <React.Fragment key={gi}>
          <div
            className={`text-11px font-600 tracking-wide text-[var(--color-text-3)] ${gi === 0 ? 'mb-8px' : 'mb-8px mt-14px'}`}
          >
            {group.label}
          </div>
          {group.items.map((item, ii) => {
            const isSelected = !item.disabled && value === item.key;

            return (
              <div
                key={`${gi}-${ii}`}
                aria-disabled={item.disabled || undefined}
                onClick={() => {
                  if (!item.disabled) onChange(item.key as StudioSourceType);
                }}
                className={[
                  'flex cursor-pointer items-center gap-11px rounded-10px border border-transparent p-10px mb-3px transition-colors',
                  item.disabled && 'cursor-not-allowed opacity-50',
                  isSelected && '!bg-primary-1 !border-primary-6 !text-primary-6',
                  !isSelected && !item.disabled && 'hover:bg-fill-2',
                ]
                  .filter(Boolean)
                  .join(' ')}
              >
                {/* Icon box */}
                <div
                  className={[
                    'flex size-30px flex-none items-center justify-center rounded-8px border',
                    isSelected
                      ? '!bg-primary-1 !border-primary-6 !text-primary-6'
                      : 'bg-[var(--color-fill-2)] border-[var(--color-border-2)] text-[var(--color-text-2)]',
                  ].join(' ')}
                >
                  {item.icon}
                </div>

                {/* Text */}
                <div className='flex min-w-0 flex-1 flex-col'>
                  <span
                    className={[
                      'text-13px font-600 leading-tight',
                      isSelected ? '!text-primary-6' : 'text-[var(--color-text-1)]',
                    ].join(' ')}
                  >
                    {item.title}
                  </span>
                  {item.description && (
                    <span className='text-11px text-[var(--color-text-3)] leading-tight mt-1px'>
                      {item.description}
                    </span>
                  )}
                </div>

                {/* Coming soon badge */}
                {item.badge && (
                  <span className='ml-auto flex-none rounded-5px bg-[var(--color-fill-3)] px-6px py-1px text-10px text-[var(--color-text-3)]'>
                    {item.badge}
                  </span>
                )}
              </div>
            );
          })}
        </React.Fragment>
      ))}
    </div>
  );
};

export default TypeRail;
