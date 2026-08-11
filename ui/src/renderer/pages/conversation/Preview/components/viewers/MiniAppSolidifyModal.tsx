/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { IApiMiniApp } from '@/common/adapter/ipcBridge';
import type { ConversationId, MiniAppId } from '@/common/types/ids';
import { Button, Form, Input, Modal, Select } from '@arco-design/web-react';
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';

/** Result reported back so the host viewer can raise the right toast. */
export interface MiniAppPublishResult {
  mode: 'create' | 'replace';
  name: string;
}

interface MiniAppPublishFields {
  name: string;
  description?: string;
  icon?: string;
}

export interface MiniAppSolidifyModalProps {
  visible: boolean;
  /** Freshest `miniapp.html` body, read by the host right before opening. */
  html: string;
  /** The owner's whole library — every row is a legal replace target. */
  apps: IApiMiniApp[];
  /**
   * Row this conversation previously published, if any. Only the picker's DEFAULT
   * selection: the user may replace any app they own, and `source_conversation_id`
   * is provenance rather than a binding.
   */
  defaultTargetId: MiniAppId | null;
  conversation_id?: ConversationId;
  /** Prefill for the name field (the conversation title when reachable). */
  defaultName: string;
  onCancel: () => void;
  onSaved: (result: MiniAppPublishResult) => void;
  onError: () => void;
}

/**
 * 「发布为小程序」表单。
 *
 * Publishes the conversation's `miniapp.html` into the mini-app library, in one of
 * two shapes (spec D20):
 *
 *  - **发布为新的小程序** — a new row, named here. Provenance is stamped only when
 *    this conversation has no row yet, so the default replace target stays
 *    unambiguous.
 *  - **替换已有小程序** — an explicit target the user PICKS from their library. It is
 *    a plain `update` carrying `html`, which the backend defines as writing the
 *    snapshot AND resyncing the on-disk working copy — without that second half
 *    the app would immediately read as 「有未发布改动」 against the copy it just
 *    replaced.
 *
 * The user is never asked to guess which app they are overwriting: with an empty
 * library the dialog opens straight on the create form.
 */
const MiniAppSolidifyModal: React.FC<MiniAppSolidifyModalProps> = ({
  visible,
  html,
  apps,
  defaultTargetId,
  conversation_id,
  defaultName,
  onCancel,
  onSaved,
  onError,
}) => {
  const { t } = useTranslation();
  const [form] = Form.useForm<MiniAppPublishFields>();
  const [saving, setSaving] = useState(false);
  // With a library to replace into we open on the choice step; otherwise there is
  // nothing to choose and the form is the whole dialog.
  const [step, setStep] = useState<'choice' | 'form'>(apps.length > 0 ? 'choice' : 'form');
  const [targetId, setTargetId] = useState<MiniAppId | null>(defaultTargetId);

  /** Does this conversation already own a row? Decides whether to stamp provenance. */
  const hasOwnRow = defaultTargetId !== null;

  const options = useMemo(() => apps.map((app) => ({ label: app.name, value: app.miniapp_id })), [apps]);

  useEffect(() => {
    if (!visible) return;
    setStep(apps.length > 0 ? 'choice' : 'form');
    // This conversation's own app when it has one, else the first row — never an
    // empty picker under a live 「替换已有小程序」 button.
    setTargetId(defaultTargetId ?? apps[0]?.miniapp_id ?? null);
    setSaving(false);
    form.resetFields();
    form.setFieldsValue({ name: defaultName, description: '', icon: '' });
  }, [visible, apps, defaultTargetId, defaultName, form]);

  const handleReplace = useCallback(async () => {
    const target = apps.find((app) => app.miniapp_id === targetId);
    if (!target) return;
    setSaving(true);
    try {
      // `html` alone: name, description and icon belong to the library page, and
      // the backend resyncs the working copy from this same body.
      await ipcBridge.miniapps.update.invoke({
        miniapp_id: target.miniapp_id,
        updates: { html },
      });
      onSaved({ mode: 'replace', name: target.name });
    } catch (error) {
      console.error('[MiniAppSolidifyModal] Failed to replace the mini-app:', error);
      onError();
    } finally {
      setSaving(false);
    }
  }, [apps, targetId, html, onSaved, onError]);

  const handleCreate = useCallback(async () => {
    let values: MiniAppPublishFields;
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
      // Provenance is recorded only by the FIRST publish of a conversation. A
      // second new row is deliberately unlinked: stamping the same
      // `source_conversation_id` twice would make the picker's default ambiguous,
      // and it would then land on whichever row the list happened to return first.
      await ipcBridge.miniapps.create.invoke({
        name,
        ...(description ? { description } : {}),
        ...(icon ? { icon } : {}),
        html,
        ...(conversation_id && !hasOwnRow ? { source_conversation_id: conversation_id } : {}),
      });
      onSaved({ mode: 'create', name });
    } catch (error) {
      console.error('[MiniAppSolidifyModal] Failed to create the mini-app:', error);
      onError();
    } finally {
      setSaving(false);
    }
  }, [form, html, conversation_id, hasOwnRow, onSaved, onError]);

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
            <Button onClick={() => setStep('form')}>{t('miniApps.save.publishAsNew')}</Button>
            <Button type='primary' loading={saving} disabled={targetId === null} onClick={() => void handleReplace()}>
              {t('miniApps.save.replaceExisting')}
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
        <div className='flex flex-col gap-10px'>
          <div className='text-13px text-t-secondary'>{t('miniApps.save.replaceHint')}</div>
          <div className='flex flex-col gap-6px'>
            <span className='text-12px text-t-tertiary'>{t('miniApps.save.replaceLabel')}</span>
            <Select
              value={targetId ?? undefined}
              options={options}
              onChange={(value) => setTargetId(value as MiniAppId)}
              placeholder={t('miniApps.save.replaceLabel')}
            />
          </div>
        </div>
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
