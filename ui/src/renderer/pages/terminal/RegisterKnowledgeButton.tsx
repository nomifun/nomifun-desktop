/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useMemo, useState } from 'react';
import { Button, Message, Modal, Radio, Tooltip } from '@arco-design/web-react';
import { LinkCloud } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { ipcBridge } from '@/common';
import { detectFamily, type AgentFamily } from './detectFamily';

interface RegisterKnowledgeButtonProps {
  cwd: string;
  command: string;
  compact?: boolean;
}

const FAMILY_OPTIONS: Array<{ value: AgentFamily; label: string }> = [
  { value: 'claude', label: 'Claude' },
  { value: 'codex', label: 'Codex' },
  { value: 'gemini', label: 'Gemini' },
];

/**
 * Registers only the stable command. No port, token, capability, or broker
 * endpoint is written to the target config.
 */
const RegisterKnowledgeButton: React.FC<RegisterKnowledgeButtonProps> = ({ cwd, command, compact }) => {
  const { t } = useTranslation();
  const autoDetected = useMemo(() => detectFamily(command), [command]);
  const [family, setFamily] = useState<AgentFamily>(autoDetected ?? 'claude');
  const [loading, setLoading] = useState(false);
  const [visible, setVisible] = useState(false);

  React.useEffect(() => {
    if (autoDetected) setFamily(autoDetected);
  }, [autoDetected]);

  const handleConfirm = useCallback(async () => {
    if (!cwd) return;
    setLoading(true);
    try {
      const outcome = await ipcBridge.terminal.registerKnowledge.invoke({ cwd, family });
      Message.success({
        content: outcome.note ? `${outcome.written_path}\n${outcome.note}` : outcome.written_path,
        duration: 5000,
      });
      setVisible(false);
    } catch (error) {
      Message.error({
        content: error instanceof Error ? error.message : String(error),
        duration: 5000,
      });
    } finally {
      setLoading(false);
    }
  }, [cwd, family]);

  const cwdEmpty = !cwd;
  const openModal = () => {
    setFamily(autoDetected ?? 'claude');
    setVisible(true);
  };
  const modal = (
    <Modal
      title={t('terminal.registerKnowledge.modalTitle', { defaultValue: '接入平台知识库' })}
      visible={visible}
      onCancel={() => setVisible(false)}
      confirmLoading={loading}
      okText={t('terminal.registerKnowledge.confirm', { defaultValue: '接入' })}
      onOk={handleConfirm}
      autoFocus={false}
      focusLock
      unmountOnExit
    >
      <p className='mb-8px text-13px text-t-secondary'>
        {t('terminal.registerKnowledge.modalDesc', {
          defaultValue: '选择 CLI 类型，一键把知识库 MCP 命令合并到配置；现有配置不会被覆盖。',
        })}
      </p>
      <p className='mb-16px text-12px leading-18px text-t-tertiary'>
        {t('terminal.registerKnowledge.securityDesc', {
          defaultValue:
            '配置中不会保存端口或凭据。CLI 启动时通过当前系统用户专属的本地安全通道获取工作区权限；NomiFun 需要保持运行。',
        })}
      </p>
      <Radio.Group value={family} onChange={(value) => setFamily(value as AgentFamily)} direction='vertical'>
        {FAMILY_OPTIONS.map((option) => (
          <Radio key={option.value} value={option.value}>
            <span className='text-13px'>
              {option.label}
              <span className='ml-8px text-12px text-t-tertiary'>
                {option.value === 'codex'
                  ? t('terminal.registerKnowledge.codexScope', { defaultValue: '写入用户级配置' })
                  : t('terminal.registerKnowledge.projectScope', { defaultValue: '写入项目配置' })}
              </span>
            </span>
          </Radio>
        ))}
      </Radio.Group>
    </Modal>
  );

  const tooltip = cwdEmpty
    ? t('terminal.registerKnowledge.cwdRequired', { defaultValue: '请先选择工作目录' })
    : t('terminal.registerKnowledge.buttonCompact', { defaultValue: '接入知识库' });

  return (
    <>
      <Tooltip content={tooltip}>
        <Button
          size='small'
          type={compact ? 'secondary' : 'primary'}
          icon={<LinkCloud size='14' />}
          disabled={cwdEmpty}
          loading={loading}
          onClick={openModal}
        >
          {!compact && t('terminal.registerKnowledge.button', { defaultValue: '一键接入平台知识库' })}
        </Button>
      </Tooltip>
      {modal}
    </>
  );
};

export default RegisterKnowledgeButton;
