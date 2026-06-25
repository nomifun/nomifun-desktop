/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * TeachingCard — Contextual callout showing "what / how to fill / how to use"
 * for the currently selected source type.
 *
 * Theme variables only; uses primary-light gradient background with accent border.
 * Text follows the exact copy from the design spec (SRC[*].teach).
 */
import React from 'react';
import { useTranslation } from 'react-i18next';
import { Info } from '@icon-park/react';
import type { StudioSourceType } from './sourceTypes';

// ─── Props ──────────────────────────────────────────────────────────────────

export interface TeachingCardProps {
  sourceType: StudioSourceType;
}

// ─── Teaching data per source type ──────────────────────────────────────────

interface TeachRow {
  labelKey: string;
  labelDefault: string;
  textKey: string;
  textDefault: string;
}

const getTeachRows = (sourceType: StudioSourceType): TeachRow[] => {
  switch (sourceType) {
    case 'blank':
      return [
        {
          labelKey: 'knowledge.studio.teachWhat',
          labelDefault: '是什么',
          textKey: 'knowledge.studio.blankTeachWhat',
          textDefault: '一个应用托管的空 .md 目录，删除时可一并清理。',
        },
        {
          labelKey: 'knowledge.studio.teachHow',
          labelDefault: '怎么填',
          textKey: 'knowledge.studio.blankTeachHow',
          textDefault: '创建后在详情页直接编辑、拖入文件，或让 AI 生成。',
        },
        {
          labelKey: 'knowledge.studio.teachUse',
          labelDefault: '怎么用',
          textKey: 'knowledge.studio.blankTeachUse',
          textDefault: '挂载到任意会话 / 终端 / 数字伙伴，模型即可在 .nomi/knowledge/ 下检索。',
        },
      ];
    case 'local':
      return [
        {
          labelKey: 'knowledge.studio.teachWhat',
          labelDefault: '是什么',
          textKey: 'knowledge.studio.localTeachWhat',
          textDefault: '把你电脑上已有目录"引用"为知识库（不复制、不搬动）。',
        },
        {
          labelKey: 'knowledge.studio.teachHow',
          labelDefault: '怎么填',
          textKey: 'knowledge.studio.localTeachHow',
          textDefault: '点"选择文件夹"挑一个含 .md 的目录即可。',
        },
        {
          labelKey: 'knowledge.studio.teachUse',
          labelDefault: '怎么用',
          textKey: 'knowledge.studio.localTeachUse',
          textDefault: '挂载即用；你在原目录里改文档，库内容随之更新。',
        },
      ];
    case 'web':
      return [
        {
          labelKey: 'knowledge.studio.teachWhat',
          labelDefault: '是什么',
          textKey: 'knowledge.studio.webTeachWhat',
          textDefault: '把一组网址作为知识来源（快照存档或实时查询）。',
        },
        {
          labelKey: 'knowledge.studio.teachHow',
          labelDefault: '怎么填',
          textKey: 'knowledge.studio.webTeachHow',
          textDefault: '粘贴网址、可选填标题；JS 重的页面勾选"浏览器渲染"。',
        },
        {
          labelKey: 'knowledge.studio.teachUse',
          labelDefault: '怎么用',
          textKey: 'knowledge.studio.webTeachUse',
          textDefault: '挂载后模型可检索这些页面；快照可在详情页一键刷新。',
        },
      ];
    case 'feishu':
      return [
        {
          labelKey: 'knowledge.studio.teachWhat',
          labelDefault: '是什么',
          textKey: 'knowledge.studio.feishuTeachWhat',
          textDefault: '连接飞书 Wiki 空间，把文档同步成本地快照供检索。',
        },
        {
          labelKey: 'knowledge.studio.teachHow',
          labelDefault: '怎么填',
          textKey: 'knowledge.studio.feishuTeachHow',
          textDefault: '选/建凭证 → 填空间 ID → 选同步频率，保存后首次同步。',
        },
        {
          labelKey: 'knowledge.studio.teachUse',
          labelDefault: '怎么用',
          textKey: 'knowledge.studio.feishuTeachUse',
          textDefault: '同步完成即为普通库，挂载即用；可定时增量同步。',
        },
      ];
    case 'import':
      return [
        {
          labelKey: 'knowledge.studio.teachWhat',
          labelDefault: '是什么',
          textKey: 'knowledge.studio.importTeachWhat',
          textDefault: '从导出的 .zip 知识库包还原成新库。',
        },
        {
          labelKey: 'knowledge.studio.teachHow',
          labelDefault: '怎么填',
          textKey: 'knowledge.studio.importTeachHow',
          textDefault: '选择 .zip 文件即可，其余自动完成。',
        },
        {
          labelKey: 'knowledge.studio.teachUse',
          labelDefault: '怎么用',
          textKey: 'knowledge.studio.importTeachUse',
          textDefault: '导入后即普通托管库，可编辑、挂载、再导出。',
        },
      ];
  }
};

// ─── Component ──────────────────────────────────────────────────────────────

const TeachingCard: React.FC<TeachingCardProps> = ({ sourceType }) => {
  const { t } = useTranslation();
  const rows = getTeachRows(sourceType);

  return (
    <div className='knowledge-studio-teaching-card mt-12px rounded-16px bg-[var(--color-bg-2)] p-14px shadow-[0_10px_30px_rgba(15,23,42,0.035)]'>
      {/* Header */}
      <div className='mb-10px flex items-center gap-8px text-12px font-700 text-[var(--color-text-1)]'>
        <span className='grid size-24px place-items-center rounded-8px bg-[rgba(var(--primary-6),0.08)] text-[rgb(var(--primary-6))]'>
          <Info theme='outline' size='14' />
        </span>
        {t('knowledge.studio.teachHeader', { defaultValue: '它是什么 · 怎么填 · 怎么用' })}
      </div>

      {/* Rows */}
      <div className='grid gap-6px'>
        {rows.map((row, idx) => (
          <div key={idx} className='flex gap-10px rounded-10px bg-[var(--color-fill-1)] px-10px py-7px text-12px leading-relaxed text-[var(--color-text-2)]'>
            <span className='w-54px flex-none font-600 text-[var(--color-text-1)]'>
              {t(row.labelKey, { defaultValue: row.labelDefault })}
            </span>
            <span>{t(row.textKey, { defaultValue: row.textDefault })}</span>
          </div>
        ))}
      </div>
    </div>
  );
};

export default TeachingCard;
