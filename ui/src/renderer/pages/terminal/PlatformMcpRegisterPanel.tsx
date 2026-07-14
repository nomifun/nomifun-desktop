/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ipcBridge } from '@/common';
import type { IMcpRegisterTemplate } from '@/common/adapter/ipcBridge';
import CopyIconButton from '@/renderer/components/base/CopyIconButton';
import NomiCollapse from '@/renderer/components/base/NomiCollapse';
import { isWindows } from '@/renderer/utils/platform';

const PlatformMcpRegisterPanel: React.FC = () => {
  const { t } = useTranslation();
  const [template, setTemplate] = useState<IMcpRegisterTemplate | null>(null);
  const [error, setError] = useState(false);
  const [fetched, setFetched] = useState(false);

  const fetchTemplate = useCallback(() => {
    if (fetched) return;
    setFetched(true);
    ipcBridge.terminal.mcpRegisterTemplate
      .invoke()
      .then(setTemplate)
      .catch(() => setError(true));
  }, [fetched]);

  return (
    <NomiCollapse
      onChange={(keys) => {
        if (keys.includes('mcp-register')) fetchTemplate();
      }}
      bordered={false}
    >
      <NomiCollapse.Item
        name='mcp-register'
        header={
          <span className='text-13px text-t-tertiary'>
            {t('terminal.create.mcpPanel.title', { defaultValue: '高级：手动注册到其它 CLI / 外置终端' })}
          </span>
        }
        contentClassName='pt-0'
      >
        <p className='mb-12px text-12px text-t-secondary'>
          {t('terminal.create.mcpPanel.description', {
            defaultValue:
              '模板只包含启动命令，不包含端口或凭据。外部 CLI 启动时通过当前系统用户专属的本地安全通道获取工作区权限；NomiFun 需要保持运行。',
          })}
        </p>
        {error && (
          <p className='text-12px text-danger-6'>
            {t('terminal.create.mcpPanel.unavailable', { defaultValue: '平台能力 API 暂不可用' })}
          </p>
        )}
        {template && (
          <div className='flex flex-col gap-12px'>
            <TemplateBlock
              label={t(
                isWindows()
                  ? 'terminal.create.mcpPanel.claudePowerShellCommand'
                  : 'terminal.create.mcpPanel.claudeCommand'
              )}
              code={template.claude_cmd}
            />
            <TemplateBlock label={t('terminal.create.mcpPanel.claudeJson')} code={template.claude_json} />
            <TemplateBlock label={t('terminal.create.mcpPanel.codexToml')} code={template.codex_toml} />
            <TemplateBlock label={t('terminal.create.mcpPanel.geminiJson')} code={template.gemini_json} />
          </div>
        )}
        {!template && !error && fetched && (
          <p className='text-12px text-t-tertiary'>
            {t('terminal.create.mcpPanel.loading', { defaultValue: '加载中...' })}
          </p>
        )}
      </NomiCollapse.Item>
    </NomiCollapse>
  );
};

const TemplateBlock: React.FC<{ label: string; code: string }> = ({ label, code }) => (
  <div>
    <div className='mb-4px flex items-center justify-between'>
      <span className='text-12px font-medium text-t-secondary'>{label}</span>
      <CopyIconButton text={code} size={14} />
    </div>
    <pre className='m-0 overflow-x-auto whitespace-pre-wrap break-all rounded-8px bg-fill-2 px-12px py-8px font-mono text-12px text-t-primary'>
      {code}
    </pre>
  </div>
);

export default PlatformMcpRegisterPanel;
