/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IApiSshConfigScan, IApiSshHost } from '@/common/adapter/ipcBridge';
import { ipcBridge } from '@/common';
import { useCallback, useRef, useState } from 'react';
import { Alert, Button, Dropdown, Form, Input, InputNumber, Menu, Message, Modal, Select, Spin } from '@arco-design/web-react';
import NomiModal from '@renderer/components/base/NomiModal';
import { VisualEmpty, VisualListRow, VisualSearch } from '../components/CodeVisualPrimitives';
import { Certificate, Delete, Download, Edit, Fingerprint, Key, Lock, More, Plus, Server, Speed, Terminal } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import useSWR from 'swr';
import type { I18nKey } from '@renderer/services/i18n';
import { useOpenSshSession } from '@renderer/pages/conversation/hooks/useOpenSshSession';
import {
  buildUpdatePayload,
  validateSshHostForm,
  type SshAuthType,
  type SshHostFormValues,
} from './sshHostForm.validation';
import {
  candidateEndpoint,
  hostBookPrimaryCta,
  scanNotes,
  summarizeImport,
} from './sshConfigImport';

const FormItem = Form.Item;

const AUTH_ICON: Record<SshAuthType, React.ReactNode> = {
  password: <Lock theme='outline' size='14' />,
  key: <Key theme='outline' size='14' />,
  certificate: <Certificate theme='outline' size='14' />,
  agent: <Fingerprint theme='outline' size='14' />,
};

/**
 * One literal {@link I18nKey} per auth method, same shape as {@link AUTH_ICON}.
 * Building the key from the value instead (`` `ssh.form.auth${…}` ``) only gets
 * past I18nKey with a cast that disables the union, and that cast is what lets
 * an unmapped method render its raw key into the row. As a table, a new method
 * is a typecheck failure here rather than a surprise on screen.
 */
const AUTH_LABEL_KEY: Record<SshAuthType, I18nKey> = {
  password: 'ssh.form.authPassword',
  key: 'ssh.form.authKey',
  certificate: 'ssh.form.authCertificate',
  agent: 'ssh.form.authAgent',
};

// ── Add / edit form modal ───────────────────────────────────────────────

interface FormModalProps {
  visible: boolean;
  editHost?: IApiSshHost;
  onClose: () => void;
  /** The saved row, so a caller that opened the form to *use* a host can act on it. */
  onSaved: (host: IApiSshHost) => void;
}

/**
 * Add / edit form for one SSH host.
 *
 * Exported because the session sidebar's remote-session menu opens the very same
 * form for its "add a host" path — an operator should never meet two different
 * host forms depending on where they started.
 */
export const SshHostFormModal: React.FC<FormModalProps> = ({ visible, editHost, onClose, onSaved }) => {
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
      let saved: IApiSshHost;
      if (editHost) {
        const updates = buildUpdatePayload(values);
        saved = await ipcBridge.ssh.update.invoke({
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
        saved = await ipcBridge.ssh.create.invoke({
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
      onSaved(saved);
      onClose();
    } catch (e) {
      Message.error(String(e));
    } finally {
      setSaving(false);
    }
  }, [editHost, isEdit, onClose, onSaved, t]);

  const handleTest = useCallback(async () => {
    const values = currentValues();
    const errorKey = validateSshHostForm(values, isEdit);
    if (errorKey) {
      Message.warning(t(errorKey));
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

// ── ~/.ssh/config import ────────────────────────────────────────────────

interface ImportModalProps {
  visible: boolean;
  /** The server's scan. `undefined` while loading or when the scan failed. */
  scan?: IApiSshConfigScan;
  onClose: () => void;
  onImported: () => void;
}

/**
 * Confirm-then-import: the whole candidate list, shown as it will be saved, with
 * one button. No per-row checkboxes — picking a subset is what the edit and
 * delete actions on the resulting rows are for, and a host book is cheap.
 *
 * The list is exactly what the server offered; nothing here re-derives a host,
 * and the request carries only aliases, so the server reads no path this screen
 * could invent.
 */
const SshConfigImportModal: React.FC<ImportModalProps> = ({ visible, scan, onClose, onImported }) => {
  const { t } = useTranslation();
  const [importing, setImporting] = useState(false);
  const candidates = scan?.hosts ?? [];
  const notes = scan ? scanNotes(scan) : [];

  const handleImport = useCallback(async () => {
    if (candidates.length === 0) return;
    setImporting(true);
    try {
      const result = await ipcBridge.ssh.importHosts.invoke({
        aliases: candidates.map((candidate) => candidate.alias),
      });
      const { level, clauses } = summarizeImport(result);
      // Every clause the summary produced, so the toast reports what actually
      // happened rather than a flat "imported".
      const text = clauses.map((clause) => t(clause.key, clause.values)).join(' ');
      if (level === 'success') Message.success(text);
      else Message.warning(text);
      onImported();
      onClose();
    } catch (e) {
      Message.error(t('ssh.import.failed', { message: String(e) }));
    } finally {
      setImporting(false);
    }
  }, [candidates, onClose, onImported, t]);

  return (
    <NomiModal
      visible={visible}
      onCancel={onClose}
      header={{ title: t('ssh.import.title'), showClose: true }}
      style={{ maxWidth: '92vw', borderRadius: 16 }}
      contentStyle={{
        background: 'var(--dialog-fill-0)',
        borderRadius: 16,
        padding: '20px 24px 16px',
        overflow: 'auto',
      }}
      okText={t('ssh.import.confirm', { count: candidates.length })}
      cancelText={t('ssh.form.cancel')}
      onOk={handleImport}
      confirmLoading={importing}
    >
      <div className='flex w-460px max-w-full flex-col gap-10px'>
        <div className='text-12px leading-18px text-t-tertiary'>
          {t('ssh.import.description', { path: scan?.configPath ?? '~/.ssh/config' })}
        </div>
        <div className='flex max-h-320px flex-col gap-6px overflow-auto'>
          {candidates.map((candidate) => (
            <div
              key={candidate.alias}
              className='flex items-center gap-10px rd-8px border border-solid border-arco-2 bg-fill-0 px-10px py-8px'
            >
              <span className='flex size-26px shrink-0 items-center justify-center rd-6px bg-brand-light text-brand'>
                <Server theme='outline' size='14' fill='currentColor' />
              </span>
              <div className='min-w-0 flex-1'>
                <div className='truncate text-13px font-600 leading-19px text-t-primary'>{candidate.alias}</div>
                <div className='mt-1px truncate font-mono text-11px leading-17px text-t-secondary'>
                  {candidateEndpoint(candidate)}
                </div>
              </div>
              {/* A key path is not a secret, and it is the one thing that decides
                  whether the imported host can connect straight away. */}
              <div className='max-w-160px shrink-0 truncate text-11px leading-17px text-t-tertiary'>
                {candidate.identityFile ? (
                  <span className='inline-flex items-center gap-4px'>
                    <Key theme='outline' size='12' />
                    <span className='truncate font-mono'>{candidate.identityFile}</span>
                  </span>
                ) : (
                  t('ssh.import.noKey')
                )}
              </div>
            </div>
          ))}
        </div>
        {notes.map((note) => (
          <div key={note.key} className='text-11px leading-17px text-t-tertiary'>
            {t(note.key, note.values)}
          </div>
        ))}
      </div>
    </NomiModal>
  );
};

// ── Host book panel ─────────────────────────────────────────────────────

const SshHostManagement: React.FC = () => {
  const { t } = useTranslation();
  const { data: hosts, mutate, error, isLoading } = useSWR('ssh-hosts.list', () => ipcBridge.ssh.list.invoke());
  // Scanned once per mount, silently: the empty state is import-first only when
  // there is genuinely something to import. `shouldRetryOnError: false` keeps a
  // backend without this route (an older build) from being polled about it.
  const { data: scan } = useSWR(
    'ssh-hosts.import-candidates',
    () => ipcBridge.ssh.importCandidates.invoke(),
    { shouldRetryOnError: false }
  );
  const [query, setQuery] = useState('');
  const [modalVisible, setModalVisible] = useState(false);
  const [importVisible, setImportVisible] = useState(false);
  const [editHost, setEditHost] = useState<IApiSshHost | undefined>();
  const [menuHostId, setMenuHostId] = useState<string>();
  const menuTriggerRef = useRef<HTMLButtonElement | null>(null);

  const cta = hostBookPrimaryCta(scan);
  const isEmpty = !hosts || hosts.length === 0;
  /** Empty book *and* a config worth reading: the import takes the lead. */
  const importFirst = isEmpty && cta.kind === 'import';

  const openAdd = () => {
    setEditHost(undefined);
    setModalVisible(true);
  };
  const openEdit = (host: IApiSshHost) => {
    setEditHost(host);
    setModalVisible(true);
  };

  // Create a nomi conversation with this host's canonical resource binding and jump
  // to it — the factory connects the host and hands the agent the remote tools.
  // Shared with the sidebar's remote-session menu.
  const openSession = useOpenSshSession();

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

  const search = query.trim().toLocaleLowerCase();
  const visibleHosts = (hosts ?? []).filter((host) =>
    (host.name + ' ' + host.username + ' ' + host.host).toLocaleLowerCase().includes(search)
  );

  return (
    <div>
      <div className='cv-toolbar'>
        <VisualSearch value={query} onChange={setQuery} label={t('settings.workspace.hostSearch')} clearLabel={t('settings.workspace.clearSearch')} />
        <div className='flex shrink-0 items-center gap-8px'>
          {!isEmpty && cta.kind === 'import' && <Button size='small' type='text' icon={<Download theme='outline' size={14} />} onClick={() => setImportVisible(true)}>{t('ssh.import.entry')}</Button>}
          <Button type={importFirst ? 'outline' : 'primary'} icon={<Plus theme='outline' size={14} />} onClick={openAdd}>{t('ssh.empty.add')}</Button>
        </div>
      </div>
      {error && <Alert className='mb-16px' type='error' content={t('settings.workspace.hostsLoadFailed')}
        action={<Button size='small' onClick={() => void mutate().catch(() => undefined)}>{t('settings.workspace.retry')}</Button>} />}
      {isLoading && !hosts ? <div className='flex min-h-200px items-center justify-center gap-8px' role='status'>
        <Spin /><span>{t('settings.workspace.hostsLoading')}</span>
      </div> : error && !hosts ? null : isEmpty ? (
        <VisualEmpty icon={<Server theme='outline' size={22} />} title={t('ssh.empty.title')} description={t('ssh.empty.description')}
          action={cta.kind === 'import' && <>
            <Button type='primary' icon={<Download theme='outline' size={14} />} onClick={() => setImportVisible(true)}>{t('ssh.import.cta')}</Button>
            <span className='cv-toolbar__hint'>{t('ssh.import.detected', { count: cta.count })}</span>
          </>} />
      ) : !visibleHosts.length ? <VisualEmpty icon={<Server theme='outline' size={22} />} title={t('settings.workspace.noResults')}
        action={<Button onClick={() => setQuery('')}>{t('settings.workspace.clearSearch')}</Button>} /> : (
        <section className='cv-host-list' aria-label={t('settings.workspace.hostList')}>
          {visibleHosts.map((host) => (
            <VisualListRow
              key={host.sshHostId}
              icon={<Server theme='outline' size={17} />}
              title={host.name}
              description={
                <span className='cv-host-details'>
                  <span className='cv-host-endpoint'>{host.username}@{host.host}:{host.port}</span>
                  <span className='inline-flex items-center gap-6px'>
                    {AUTH_ICON[host.authType]}{t(AUTH_LABEL_KEY[host.authType])}{host.sudoPassword ? <span>· sudo</span> : null}
                  </span>
                </span>
              }
              action={
                <Dropdown
                  trigger='click'
                  position='br'
                  popupVisible={menuHostId === host.sshHostId}
                  onVisibleChange={(visible) => setMenuHostId((current) =>
                    visible ? host.sshHostId : current === host.sshHostId ? undefined : current
                  )}
                  triggerProps={{ escToClose: true }}
                  droplist={
                    <Menu className='cv-host-menu' onKeyDown={(event) => {
                      if (event.key === 'Escape') {
                        event.stopPropagation();
                        setMenuHostId(undefined);
                        menuTriggerRef.current?.focus();
                      }
                    }}>
                      <Menu.Item key='session' onClick={() => void openSession(host)}>
                        <span className='flex items-center gap-8px'>
                          <Terminal theme='outline' size={15} />{t('ssh.newSession')}
                        </span>
                      </Menu.Item>
                      <Menu.Item key='edit' onClick={() => openEdit(host)}>
                        <span className='flex items-center gap-8px'>
                          <Edit theme='outline' size={15} />{t('common.edit')}
                        </span>
                      </Menu.Item>
                      <Menu.Item key='delete' className='cv-host-menu__delete' onClick={() => handleDelete(host)}>
                        <span className='flex items-center gap-8px'>
                          <Delete theme='outline' size={15} />{t('ssh.delete.ok')}
                        </span>
                      </Menu.Item>
                    </Menu>
                  }
                >
                  <Button
                    className='cv-host-more'
                    size='small'
                    aria-label={`${t('common.more')} · ${host.name}`}
                    aria-haspopup='menu'
                    aria-expanded={menuHostId === host.sshHostId}
                    title={t('common.more')}
                    onFocus={(event) => { menuTriggerRef.current = event.currentTarget; }}
                    icon={<More theme='outline' size={17} />}
                  />
                </Dropdown>
              }
            />
          ))}
        </section>
      )}

      <SshHostFormModal
        visible={modalVisible}
        editHost={editHost}
        onClose={() => setModalVisible(false)}
        onSaved={() => void mutate()}
      />

      <SshConfigImportModal
        visible={importVisible}
        scan={scan}
        onClose={() => setImportVisible(false)}
        onImported={() => void mutate()}
      />
    </div>
  );
};

export default SshHostManagement;
