/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Rename + delete for a solidified mini-app, shared by the library grid and the
 * full-page runner.
 *
 * Both surfaces offer exactly the same two mutations with the same copy and the
 * same confirmation; only what they do afterwards differs (the grid reloads, the
 * runner adopts the updated record or navigates away). So the modal, the form
 * state, the validation-vs-transport error split and the toasts live here once,
 * and the callers supply the two continuations.
 */

import React, { useCallback, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Form, Input, Modal } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import type { IApiMiniApp } from '@/common/adapter/ipcBridge';
import { useArcoMessage } from '@renderer/utils/ui/useArcoMessage';

export interface UseMiniAppMutationsOptions {
  /** Ran after a successful rename, with the record the backend echoed back. */
  onRenamed?: (app: IApiMiniApp) => void;
  /** Ran after a successful delete, with the record that is now gone. */
  onDeleted?: (app: IApiMiniApp) => void;
}

export interface MiniAppMutations {
  /**
   * Rename modal + Arco message holder. Render it once inside the host page —
   * without it neither the dialog nor the toasts appear.
   */
  node: React.ReactNode;
  /** Opens the rename dialog prefilled with the app's current name. */
  openRename: (app: IApiMiniApp) => void;
  /** Opens the destructive confirmation; deletes on confirm. */
  confirmDelete: (app: IApiMiniApp) => void;
}

export const useMiniAppMutations = ({ onRenamed, onDeleted }: UseMiniAppMutationsOptions = {}): MiniAppMutations => {
  const { t } = useTranslation();
  const [message, messageHolder] = useArcoMessage();
  const [form] = Form.useForm<{ name: string }>();
  const [renaming, setRenaming] = useState<IApiMiniApp | null>(null);
  const [savingRename, setSavingRename] = useState(false);

  const openRename = useCallback(
    (app: IApiMiniApp) => {
      setRenaming(app);
      form.resetFields();
      form.setFieldsValue({ name: app.name });
    },
    [form]
  );

  const submitRename = useCallback(async () => {
    if (!renaming) return;
    try {
      const values = await form.validate();
      setSavingRename(true);
      const updated = await ipcBridge.miniapps.update.invoke({
        miniapp_id: renaming.miniapp_id,
        updates: { name: values.name.trim() },
      });
      message.success(t('miniApps.rename.success'));
      setRenaming(null);
      onRenamed?.(updated);
    } catch (e) {
      // Form validation rejects with a field-error map (no `message`); ignore those.
      if (e instanceof Error) {
        message.error(e.message);
      }
    } finally {
      setSavingRename(false);
    }
  }, [renaming, form, message, t, onRenamed]);

  const confirmDelete = useCallback(
    (app: IApiMiniApp) => {
      Modal.confirm({
        title: t('miniApps.delete.confirmTitle'),
        content: t('miniApps.delete.confirmContent', { name: app.name }),
        okButtonProps: { status: 'danger' },
        onOk: async () => {
          try {
            await ipcBridge.miniapps.delete.invoke({ miniapp_id: app.miniapp_id });
            message.success(t('miniApps.delete.success'));
            onDeleted?.(app);
          } catch (e) {
            message.error(e instanceof Error ? e.message : String(e));
          }
        },
      });
    },
    [message, t, onDeleted]
  );

  const node = (
    <>
      {messageHolder}
      <Modal
        title={t('miniApps.rename.title')}
        visible={renaming !== null}
        confirmLoading={savingRename}
        onOk={() => void submitRename()}
        onCancel={() => setRenaming(null)}
        autoFocus={false}
        unmountOnExit
      >
        <Form form={form} layout='vertical'>
          <Form.Item
            label={t('miniApps.rename.label')}
            field='name'
            rules={[{ required: true, message: t('miniApps.rename.required') }]}
          >
            <Input
              placeholder={t('miniApps.rename.placeholder')}
              maxLength={100}
              showWordLimit
              onPressEnter={() => void submitRename()}
            />
          </Form.Item>
        </Form>
      </Modal>
    </>
  );

  return { node, openRename, confirmDelete };
};
