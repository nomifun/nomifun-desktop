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
  /** Working directory for the terminal session. */
  cwd: string;
  /** The launch command string (used for auto-detecting the agent family). */
  command: string;
  /** Compact mode for inline header usage (icon-only button). */
  compact?: boolean;
}

const FAMILY_OPTIONS: { value: AgentFamily; label: string; note: string }[] = [
  { value: 'claude', label: 'Claude', note: 'claude' },
  { value: 'codex', label: 'Codex', note: 'codex' },
  { value: 'gemini', label: 'Gemini', note: 'gemini' },
];

/**
 * One-click button that registers the platform knowledge MCP into the working
 * path's CLI auto-discovery config. Clicking opens a modal where the user picks
 * the CLI family, then confirms to register.
 */
const RegisterKnowledgeButton: React.FC<RegisterKnowledgeButtonProps> = ({ cwd, command, compact }) => {
  const { t } = useTranslation();
  const autoDetected = useMemo(() => detectFamily(command), [command]);
  const [family, setFamily] = useState<AgentFamily>(autoDetected ?? 'claude');
  const [loading, setLoading] = useState(false);
  const [visible, setVisible] = useState(false);

  // Sync the family selection when command changes and detection succeeds.
  React.useEffect(() => {
    if (autoDetected) setFamily(autoDetected);
  }, [autoDetected]);

  const handleConfirm = useCallback(async () => {
    if (!cwd) return;
    setLoading(true);
    try {
      const outcome = await ipcBridge.terminal.registerKnowledge.invoke({ cwd, family });
      const msg = outcome.note
        ? `${outcome.written_path}\n${outcome.note}`
        : outcome.written_path;
      Message.success({
        content: msg,
        duration: 5000,
      });
      setVisible(false);
    } catch (err) {
      Message.error({
        content: err instanceof Error ? err.message : String(err),
        duration: 4000,
      });
    } finally {
      setLoading(false);
    }
  }, [cwd, family]);

  const cwdEmpty = !cwd;
  const disabledTooltip = cwdEmpty
    ? t('terminal.registerKnowledge.cwdRequired', { defaultValue: '请先选择工作目录' })
    : undefined;

  const openModal = () => {
    // Reset family to auto-detected on each open
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
      <p className='mb-16px text-13px text-t-secondary'>
        {t('terminal.registerKnowledge.modalDesc', {
          defaultValue: '选择 CLI 类型，一键将知识检索 MCP 写入工作路径配置文件。',
        })}
      </p>
      <Radio.Group value={family} onChange={(v) => setFamily(v as AgentFamily)} direction='vertical'>
        {FAMILY_OPTIONS.map((opt) => (
          <Radio key={opt.value} value={opt.value}>
            <span className='text-13px'>
              {opt.label}
              <span className='ml-8px text-12px text-t-tertiary'>
                {t(`terminal.registerKnowledge.familyNote.${opt.note}` as 'terminal.registerKnowledge.cwdRequired', {
                  defaultValue:
                    opt.value === 'codex'
                      ? '写全局 ~/.codex/config.toml'
                      : '写项目配置文件',
                })}
              </span>
            </span>
          </Radio>
        ))}
      </Radio.Group>
    </Modal>
  );

  if (compact) {
    return (
      <>
        <Tooltip content={disabledTooltip || t('terminal.registerKnowledge.buttonCompact', { defaultValue: '接入知识库' })}>
          <Button
            size='small'
            type='secondary'
            icon={<LinkCloud size='14' />}
            disabled={cwdEmpty}
            loading={loading}
            onClick={openModal}
          />
        </Tooltip>
        {modal}
      </>
    );
  }

  return (
    <>
      <Tooltip content={disabledTooltip} disabled={!cwdEmpty}>
        <Button
          size='small'
          type='primary'
          status='default'
          icon={<LinkCloud size='14' />}
          disabled={cwdEmpty}
          loading={loading}
          onClick={openModal}
        >
          {t('terminal.registerKnowledge.button', { defaultValue: '一键接入平台知识库' })}
        </Button>
      </Tooltip>
      {modal}
    </>
  );
};

export default RegisterKnowledgeButton;
