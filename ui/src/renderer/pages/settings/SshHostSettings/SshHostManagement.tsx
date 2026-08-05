/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IApiSshHost } from '@/common/adapter/ipcBridge';
import { ipcBridge } from '@/common';
import { useCallback, useState } from 'react';
import { Button, Form, Input, InputNumber, Message, Modal, Select } from '@arco-design/web-react';
import NomiModal from '@renderer/components/base/NomiModal';
import { Certificate, Edit, Fingerprint, Key, Lock, Plus, Server, Speed } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import useSWR from 'swr';
import {
  buildUpdatePayload,
  canTestConnection,
  validateSshHostForm,
  type SshAuthType,
  type SshHostFormValues,
} from './sshHostForm.validation';

const FormItem = Form.Item;

const AUTH_ICON: Record<SshAuthType, React.ReactNode> = {
  password: <Lock theme='outline' size='14' />,
  key: <Key theme='outline' size='14' />,
  certificate: <Certificate theme='outline' size='14' />,
  agent: <Fingerprint theme='outline' size='14' />,
};

// ── Add / edit form modal ───────────────────────────────────────────────

interface FormModalProps {
  visible: boolean;
  editHost?: IApiSshHost;
  onClose: () => void;
  onSaved: () => void;
}

const SshHostFormModal: React.FC<FormModalProps> = ({ visible, editHost, onClose, onSaved }) => {
  const { t } = useTranslation();
  const [form] = Form.useForm<SshHostFormValues>();
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);

  const isEdit = Boolean(editHost);

  const currentValues = (): SshHostFormValues => {
    const v = form.getFieldsValue() as Partial<SshHostFormValues>;
    return {
      name: v.name ?? '',
      host: v.host ?? '',
      port: typeof v.port === 'number' ? v.port : 22,
      username: v.username ?? '',
      authType: (v.authType as SshAuthType) ?? 'password',
      password: v.password ?? null,
      privateKey: v.privateKey ?? null,
      passphrase: v.passphrase ?? null,
      certificate: v.certificate ?? null,
      sudoPassword: v.sudoPassword ?? null,
    };
  };

  const handleSave = useCallback(async () => {
    const values = currentValues();
    const errorKey = validateSshHostForm(values, isEdit);
    if (errorKey) {
      Message.warning(t(errorKey));
      return;
    }
    setSaving(true);
    try {
      if (editHost) {
        const updates = buildUpdatePayload(values);
        await ipcBridge.ssh.update.invoke({
          ssh_host_id: editHost.sshHostId,
          updates: {
            name: updates.name,
            host: updates.host,
            port: updates.port,
            username: updates.username,
            authType: updates.authType,
            password: updates.password ?? undefined,
            privateKey: updates.privateKey ?? undefined,
            passphrase: updates.passphrase ?? undefined,
            certificate: updates.certificate ?? undefined,
            sudoPassword: updates.sudoPassword ?? undefined,
          },
        });
      } else {
        await ipcBridge.ssh.create.invoke({
          name: values.name,
          host: values.host,
          port: values.port,
          username: values.username,
          authType: values.authType,
          password: values.password ?? undefined,
          privateKey: values.privateKey ?? undefined,
          passphrase: values.passphrase ?? undefined,
          certificate: values.certificate ?? undefined,
          sudoPassword: values.sudoPassword ?? undefined,
        });
      }
      onSaved();
      onClose();
    } catch (e) {
      Message.error(String(e));
    } finally {
      setSaving(false);
    }
  }, [editHost, isEdit, onClose, onSaved, t]);

  const handleTest = useCallback(async () => {
    const values = currentValues();
    if (!canTestConnection(values, isEdit)) {
      const errorKey = validateSshHostForm(values, isEdit);
      if (errorKey) Message.warning(t(errorKey));
      return;
    }
    if (!editHost) {
      // Test-connection probes a saved host; save first for a new host.
      Message.info(t('ssh.form.save'));
      return;
    }
    setTesting(true);
    try {
      const res = await ipcBridge.ssh.testConnection.invoke({ ssh_host_id: editHost.sshHostId });
      if (res.ok) Message.success(t('ssh.test.ok'));
      else Message.error(t('ssh.test.failed', { message: res.message }));
    } catch (e) {
      Message.error(t('ssh.test.failed', { message: String(e) }));
    } finally {
      setTesting(false);
    }
  }, [editHost, isEdit, t]);

  return (
    <NomiModal
      visible={visible}
      onCancel={onClose}
      header={{
        title: editHost ? t('ssh.form.editTitle') : t('ssh.form.addTitle'),
        showClose: true,
      }}
      style={{ maxWidth: '92vw', borderRadius: 16 }}
      contentStyle={{ background: 'var(--dialog-fill-0)', borderRadius: 16, padding: '20px 24px 16px', overflow: 'auto' }}
      okText={t('ssh.form.save')}
      cancelText={t('ssh.form.cancel')}
      onOk={handleSave}
      confirmLoading={saving}
      afterOpen={() => {
        form.setFieldsValue({
          name: editHost?.name ?? '',
          host: editHost?.host ?? '',
          port: editHost?.port ?? 22,
          username: editHost?.username ?? '',
          authType: (editHost?.authType as SshAuthType) ?? 'password',
          // Masked sentinels: the server returns '***' for stored secrets.
          password: editHost?.password ?? undefined,
          privateKey: editHost?.privateKey ?? undefined,
          passphrase: editHost?.passphrase ?? undefined,
          certificate: editHost?.certificate ?? undefined,
          sudoPassword: editHost?.sudoPassword ?? undefined,
        });
      }}
      afterClose={() => form.resetFields()}
    >
      <Form form={form} layout='vertical' style={{ width: 460, maxWidth: '100%' }}>
        <FormItem label={t('ssh.form.name')} field='name' rules={[{ required: true }]}>
          <Input placeholder='prod-web-01' />
        </FormItem>
        <div className='flex gap-8px'>
          <FormItem label={t('ssh.form.host')} field='host' rules={[{ required: true }]} className='flex-1'>
            <Input placeholder='10.0.3.21' />
          </FormItem>
          <FormItem label={t('ssh.form.port')} field='port' style={{ width: 110 }}>
            <InputNumber min={1} max={65535} placeholder='22' />
          </FormItem>
        </div>
        <FormItem label={t('ssh.form.username')} field='username' rules={[{ required: true }]}>
          <Input placeholder='deploy' />
        </FormItem>
        <FormItem label={t('ssh.form.authType')} field='authType' rules={[{ required: true }]}>
          <Select>
            <Select.Option value='password'>{t('ssh.form.authPassword')}</Select.Option>
            <Select.Option value='key'>{t('ssh.form.authKey')}</Select.Option>
            <Select.Option value='certificate'>{t('ssh.form.authCertificate')}</Select.Option>
            <Select.Option value='agent'>{t('ssh.form.authAgent')}</Select.Option>
          </Select>
        </FormItem>

        {/* Conditional credential fields per auth method */}
        <FormItem shouldUpdate noStyle>
          {(values: Partial<SshHostFormValues>) => {
            const auth = (values.authType as SshAuthType) ?? 'password';
            return (
              <>
                {auth === 'password' && (
                  <FormItem label={t('ssh.form.password')} field='password'>
                    <Input.Password
                      placeholder={isEdit ? t('ssh.form.secretKept') : ''}
                      autoComplete='new-password'
                    />
                  </FormItem>
                )}
                {(auth === 'key' || auth === 'certificate') && (
                  <>
                    <FormItem label={t('ssh.form.privateKey')} field='privateKey'>
                      <Input.TextArea
                        placeholder={t('ssh.form.privateKeyPlaceholder')}
                        autoSize={{ minRows: 3, maxRows: 8 }}
                        spellCheck={false}
                      />
                    </FormItem>
                    <FormItem label={t('ssh.form.passphrase')} field='passphrase'>
                      <Input.Password
                        placeholder={isEdit ? t('ssh.form.secretKept') : ''}
                        autoComplete='new-password'
                      />
                    </FormItem>
                  </>
                )}
                {auth === 'certificate' && (
                  <FormItem label={t('ssh.form.certificate')} field='certificate'>
                    <Input.TextArea autoSize={{ minRows: 2, maxRows: 5 }} spellCheck={false} />
                  </FormItem>
                )}
              </>
            );
          }}
        </FormItem>

        <FormItem
          label={t('ssh.form.sudoPassword')}
          field='sudoPassword'
          extra={<span className='text-12px text-t-tertiary'>{t('ssh.form.sudoHint')}</span>}
        >
          <Input.Password placeholder={isEdit ? t('ssh.form.secretKept') : ''} autoComplete='new-password' />
        </FormItem>

        {isEdit && (
          <Button long type='outline' icon={<Speed theme='outline' size='14' />} loading={testing} onClick={handleTest}>
            {t('ssh.form.test')}
          </Button>
        )}
      </Form>
    </NomiModal>
  );
};

// ── Host book panel ─────────────────────────────────────────────────────

const SshHostManagement: React.FC = () => {
  const { t } = useTranslation();
  const { data: hosts, mutate } = useSWR('ssh-hosts.list', () => ipcBridge.ssh.list.invoke());
  const [modalVisible, setModalVisible] = useState(false);
  const [editHost, setEditHost] = useState<IApiSshHost | undefined>();

  const openAdd = () => {
    setEditHost(undefined);
    setModalVisible(true);
  };
  const openEdit = (host: IApiSshHost) => {
    setEditHost(host);
    setModalVisible(true);
  };

  const handleDelete = useCallback(
    (host: IApiSshHost) => {
      Modal.confirm({
        title: t('ssh.delete.confirm'),
        okButtonProps: { status: 'danger' },
        okText: t('ssh.delete.ok'),
        cancelText: t('ssh.delete.cancel'),
        onOk: async () => {
          await ipcBridge.ssh.delete.invoke({ ssh_host_id: host.sshHostId });
          await mutate();
        },
      });
    },
    [t, mutate]
  );

  return (
    <div>
      <div className='mb-14px flex items-center justify-between gap-8px'>
        <div className='text-13px text-t-tertiary'>{t('ssh.description')}</div>
        <Button type='primary' icon={<Plus theme='outline' size='14' />} onClick={openAdd}>
          {t('ssh.empty.add')}
        </Button>
      </div>

      {!hosts || hosts.length === 0 ? (
        <div className='flex min-h-200px flex-col items-center justify-center gap-10px rd-12px border border-arco-2 bg-fill-0 px-24px py-28px text-center'>
          <span className='flex size-44px items-center justify-center rd-12px bg-brand-light text-brand'>
            <Server theme='outline' size='24' fill='currentColor' />
          </span>
          <div className='text-15px font-600 leading-22px text-t-primary'>{t('ssh.empty.title')}</div>
          <div className='mt-2px text-12px leading-18px text-t-tertiary'>{t('ssh.empty.description')}</div>
        </div>
      ) : (
        <div className='flex flex-col gap-8px'>
          {hosts.map((host) => (
            <div
              key={host.sshHostId}
              className='group flex items-center gap-10px rd-10px border border-arco-2 bg-fill-0 px-12px py-10px transition-colors hover:border-[var(--color-border-3)]'
            >
              <span className='flex size-30px shrink-0 items-center justify-center rd-8px bg-brand-light text-brand'>
                <Server theme='outline' size='16' fill='currentColor' />
              </span>
              <div className='min-w-0 flex-1'>
                <div className='truncate text-13px font-600 leading-19px text-t-primary'>{host.name}</div>
                <div className='mt-2px truncate font-mono text-12px leading-18px text-t-secondary'>
                  {host.username}@{host.host}:{host.port}
                </div>
                <div className='mt-3px flex items-center gap-6px text-11px leading-17px text-t-tertiary'>
                  <span className='inline-flex items-center gap-4px'>
                    {AUTH_ICON[host.authType]}
                    {t(`ssh.form.auth${host.authType.charAt(0).toUpperCase()}${host.authType.slice(1)}` as never)}
                  </span>
                  {host.sudoPassword ? <span>· sudo</span> : null}
                </div>
              </div>
              <div className='flex shrink-0 items-center gap-4px opacity-0 transition-opacity group-hover:opacity-100'>
                <Button size='small' type='secondary' icon={<Edit theme='outline' size='14' />} onClick={() => openEdit(host)} />
                <Button size='small' type='secondary' status='danger' onClick={() => handleDelete(host)}>
                  {t('ssh.delete.ok')}
                </Button>
              </div>
            </div>
          ))}
        </div>
      )}

      <SshHostFormModal
        visible={modalVisible}
        editHost={editHost}
        onClose={() => setModalVisible(false)}
        onSaved={() => void mutate()}
      />
    </div>
  );
};

export default SshHostManagement;
