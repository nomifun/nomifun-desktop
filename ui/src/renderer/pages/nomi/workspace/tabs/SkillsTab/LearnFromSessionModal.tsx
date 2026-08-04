/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Input, Message, Modal } from '@arco-design/web-react';
import { tryParseEntityId } from '@/common/types/ids';

interface LearnFromSessionModalProps {
  visible: boolean;
  onClose: () => void;
  /** Resolves true when a draft skill was produced. */
  onSubmit: (conversationId: string) => Promise<boolean>;
}

/**
 * 从会话学习 — a one-shot action (paste a session id, get a draft), so it stays a
 * modal rather than occupying the detail pane.
 */
const LearnFromSessionModal: React.FC<LearnFromSessionModalProps> = ({ visible, onClose, onSubmit }) => {
  const { t } = useTranslation();
  const [conversationId, setConversationId] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const trimmed = conversationId.trim();
  // The backend takes a canonical id and the parser throws on anything else, so
  // validate here rather than letting a typo surface as a raw parse error.
  const malformed = trimmed.length > 0 && tryParseEntityId('conversation', trimmed) === null;

  const close = () => {
    setConversationId('');
    onClose();
  };

  const submit = async () => {
    if (!trimmed || malformed) return;
    setSubmitting(true);
    try {
      const drafted = await onSubmit(conversationId);
      Message.success(
        drafted
          ? t('nomi.skills.learnedFromSessionOk', {
              defaultValue: '已根据这段会话起草技能，去列表里审阅',
            })
          : t('nomi.skills.taughtNone', { defaultValue: '没能从这个会话提炼出技能' })
      );
      close();
    } catch (error) {
      Message.error(String(error));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Modal
      title={t('nomi.skills.learnFromSession', { defaultValue: '从会话学习' })}
      visible={visible}
      onOk={() => void submit()}
      onCancel={close}
      confirmLoading={submitting}
      okButtonProps={{ disabled: !trimmed || malformed }}
      okText={t('nomi.skills.learnFromSessionOk', { defaultValue: '开始学习' })}
      cancelText={t('nomi.skills.cancel', { defaultValue: '取消' })}
    >
      <div className='flex flex-col gap-8px'>
        <span className='text-12px leading-18px text-t-secondary'>
          {t('nomi.skills.learnFromSessionHint', {
            defaultValue: '把一段已完成多步操作的会话 ID 填进来，伙伴会从里面提炼出一个草稿技能，等你审阅。',
          })}
        </span>
        <Input
          value={conversationId}
          onChange={setConversationId}
          placeholder={t('nomi.skills.teachPlaceholder', { defaultValue: '会话 ID' })}
        />
        {malformed && (
          <span className='text-12px leading-18px text-[rgb(var(--danger-6))]'>
            {t('nomi.skills.learnFromSessionInvalid', {
              defaultValue: '这不像一个会话 ID，请从会话地址栏里复制完整的 ID。',
            })}
          </span>
        )}
      </div>
    </Modal>
  );
};

export default LearnFromSessionModal;
