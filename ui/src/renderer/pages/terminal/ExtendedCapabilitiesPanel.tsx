/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useState } from 'react';
import { Select } from '@arco-design/web-react';
import { IconDown } from '@arco-design/web-react/icon';
import { useTranslation } from 'react-i18next';
import type { IKnowledgeBase } from '@/common/adapter/ipcBridge';
import PlatformMcpRegisterPanel from './PlatformMcpRegisterPanel';
import RegisterKnowledgeButton from './RegisterKnowledgeButton';
import { isDesktopShell } from '@/renderer/utils/platform';

interface ExtendedCapabilitiesPanelProps {
  cwd: string;
  command: string;
  /** Knowledge bases available to mount (empty → knowledge platform unavailable). */
  knowledgeBases: IKnowledgeBase[];
  /** Currently-selected (mounted) knowledge base ids. */
  kbIds: string[];
  onKbIdsChange: (ids: string[]) => void;
}

/**
 * Optional knowledge integration for a terminal launch. Terminal sessions do
 * not own Agent automation; this panel only mounts Knowledge and exposes the
 * credential-free CLI registration path.
 */
const ExtendedCapabilitiesPanel: React.FC<ExtendedCapabilitiesPanelProps> = ({
  cwd,
  command,
  knowledgeBases,
  kbIds,
  onKbIdsChange,
}) => {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);

  const hasKnowledge = knowledgeBases.length > 0;

  return (
    <div className='mt-20px rounded-12px b-1px b-solid border-arco-2 bg-fill-1'>
      {/* Collapsible header — this is an optional drawer, collapsed by default */}
      <button
        type='button'
        aria-expanded={expanded}
        onClick={() => setExpanded((e) => !e)}
        className='flex w-full cursor-pointer appearance-none items-center justify-between gap-12px b-none bg-transparent px-16px py-12px text-left'
      >
        <div className='min-w-0'>
          <div className='text-14px font-semibold text-t-primary'>
            {t('terminal.extended.title', { defaultValue: '知识库接入' })}
          </div>
          <div className='mt-2px text-12px text-t-tertiary'>
            {t('terminal.extended.subtitle', { defaultValue: '为该终端挂载知识库，或为外置 Agent CLI 注册无密钥连接。' })}
          </div>
        </div>
        <IconDown
          className={`shrink-0 text-14px text-t-tertiary transition-transform ${expanded ? 'rotate-180' : ''}`}
        />
      </button>

      {expanded && (
        <div className='px-16px pb-16px'>
          <div className='rounded-8px bg-fill-0 px-12px py-10px'>
            <div className='text-13px font-medium text-t-primary'>
              {t('terminal.extended.knowledgeLabel', { defaultValue: '平台知识库' })}
            </div>
            <div className='mt-2px text-12px leading-16px text-t-tertiary'>
              {t('terminal.extended.knowledgeDesc', {
                defaultValue: '挂载知识库到工作路径，供该终端的 Agent 检索。',
              })}
            </div>

            {hasKnowledge && (
              <Select
                className='mt-8px w-full'
                mode='multiple'
                allowClear
                placeholder={t('terminal.create.knowledgePlaceholder')}
                value={kbIds}
                maxTagCount={3}
                options={knowledgeBases.map((b) => ({ label: b.name, value: b.knowledge_base_id }))}
                onChange={(v) => onKbIdsChange(v as string[])}
              />
            )}
            {/* External-CLI registration is a desktop-host operation (writes
                agent CLI configs on the backend machine); in WebUI browser
                mode the note + button + template panel all disappear
                (audit 2026-07-30, finding I). */}
            {isDesktopShell() && (
              <>
                <div className='mt-8px flex items-start justify-between gap-12px'>
                  <div className='min-w-0 flex-1 text-12px leading-16px text-t-tertiary'>
                    {t('terminal.extended.knowledgeConnectNote', {
                      defaultValue:
                        '平台终端会自动注入；包装命令、自定义或外置终端可把无密钥命令注册到工作路径。',
                    })}
                  </div>
                  <div className='shrink-0'>
                    <RegisterKnowledgeButton cwd={cwd} command={command} />
                  </div>
                </div>
                <div className='mt-8px'>
                  <PlatformMcpRegisterPanel />
                </div>
              </>
            )}
          </div>
        </div>
      )}
    </div>
  );
};

export default ExtendedCapabilitiesPanel;
