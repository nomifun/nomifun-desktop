/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { IApiMiniApp } from '@/common/adapter/ipcBridge';
import type { ConversationId } from '@/common/types/ids';
import { Button, Form, Input, Modal } from '@arco-design/web-react';
import React, { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

/** Result reported back so the host viewer can raise the right toast. */
export interface MiniAppSolidifyResult {
  mode: 'create' | 'update';
  name: string;
}

interface MiniAppSolidifyFields {
  name: string;
  description?: string;
  icon?: string;
}

export interface MiniAppSolidifyModalProps {
  visible: boolean;
  /** Freshest `miniapp.html` body, read by the host right before opening. */
  html: string;
  /** Prior save from THIS conversation, if any — enables the update path. */
  existing: IApiMiniApp | null;
  conversation_id?: ConversationId;
  /** Prefill for the name field (the conversation title when reachable). */
  defaultName: string;
  onCancel: () => void;
  onSaved: (result: MiniAppSolidifyResult) => void;
  onError: () => void;
}

/**
 * 「固化为小程序」表单。
 *
 * Solidify dialog: writes the conversation's `miniapp.html` into the mini-app
 * library. When this conversation already produced a mini-app the user first
 * picks between updating that record (HTML only — name/description/icon stay
 * whatever the library page shows) and saving a brand new one. The new one is a
 * detached copy: it records no source conversation, so the conversation keeps
 * pointing at exactly one updatable row.
 */
const MiniAppSolidifyModal: React.FC<MiniAppSolidifyModalProps> = ({
  visible,
  html,
  existing,
  conversation_id,
  defaultName,
  onCancel,
  onSaved,
  onError,
}) => {
  const { t } = useTranslation();
  const [form] = Form.useForm<MiniAppSolidifyFields>();
  const [saving, setSaving] = useState(false);
  // With a prior save we open on the choice step; otherwise straight to the form.
  const [step, setStep] = useState<'choice' | 'form'>(existing ? 'choice' : 'form');

  useEffect(() => {
    if (!visible) return;
    setStep(existing ? 'choice' : 'form');
    setSaving(false);
    form.resetFields();
    form.setFieldsValue({ name: defaultName, description: '', icon: '' });
  }, [visible, existing, defaultName, form]);

  const handleUpdateExisting = useCallback(async () => {
    if (!existing) return;
    setSaving(true);
    try {
      await ipcBridge.miniapps.update.invoke({
        miniapp_id: existing.miniapp_id,
        updates: { html },
      });
      onSaved({ mode: 'update', name: existing.name });
    } catch (error) {
      console.error('[MiniAppSolidifyModal] Failed to update mini-app:', error);
      onError();
    } finally {
      setSaving(false);
    }
  }, [existing, html, onSaved, onError]);

  const handleCreate = useCallback(async () => {
    let values: MiniAppSolidifyFields;
    try {
      values = await form.validate();
    } catch {
      // Field-level validation errors are already rendered inside the form.
      return;
    }
    const name = values.name.trim();
    const description = values.description?.trim();
    const icon = values.icon?.trim();
    setSaving(true);
    try {
      // Provenance is recorded only by the FIRST solidify of a conversation.
      // A fork ("save as new" while `existing` is set) is deliberately unlinked:
      // stamping the same `source_conversation_id` on a second row would make the
      // "update the existing one" lookup above ambiguous, and the next update
      // would target whichever row the list happened to return first.
      const isFork = existing !== null;
      await ipcBridge.miniapps.create.invoke({
        name,
        ...(description ? { description } : {}),
        ...(icon ? { icon } : {}),
        html,
        ...(conversation_id && !isFork ? { source_conversation_id: conversation_id } : {}),
      });
      onSaved({ mode: 'create', name });
    } catch (error) {
      console.error('[MiniAppSolidifyModal] Failed to create mini-app:', error);
      onError();
    } finally {
      setSaving(false);
    }
  }, [form, html, conversation_id, existing, onSaved, onError]);

  return (
    <Modal
      title={t('miniApps.save.title')}
      visible={visible}
      onCancel={onCancel}
      autoFocus={false}
      unmountOnExit
      footer={
        step === 'choice' ? (
          <div className='flex items-center justify-end gap-8px'>
            <Button onClick={onCancel}>{t('miniApps.save.cancel')}</Button>
            <Button onClick={() => setStep('form')}>{t('miniApps.save.saveAsNew')}</Button>
            <Button type='primary' loading={saving} onClick={() => void handleUpdateExisting()}>
              {t('miniApps.save.updateExisting', { name: existing?.name ?? '' })}
            </Button>
          </div>
        ) : (
          <div className='flex items-center justify-end gap-8px'>
            <Button onClick={onCancel}>{t('miniApps.save.cancel')}</Button>
            <Button type='primary' loading={saving} onClick={() => void handleCreate()}>
              {t('miniApps.save.confirm')}
            </Button>
          </div>
        )
      }
    >
      {step === 'choice' ? (
        <div className='text-13px text-t-secondary'>{t('miniApps.save.existingHint')}</div>
      ) : (
        <Form form={form} layout='vertical'>
          <Form.Item
            label={t('miniApps.save.nameLabel')}
            field='name'
            rules={[{ required: true, message: t('miniApps.save.nameRequired') }]}
          >
            <Input
              placeholder={t('miniApps.save.namePlaceholder')}
              maxLength={100}
              showWordLimit
              onPressEnter={() => void handleCreate()}
            />
          </Form.Item>
          <Form.Item label={t('miniApps.save.descriptionLabel')} field='description'>
            <Input placeholder={t('miniApps.save.descriptionPlaceholder')} maxLength={200} />
          </Form.Item>
          <Form.Item label={t('miniApps.save.iconLabel')} field='icon'>
            <Input placeholder={t('miniApps.save.iconPlaceholder')} maxLength={8} />
          </Form.Item>
        </Form>
      )}
    </Modal>
  );
};

export default MiniAppSolidifyModal;
