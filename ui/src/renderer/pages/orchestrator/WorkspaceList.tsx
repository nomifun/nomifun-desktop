/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Form, Input, Select, Spin } from '@arco-design/web-react';
import { Plus, FolderClose, Funds } from '@icon-park/react';
import classNames from 'classnames';
import { ipcBridge } from '@/common';
import type { TFleet, TOrchWorkspace } from '@/common/types/orchestrator/orchestratorTypes';
import NomiModal from '@/renderer/components/base/NomiModal';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import { useFleets, useWorkspaces } from './useOrchestratorData';

type CreateFormValues = {
  name: string;
  default_fleet_id?: string;
};

/** New-workspace modal: name (required) + optional default fleet. */
const CreateWorkspaceModal: React.FC<{
  visible: boolean;
  fleets: TFleet[];
  onClose: () => void;
  onCreated: () => void;
}> = ({ visible, fleets, onClose, onCreated }) => {
  const { t } = useTranslation();
  const [form] = Form.useForm<CreateFormValues>();
  const [message, ctx] = useArcoMessage();
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    if (visible) {
      form.resetFields();
    }
  }, [visible, form]);

  const handleSubmit = async () => {
    try {
      const values = await form.validate();
      setSubmitting(true);
      await ipcBridge.orchestrator.workspaces.create.invoke({
        name: values.name.trim(),
        default_fleet_id: values.default_fleet_id || undefined,
      });
      message.success(t('orchestrator.workspace.modal.createOk'));
      onCreated();
      onClose();
    } catch (e) {
      // form.validate() rejects with a field-errors map (no message) on
      // validation failure; only surface real backend errors as a toast.
      if (e instanceof Error || typeof e === 'string') {
        message.error(t('orchestrator.workspace.modal.createError', { error: String(e) }));
      }
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <>
      {ctx}
      <NomiModal
        visible={visible}
        size='small'
        header={t('orchestrator.workspace.modal.title')}
        onCancel={onClose}
        onOk={() => void handleSubmit()}
        confirmLoading={submitting}
        cancelText={t('orchestrator.workspace.modal.cancel')}
        okText={t('orchestrator.workspace.modal.confirm')}
        contentStyle={{ padding: '20px 24px 4px' }}
      >
        <Form form={form} layout='vertical'>
          <Form.Item
            field='name'
            label={t('orchestrator.workspace.modal.nameLabel')}
            rules={[{ required: true, message: t('orchestrator.workspace.modal.nameRequired') }]}
          >
            <Input placeholder={t('orchestrator.workspace.modal.namePlaceholder')} allowClear autoFocus />
          </Form.Item>
          <Form.Item field='default_fleet_id' label={t('orchestrator.workspace.modal.defaultFleetLabel')}>
            <Select
              placeholder={t('orchestrator.workspace.modal.defaultFleetPlaceholder')}
              allowClear
              options={fleets.map((f) => ({ label: f.name, value: f.id }))}
            />
          </Form.Item>
        </Form>
      </NomiModal>
    </>
  );
};

/** A single workspace row/card. */
const WorkspaceCard: React.FC<{ workspace: TOrchWorkspace; fleetName?: string }> = ({ workspace, fleetName }) => {
  const { t } = useTranslation();
  return (
    <div className='rd-12px bg-1 px-16px py-14px flex items-center gap-12px transition-colors hover:bg-fill-1'>
      <span className='size-36px rd-10px bg-primary-1 text-primary-6 flex items-center justify-center shrink-0'>
        <FolderClose theme='outline' size='18' strokeWidth={3} />
      </span>
      <div className='min-w-0 flex-1'>
        <div className='text-14px font-600 text-t-primary truncate leading-tight'>{workspace.name}</div>
        <div className='mt-3px text-12px text-t-tertiary flex items-center gap-4px truncate'>
          <Funds theme='outline' size='13' strokeWidth={3} />
          <span className='truncate'>{fleetName ?? t('orchestrator.workspace.noDefaultFleet')}</span>
        </div>
      </div>
    </div>
  );
};

/**
 * WorkspaceList — the 「工作间」section of the orchestration page. Lists the
 * persisted orchestration workspaces as cards and offers a「新建工作间」action
 * that opens a small NomiModal (name + optional default fleet) wired to
 * `ipcBridge.orchestrator.workspaces.create` and revalidates via SWR `mutate`.
 */
const WorkspaceList: React.FC = () => {
  const { t } = useTranslation();
  const { data: workspaces, isLoading, error, mutate } = useWorkspaces();
  const { data: fleets } = useFleets();
  const [createOpen, setCreateOpen] = useState(false);

  const fleetList = fleets ?? [];
  const fleetNameById = useCallback(
    (id?: string) => (id ? fleetList.find((f) => f.id === id)?.name : undefined),
    [fleetList]
  );

  const list = workspaces ?? [];

  return (
    <div className='w-full'>
      <div className='flex items-center justify-between gap-12px mb-16px'>
        <div className='min-w-0'>
          <div className='text-18px font-600 text-t-primary leading-tight'>{t('orchestrator.workspace.title')}</div>
          <div className='mt-4px text-12px leading-16px text-t-tertiary'>{t('orchestrator.workspace.subtitle')}</div>
        </div>
        <div
          role='button'
          tabIndex={0}
          onClick={() => setCreateOpen(true)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault();
              setCreateOpen(true);
            }
          }}
          className={classNames(
            'shrink-0 h-34px px-14px rd-8px flex items-center gap-6px cursor-pointer select-none',
            'bg-primary-6 text-white hover:opacity-90 active:opacity-80 transition-opacity'
          )}
        >
          <Plus theme='outline' size='15' strokeWidth={4} />
          <span className='text-13px font-500'>{t('orchestrator.workspace.create')}</span>
        </div>
      </div>

      {isLoading ? (
        <div className='py-48px flex items-center justify-center'>
          <Spin />
        </div>
      ) : error ? (
        <div className='py-48px text-center text-13px text-t-tertiary'>{t('orchestrator.workspace.loadError')}</div>
      ) : list.length === 0 ? (
        <div className='py-48px text-center text-13px text-t-tertiary'>{t('orchestrator.workspace.empty')}</div>
      ) : (
        <div className='grid gap-10px' style={{ gridTemplateColumns: 'repeat(auto-fill, minmax(min(260px, 100%), 1fr))' }}>
          {list.map((ws) => (
            <WorkspaceCard key={ws.id} workspace={ws} fleetName={fleetNameById(ws.default_fleet_id)} />
          ))}
        </div>
      )}

      <CreateWorkspaceModal
        visible={createOpen}
        fleets={fleetList}
        onClose={() => setCreateOpen(false)}
        onCreated={() => void mutate()}
      />
    </div>
  );
};

export default WorkspaceList;
