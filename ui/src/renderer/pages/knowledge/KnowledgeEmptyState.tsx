/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { BookOne, FolderOpen, Plus, Upload, Earth, LarkOne } from '@icon-park/react';
import { FEISHU_KNOWLEDGE_CREATION_ENABLED } from './CreateStudio/sourceTypes';

export type KnowledgeKindShortcut = 'blank' | 'local' | 'web' | 'feishu';

interface KnowledgeEmptyStateProps {
  onCreate: (initialKind?: KnowledgeKindShortcut) => void;
  onImport?: () => void;
}

/**
 * First-run empty state for the knowledge list: prominent three-step onboarding
 * (create -> fill -> mount), type shortcut tiles, and import CTA.
 *
 * Redesigned to be visually prominent as a full-page landing. All colors use
 * theme variables (no hard-coded semantic colors).
 */
const KnowledgeEmptyState: React.FC<KnowledgeEmptyStateProps> = ({ onCreate, onImport }) => {
  const { t } = useTranslation();

  // ─── Three-step lifecycle ───────────────────────────────────────────────────

  const steps: { num: number; icon: React.ReactNode; title: string; desc: string }[] = [
    {
      num: 1,
      icon: <Plus theme='outline' size='18' />,
      title: t('knowledge.onboarding.step1Title', { defaultValue: '创建' }),
      desc: t('knowledge.onboarding.step1Desc', { defaultValue: '从空白、本地文件夹或一组 URL 创建知识库。' }),
    },
    {
      num: 2,
      icon: <BookOne theme='outline' size='18' />,
      title: t('knowledge.onboarding.step2Title', { defaultValue: '填充' }),
      desc: t('knowledge.onboarding.step2Desc', { defaultValue: '放入 .md 文档——也可以让 AI 自动生成梗概和 README。' }),
    },
    {
      num: 3,
      icon: <FolderOpen theme='outline' size='18' />,
      title: t('knowledge.onboarding.step3Title', { defaultValue: '挂载' }),
      desc: t('knowledge.onboarding.step3Desc', { defaultValue: '挂载到会话，模型会在 .nomi/knowledge/ 下随时查阅。' }),
    },
  ];

  // ─── Type shortcut tiles ────────────────────────────────────────────────────

  const kinds: {
    key: KnowledgeKindShortcut;
    label: string;
    icon: React.ReactNode;
    disabled?: boolean;
    badge?: string;
  }[] = [
    {
      key: 'blank',
      label: t('knowledge.empty.kindBlank', { defaultValue: '空白知识库' }),
      icon: <Plus theme='outline' size='20' />,
    },
    {
      key: 'local',
      label: t('knowledge.empty.kindLocal', { defaultValue: '本地目录' }),
      icon: <FolderOpen theme='outline' size='20' />,
    },
    {
      key: 'web',
      label: t('knowledge.empty.kindWeb', { defaultValue: '从网页抓取' }),
      icon: <Earth theme='outline' size='20' />,
    },
    {
      key: 'feishu',
      label: t('knowledge.empty.kindFeishu', { defaultValue: '飞书文档' }),
      icon: <LarkOne theme='outline' size='20' />,
      disabled: !FEISHU_KNOWLEDGE_CREATION_ENABLED,
      badge: !FEISHU_KNOWLEDGE_CREATION_ENABLED
        ? t('knowledge.studio.temporarilyDisabled', { defaultValue: '暂不可用' })
        : undefined,
    },
  ];

  return (
    <div className='flex w-full flex-col items-center gap-32px px-16px py-56px'>
      {/* Hero icon + headline */}
      <div className='flex flex-col items-center gap-12px text-center'>
        <div className='flex size-72px items-center justify-center rounded-full bg-[var(--color-primary-light-1)] text-[rgb(var(--primary-6))]'>
          <BookOne theme='outline' size='36' fill='currentColor' />
        </div>
        <h2 className='m-0 text-22px font-bold text-[var(--color-text-1)]'>
          {t('knowledge.onboarding.title', { defaultValue: '开始管理你的专属知识' })}
        </h2>
        <p className='m-0 max-w-480px text-14px leading-relaxed text-[var(--color-text-3)]'>
          {t('knowledge.onboarding.subtitle', {
            defaultValue: '知识库是一个 Markdown 文档目录，可以挂载进任意会话，作为模型的扩展知识来源。',
          })}
        </p>
      </div>

      {/* Three-step onboarding cards */}
      <div className='flex w-full max-w-780px flex-col gap-14px sm:flex-row'>
        {steps.map((s) => (
          <div
            key={s.num}
            className='flex flex-1 flex-col gap-10px rounded-14px border border-solid border-[var(--color-border-2)] bg-[var(--color-fill-1)] p-20px'
          >
            {/* Step number badge + title */}
            <div className='flex items-center gap-10px'>
              <span className='flex size-28px items-center justify-center rounded-8px bg-[var(--color-primary-light-1)] text-[rgb(var(--primary-6))] text-12px font-bold'>
                {s.num}
              </span>
              <span className='flex items-center gap-6px text-14px font-semibold text-[var(--color-text-1)]'>
                {s.icon}
                {s.title}
              </span>
            </div>
            {/* Description */}
            <span className='text-13px leading-[1.7] text-[var(--color-text-3)]'>{s.desc}</span>
          </div>
        ))}
      </div>

      {/* Type shortcut tiles */}
      <div className='flex w-full max-w-780px flex-col gap-10px'>
        <span className='text-12px font-semibold text-[var(--color-text-3)]'>
          {t('knowledge.empty.quickStart', { defaultValue: '快速开始' })}
        </span>
        <div className='grid grid-cols-2 gap-10px sm:grid-cols-4'>
          {kinds.map((k) => (
            <div
              key={k.key}
              role='button'
              tabIndex={k.disabled ? -1 : 0}
              aria-disabled={k.disabled || undefined}
              onClick={() => {
                if (!k.disabled) onCreate(k.key);
              }}
              onKeyDown={(e) => {
                if (!k.disabled && (e.key === 'Enter' || e.key === ' ')) {
                  e.preventDefault();
                  onCreate(k.key);
                }
              }}
              className={[
                'flex flex-col items-center justify-center gap-8px select-none',
                'rounded-12px border border-dashed border-[var(--color-border-2)] bg-transparent',
                'py-20px px-12px',
                'text-[var(--color-text-2)]',
                k.disabled
                  ? 'cursor-not-allowed opacity-50'
                  : 'cursor-pointer hover:border-[var(--color-primary-light-3)] hover:text-[rgb(var(--primary-6))] hover:bg-[var(--color-primary-light-1)]',
                'transition-all duration-150',
              ].join(' ')}
            >
              {k.icon}
              <span className='text-12px font-medium'>{k.label}</span>
              {k.badge && (
                <span className='rounded-5px bg-[var(--color-fill-3)] px-6px py-1px text-10px text-[var(--color-text-3)]'>
                  {k.badge}
                </span>
              )}
            </div>
          ))}
        </div>
      </div>

      {/* Primary CTA + Import secondary CTA */}
      <div className='flex items-center gap-14px'>
        <div
          role='button'
          tabIndex={0}
          onClick={() => onCreate()}
          onKeyDown={(e) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault();
              onCreate();
            }
          }}
          className={[
            'inline-flex items-center gap-7px cursor-pointer select-none',
            'rounded-full px-22px py-10px text-14px font-700',
            'border border-solid border-transparent',
            'bg-[rgba(var(--primary-6),0.12)] text-[var(--color-text-1)]',
            'shadow-[0_6px_18px_rgba(var(--primary-6),0.14)]',
            'hover:bg-[rgba(var(--primary-6),0.18)]',
            'focus-visible:border-[rgb(var(--primary-6))] focus-visible:outline-none',
            'transition-all duration-150',
          ].join(' ')}
        >
          <Plus theme='outline' size='15' className='text-[rgb(var(--primary-6))]' />
          {t('knowledge.newBase', { defaultValue: '新建知识库' })}
        </div>
        {onImport && (
          <div
            role='button'
            tabIndex={0}
            onClick={onImport}
            onKeyDown={(e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault();
                onImport();
              }
            }}
            className={[
              'inline-flex items-center gap-7px cursor-pointer select-none',
              'rounded-full px-22px py-10px text-14px font-medium',
              'border border-solid border-[var(--color-border-2)] bg-[var(--color-fill-1)] text-[var(--color-text-2)]',
              'hover:border-[var(--color-primary-light-3)] hover:text-[rgb(var(--primary-6))] hover:bg-[var(--color-primary-light-1)]',
              'transition-all duration-150',
            ].join(' ')}
          >
            <Upload theme='outline' size='15' />
            {t('knowledge.onboarding.import', { defaultValue: '导入' })}
          </div>
        )}
      </div>
    </div>
  );
};

export default KnowledgeEmptyState;
