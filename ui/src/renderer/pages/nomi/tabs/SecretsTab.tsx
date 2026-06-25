/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Empty, Input, Message, Modal, Popconfirm, Spin, Tag } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import type { ISecretListItem } from '@/common/adapter/ipcBridge';
import type { useCompanion } from '../useNomi';

interface Props {
  companion: ReturnType<typeof useCompanion>;
}

/**
 * 伙伴「浏览器凭据」(browser-use secrets) Tab —— per-pet 注册凭据。
 *
 * 用户注册 `(name, value, allowed_origins)`：value 加密落机器绑定 vault，**永不**回前端/LLM
 * （列表只显 name + 绑定域）。在浏览器动作里以 `secret:NAME` 引用，仅在 origin 匹配 allowed_origins 时
 * 注入真值（origin 门 fail-closed）。allowed_origins 同时作浏览器出口域 allowlist（共用一份配置）。
 */
const SecretsTab: React.FC<Props> = ({ companion }) => {
  const { t } = useTranslation();
  const { profile } = companion;
  const petId = profile?.id ?? '';

  const [secrets, setSecrets] = useState<ISecretListItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [addVisible, setAddVisible] = useState(false);
  const [name, setName] = useState('');
  const [value, setValue] = useState('');
  const [origins, setOrigins] = useState('');
  const [saving, setSaving] = useState(false);

  const refreshSeq = useRef(0);

  const refresh = useCallback(async () => {
    if (!petId) return;
    const seq = ++refreshSeq.current;
    setLoading(true);
    try {
      const list = await ipcBridge.browserSecret.list.invoke({ pet_id: petId });
      if (seq === refreshSeq.current) setSecrets(list);
    } catch (e) {
      if (seq === refreshSeq.current) Message.error(String(e));
    } finally {
      if (seq === refreshSeq.current) setLoading(false);
    }
  }, [petId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const openAdd = () => {
    setName('');
    setValue('');
    setOrigins('');
    setAddVisible(true);
  };

  const submitAdd = useCallback(async () => {
    const trimmedName = name.trim();
    const allowedOrigins = origins
      .split(/[\n,]/)
      .map((o) => o.trim())
      .filter((o) => o.length > 0);
    if (!trimmedName) {
      Message.warning(t('nomi.secrets.nameRequired'));
      return;
    }
    if (!value) {
      Message.warning(t('nomi.secrets.valueRequired'));
      return;
    }
    if (allowedOrigins.length === 0) {
      Message.warning(t('nomi.secrets.originsRequired'));
      return;
    }
    setSaving(true);
    try {
      await ipcBridge.browserSecret.register.invoke({
        pet_id: petId,
        name: trimmedName,
        value,
        allowed_origins: allowedOrigins,
      });
      Message.success(t('nomi.secrets.registered'));
      setAddVisible(false);
      // Clear the plaintext value from component state immediately after submit.
      setValue('');
      await refresh();
    } catch (e) {
      Message.error(String(e));
    } finally {
      setSaving(false);
    }
  }, [name, value, origins, petId, refresh, t]);

  const remove = useCallback(
    async (secretName: string) => {
      try {
        await ipcBridge.browserSecret.remove.invoke({ pet_id: petId, name: secretName });
        Message.success(t('nomi.secrets.removed'));
        await refresh();
      } catch (e) {
        Message.error(String(e));
      }
    },
    [petId, refresh, t]
  );

  if (!profile) {
    return (
      <div className='flex justify-center py-40px'>
        <Spin />
      </div>
    );
  }

  return (
    <div className='flex flex-col gap-10px py-8px'>
      <div className='flex items-start gap-16px bg-fill-2 rd-10px px-14px py-12px'>
        <div className='w-200px shrink-0'>
          <div className='text-14px text-t-primary font-500'>{t('nomi.secrets.title')}</div>
          <div className='text-12px text-t-tertiary mt-2px'>{t('nomi.secrets.hint')}</div>
        </div>
        <div className='flex-1 min-w-0 flex flex-col gap-8px'>
          <div>
            <Button type='primary' size='small' onClick={openAdd}>
              {t('nomi.secrets.add')}
            </Button>
          </div>
          {loading ? (
            <div className='flex justify-center py-20px'>
              <Spin />
            </div>
          ) : secrets.length === 0 ? (
            <Empty description={t('nomi.secrets.empty')} />
          ) : (
            <div className='flex flex-col gap-6px'>
              {secrets.map((s) => (
                <div
                  key={s.name}
                  className='flex items-center justify-between gap-12px bg-fill-1 rd-8px px-12px py-8px'
                >
                  <div className='min-w-0'>
                    <div className='text-13px text-t-primary font-500 truncate'>
                      <span className='text-t-tertiary'>secret:</span>
                      {s.name}
                    </div>
                    <div className='flex flex-wrap gap-4px mt-4px'>
                      {s.allowed_origins.map((o) => (
                        <Tag key={o} size='small' color='arcoblue'>
                          {o}
                        </Tag>
                      ))}
                    </div>
                  </div>
                  <Popconfirm
                    title={t('nomi.secrets.removeConfirm', { name: s.name })}
                    onOk={() => remove(s.name)}
                  >
                    <Button size='mini' status='danger' type='text'>
                      {t('nomi.secrets.delete')}
                    </Button>
                  </Popconfirm>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>

      <Modal
        title={t('nomi.secrets.add')}
        visible={addVisible}
        onOk={submitAdd}
        confirmLoading={saving}
        onCancel={() => setAddVisible(false)}
        okText={t('nomi.secrets.register')}
      >
        <div className='flex flex-col gap-12px'>
          <div>
            <div className='text-12px text-t-secondary mb-4px'>{t('nomi.secrets.nameLabel')}</div>
            <Input value={name} onChange={setName} placeholder={t('nomi.secrets.namePlaceholder')} />
          </div>
          <div>
            <div className='text-12px text-t-secondary mb-4px'>{t('nomi.secrets.valueLabel')}</div>
            <Input.Password
              value={value}
              onChange={setValue}
              placeholder={t('nomi.secrets.valuePlaceholder')}
            />
            <div className='text-11px text-t-tertiary mt-4px'>{t('nomi.secrets.valueWriteOnly')}</div>
          </div>
          <div>
            <div className='text-12px text-t-secondary mb-4px'>{t('nomi.secrets.originsLabel')}</div>
            <Input.TextArea
              value={origins}
              onChange={setOrigins}
              placeholder={t('nomi.secrets.originsPlaceholder')}
              autoSize={{ minRows: 2, maxRows: 4 }}
            />
            <div className='text-11px text-t-tertiary mt-4px'>{t('nomi.secrets.originsHint')}</div>
          </div>
        </div>
      </Modal>
    </div>
  );
};

export default SecretsTab;
