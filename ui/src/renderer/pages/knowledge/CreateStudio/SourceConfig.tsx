/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * SourceConfig — Right-side configuration panel that switches by `sourceType`.
 *
 * Renders the appropriate sub-block for each source kind:
 * - blank: informational note (no config needed)
 * - local: folder path selector
 * - web: snapshot/realtime segment + dynamic URL rows
 * - import: zip file selector
 *
 * Controlled: accepts `value` / `onChange` from parent (index.tsx holds state).
 * Theme variables only; no hard-coded semantic colors.
 */
import React, { useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Input, Message, Switch } from '@arco-design/web-react';
import { FolderOpen, Info } from '@icon-park/react';
import { ipcBridge } from '@/common';
import { isDesktopShell } from '@renderer/utils/platform';
import KnowledgeUrlEntriesEditor from '../KnowledgeUrlEntriesEditor';
import {
  MAX_KNOWLEDGE_SOURCE_ENTRIES,
  type KnowledgeUrlDraft,
} from '../knowledgeUrlEntries';
import type { StudioSourceType } from './sourceTypes';

// ─── Value Shape ────────────────────────────────────────────────────────────

export type UrlMode = 'snapshot' | 'live';

export type UrlEntry = KnowledgeUrlDraft;

export interface SourceConfigValue {
  /** local */
  rootPath?: string;
  /** web */
  urlMode?: UrlMode;
  urlEntries?: UrlEntry[];
  browserRender?: boolean;
  /** import */
  importPath?: string;
}

// ─── Props ──────────────────────────────────────────────────────────────────

export interface SourceConfigProps {
  sourceType: StudioSourceType;
  value: SourceConfigValue;
  onChange: (value: SourceConfigValue) => void;
}

const sourcePanelClass =
  'knowledge-source-panel space-y-12px rounded-16px bg-[var(--color-bg-2)] p-14px shadow-[0_10px_30px_rgba(15,23,42,0.035)]';

const sourceTitleClass = 'text-13px font-700 text-[var(--color-text-1)]';

const sourceLabelClass = 'mb-5px block text-13px font-500 text-[var(--color-text-2)]';

const sourceInputClass =
  'knowledge-source-input rounded-12px border-transparent bg-[var(--color-fill-1)] transition-[background-color,border-color,box-shadow] hover:bg-[var(--color-fill-2)] focus-within:shadow-[0_0_0_3px_rgba(var(--primary-6),0.1)]';

const sourceButtonClass =
  'knowledge-source-button rounded-10px border-transparent bg-[var(--color-fill-1)] text-[var(--color-text-2)] hover:bg-[var(--color-fill-2)] hover:text-[var(--color-text-1)]';

const sourceNoteClass =
  'knowledge-source-note flex gap-8px rounded-12px bg-[var(--color-fill-1)] px-10px py-8px text-12px leading-relaxed text-[var(--color-text-2)] shadow-[inset_0_0_0_1px_rgba(0,0,0,0.035)]';

const segmentGroupClass = 'inline-flex gap-4px rounded-11px bg-[var(--color-fill-1)] p-4px';

const segmentButtonBaseClass =
  'rounded-8px border-none px-13px py-7px text-12px font-inherit cursor-pointer transition-[background-color,color,box-shadow] focus-visible:outline-none focus-visible:shadow-[0_0_0_3px_rgba(var(--primary-6),0.12)]';

const segmentButtonActiveClass = 'bg-[var(--color-bg-2)] font-600 text-primary-6 shadow-[0_2px_8px_rgba(var(--primary-6),0.12)]';

const segmentButtonIdleClass = 'bg-transparent text-[var(--color-text-2)] hover:bg-[var(--color-fill-2)] hover:text-[var(--color-text-1)]';

// ─── Component ──────────────────────────────────────────────────────────────

const SourceConfig: React.FC<SourceConfigProps> = ({ sourceType, value, onChange }) => {
  const { t } = useTranslation();
  const isDesktop = isDesktopShell();

  // ─── Shared updater ───────────────────────────────────────────────────────

  const update = useCallback(
    (patch: Partial<SourceConfigValue>) => {
      onChange({ ...value, ...patch });
    },
    [value, onChange],
  );

  // ─── Blank ────────────────────────────────────────────────────────────────

  if (sourceType === 'blank') {
    return (
      <div className={sourcePanelClass}>
        <div className={sourceTitleClass}>
          {t('knowledge.studio.srcTitleBlank', { defaultValue: '来源 · 空白知识库' })}
        </div>
        <div className={sourceNoteClass}>
          <Info theme='outline' size='14' className='mt-2px flex-none text-[var(--color-text-3)]' />
          <div>
            {t('knowledge.studio.blankNote', {
              defaultValue:
                '系统会自动创建一个由应用托管的 Markdown 目录。无需更多配置 —— 创建后把 .md 文件放进去，或让 AI 自动生成梗概与 README。',
            })}
          </div>
        </div>
      </div>
    );
  }

  // ─── Local ────────────────────────────────────────────────────────────────

  if (sourceType === 'local') {
    const handleBrowseFolder = async () => {
      if (!isDesktop) return;
      try {
        const files = await ipcBridge.dialog.showOpen.invoke({ properties: ['openDirectory'] });
        if (files?.[0]) {
          update({ rootPath: files[0] });
        }
      } catch (e) {
        Message.error(String(e));
      }
    };

    return (
      <div className={sourcePanelClass}>
        <div className={sourceTitleClass}>
          {t('knowledge.studio.srcTitleLocal', { defaultValue: '来源 · 本地文件夹' })}
        </div>
        <div>
          <label className={sourceLabelClass}>
            {t('knowledge.studio.localFolderPath', { defaultValue: '文件夹路径' })}
          </label>
          <div className='flex gap-9px'>
            <Input
              className={`${sourceInputClass} flex-1`}
              placeholder={t('knowledge.studio.localFolderPlaceholder', { defaultValue: '选择电脑上一个已有目录' })}
              value={value.rootPath ?? ''}
              onChange={(v) => update({ rootPath: v })}
              readOnly={isDesktop}
            />
            {isDesktop && (
              <Button className={sourceButtonClass} onClick={() => void handleBrowseFolder()}>
                <FolderOpen theme='outline' size='14' className='mr-4px' />
                {t('knowledge.studio.localBrowse', { defaultValue: '选择文件夹' })}
              </Button>
            )}
          </div>
        </div>
        <div className={sourceNoteClass}>
          <Info theme='outline' size='14' className='mt-2px flex-none text-[var(--color-text-3)]' />
          <div>
            {t('knowledge.studio.localReadonlyNote', {
              defaultValue:
                '应用以只读引用方式接入，绝不改动你的目录结构。目录里 .md 的增删会自动反映到库里。',
            })}
          </div>
        </div>
      </div>
    );
  }

  // ─── Web ──────────────────────────────────────────────────────────────────

  if (sourceType === 'web') {
    const urlMode = value.urlMode ?? 'snapshot';
    const entries = value.urlEntries ?? [{ url: '', title: '' }];

    return (
      <div className={sourcePanelClass}>
        <div className={sourceTitleClass}>
          {t('knowledge.studio.srcTitleWeb', { defaultValue: '来源 · 网页 / URL' })}
        </div>

        {/* Crawl mode segment */}
        <div>
          <label className={sourceLabelClass}>
            {t('knowledge.studio.webCrawlMode', { defaultValue: '抓取模式' })}
          </label>
          <div className={segmentGroupClass}>
            <button
              type='button'
              className={`${segmentButtonBaseClass} ${urlMode === 'snapshot' ? segmentButtonActiveClass : segmentButtonIdleClass}`}
              onClick={() => update({ urlMode: 'snapshot' })}
            >
              {t('knowledge.studio.webSnapshot', { defaultValue: '快照（创建时抓取存档）' })}
            </button>
            <button
              type='button'
              className={`${segmentButtonBaseClass} ${urlMode === 'live' ? segmentButtonActiveClass : segmentButtonIdleClass}`}
              onClick={() => update({ urlMode: 'live' })}
            >
              {t('knowledge.studio.webRealtime', { defaultValue: '实时（运行时现查）' })}
            </button>
          </div>
          <div className='mt-6px text-11px text-[var(--color-text-3)]'>
            {t('knowledge.studio.webModeHint', {
              defaultValue:
                '快照：现在就抓取并存为本地文档，之后可随时刷新。实时：不抓取，会话运行时把这些网址作为实时来源查询。',
            })}
          </div>
        </div>

        {/* URL list */}
        <div>
          <label className={sourceLabelClass}>
            {t('knowledge.studio.webUrlList', { defaultValue: '网址列表' })}
            <span className='ml-6px font-400 text-[var(--color-text-3)]'>
              {t('knowledge.studio.webUrlMax', { defaultValue: '（最多 16 条）' })}
            </span>
          </label>
          <KnowledgeUrlEntriesEditor
            entries={entries}
            maxEntries={MAX_KNOWLEDGE_SOURCE_ENTRIES}
            onChange={(urlEntries) => update({ urlEntries })}
          />
        </div>

        {/* Browser render switch */}
        <div className='flex items-center gap-10px'>
          <Switch
            size='small'
            checked={value.browserRender ?? false}
            onChange={(checked) => update({ browserRender: checked })}
          />
          <span className='text-12px text-[var(--color-text-2)]'>
            {t('knowledge.studio.webBrowserRenderLabel', {
              defaultValue: '用真实浏览器渲染后抓取',
            })}
          </span>
          <span className='text-11px text-[var(--color-text-3)]'>
            {t('knowledge.studio.webBrowserRenderNote', {
              defaultValue: '适合 JS 渲染的单页应用',
            })}
          </span>
        </div>
      </div>
    );
  }

  // ─── Import ───────────────────────────────────────────────────────────────

  if (sourceType === 'import') {
    const handleBrowseZip = async () => {
      if (!isDesktop) return;
      try {
        const files = await ipcBridge.dialog.showOpen.invoke({
          properties: ['openFile'],
          filters: [{ name: 'Zip', extensions: ['zip'] }],
        });
        if (files?.[0]) {
          update({ importPath: files[0] });
        }
      } catch (e) {
        Message.error(String(e));
      }
    };

    return (
      <div className={sourcePanelClass}>
        <div className={sourceTitleClass}>
          {t('knowledge.studio.srcTitleImport', { defaultValue: '来源 · 导入 .zip 包' })}
        </div>
        <div>
          <label className={sourceLabelClass}>
            {t('knowledge.studio.importFile', { defaultValue: '知识库备份包' })}
          </label>
          <div className='flex gap-9px'>
            <Input
              className={`${sourceInputClass} flex-1`}
              placeholder={t('knowledge.studio.importPlaceholder', { defaultValue: '选择一个导出的 .zip 文件' })}
              value={value.importPath ?? ''}
              onChange={(v) => update({ importPath: v })}
              readOnly={isDesktop}
            />
            {isDesktop && (
              <Button className={sourceButtonClass} onClick={() => void handleBrowseZip()}>
                <FolderOpen theme='outline' size='14' className='mr-4px' />
                {t('knowledge.studio.importBrowse', { defaultValue: '选择文件' })}
              </Button>
            )}
          </div>
        </div>
        <div className={sourceNoteClass}>
          <Info theme='outline' size='14' className='mt-2px flex-none text-[var(--color-text-3)]' />
          <div>
            {t('knowledge.studio.importNote', {
              defaultValue:
                '从其它设备 / 库导出的 .zip 还原成一个新的托管库，导入后可继续编辑与挂载。',
            })}
          </div>
        </div>
      </div>
    );
  }

  return null;
};

export default SourceConfig;
