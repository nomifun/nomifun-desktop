/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Empty, Input, Message, Modal, Popconfirm, Select, Spin, Tag, Tooltip } from '@arco-design/web-react';
import { Pin } from '@icon-park/react';
import { ipcBridge } from '@/common';
import type { ICompanionMemory } from '@/common/adapter/ipcBridge';

const KINDS = ['profile', 'preference', 'knowledge', 'episode', 'task', 'affective'] as const;

const KIND_COLORS: Record<string, string> = {
  profile: 'gray',
  preference: 'pinkpurple',
  knowledge: 'green',
  episode: 'orange',
  task: 'red',
  affective: 'purple',
};

const MemoriesTab: React.FC = () => {
  const { t } = useTranslation();
  const [memories, setMemories] = useState<ICompanionMemory[]>([]);
  const [loading, setLoading] = useState(true);
  const [kind, setKind] = useState<string>('');
  const [q, setQ] = useState('');
  const [memStatus, setMemStatus] = useState('active');
  const [addVisible, setAddVisible] = useState(false);
  const [addKind, setAddKind] = useState<string>('knowledge');
  const [addContent, setAddContent] = useState('');

  const refreshSeq = useRef(0);

  const refresh = useCallback(async () => {
    const seq = ++refreshSeq.current;
    setLoading(true);
    try {
      const list = await ipcBridge.companion.listMemories.invoke({
        kind: kind || undefined,
        q: q || undefined,
        status: memStatus,
        limit: 200,
      });
      // Out-of-order guard: a slow stale response must not clobber the
      // results of a newer query (rapid typing fires overlapping requests).
      if (seq === refreshSeq.current) setMemories(list);
    } catch (e) {
      if (seq === refreshSeq.current) Message.error(String(e));
    } finally {
      if (seq === refreshSeq.current) setLoading(false);
    }
  }, [kind, q, memStatus]);

  // Debounce keystroke-driven refetches; filter changes flush immediately
  // because the debounce window is short enough not to feel laggy.
  useEffect(() => {
    const timer = setTimeout(() => void refresh(), 250);
    return () => clearTimeout(timer);
  }, [refresh]);

  // nomi can now save memories mid-chat — reflect them live.
  useEffect(() => {
    const unsub = ipcBridge.companion.onMemoryCreated.on(() => void refresh());
    return unsub;
  }, [refresh]);

  const togglePin = useCallback(
    async (m: ICompanionMemory) => {
      await ipcBridge.companion.updateMemory.invoke({ id: m.id, pinned: !m.pinned });
      void refresh();
    },
    [refresh]
  );

  const toggleArchive = useCallback(
    async (m: ICompanionMemory) => {
      await ipcBridge.companion.updateMemory.invoke({ id: m.id, status: m.status === 'active' ? 'archived' : 'active' });
      void refresh();
    },
    [refresh]
  );

  const remove = useCallback(
    async (m: ICompanionMemory) => {
      await ipcBridge.companion.deleteMemory.invoke({ id: m.id });
      void refresh();
    },
    [refresh]
  );

  const add = useCallback(async () => {
    if (!addContent.trim()) return;
    try {
      await ipcBridge.companion.addMemory.invoke({ kind: addKind, content: addContent.trim() });
      setAddVisible(false);
      setAddContent('');
      void refresh();
      Message.success(t('nomi.memories.added'));
    } catch (e) {
      Message.error(String(e));
    }
  }, [addKind, addContent, refresh, t]);

  return (
    <div className='flex flex-col gap-12px py-8px'>
      <div className='flex gap-8px flex-wrap items-center'>
        <Select style={{ width: 140 }} value={kind} onChange={setKind} placeholder={t('nomi.memories.kindAll')}>
          <Select.Option value=''>{t('nomi.memories.kindAll')}</Select.Option>
          {KINDS.map((k) => (
            <Select.Option key={k} value={k}>
              {t(`nomi.kinds.${k}`)}
            </Select.Option>
          ))}
        </Select>
        <Select style={{ width: 110 }} value={memStatus} onChange={setMemStatus}>
          <Select.Option value='active'>{t('nomi.memories.statusActive')}</Select.Option>
          <Select.Option value='archived'>{t('nomi.memories.statusArchived')}</Select.Option>
        </Select>
        <Input.Search
          style={{ width: 220 }}
          placeholder={t('nomi.memories.searchPlaceholder')}
          value={q}
          onChange={setQ}
          allowClear
        />
        <Button type='primary' onClick={() => setAddVisible(true)}>
          {t('nomi.memories.add')}
        </Button>
      </div>
      {loading ? (
        <div className='flex justify-center py-40px'>
          <Spin />
        </div>
      ) : memories.length === 0 ? (
        <Empty description={t('nomi.memories.empty')} />
      ) : (
        <div className='flex flex-col gap-8px'>
          {memories.map((m) => (
            <div key={m.id} className='flex items-start gap-10px bg-fill-2 rd-10px px-12px py-10px'>
              <Tag color={KIND_COLORS[m.kind]}>{t(`nomi.kinds.${m.kind}`)}</Tag>
              <div className='flex-1 min-w-0'>
                <div className='text-13px text-t-primary break-words'>{m.content}</div>
                <div className='mt-4px flex items-center gap-10px text-11px text-t-tertiary'>
                  <span>
                    {t('nomi.memories.strength')} {(m.strength * 100).toFixed(0)}%
                  </span>
                  <span>{new Date(m.updated_at).toLocaleString()}</span>
                  {m.source !== 'learn' && <span>{t(`nomi.memories.source_${m.source}`, m.source)}</span>}
                </div>
              </div>
              <div className='flex items-center gap-4px shrink-0'>
                <Tooltip content={m.pinned ? t('nomi.memories.unpin') : t('nomi.memories.pin')}>
                  <Button
                    size='mini'
                    type={m.pinned ? 'primary' : 'secondary'}
                    icon={<Pin theme='outline' size='12' />}
                    onClick={() => void togglePin(m)}
                  />
                </Tooltip>
                <Button size='mini' onClick={() => void toggleArchive(m)}>
                  {m.status === 'active' ? t('nomi.memories.archive') : t('nomi.memories.restore')}
                </Button>
                <Popconfirm title={t('nomi.memories.deleteConfirm')} onOk={() => void remove(m)}>
                  <Button size='mini' status='danger'>
                    {t('nomi.memories.delete')}
                  </Button>
                </Popconfirm>
              </div>
            </div>
          ))}
        </div>
      )}
      <Modal
        title={t('nomi.memories.add')}
        visible={addVisible}
        onOk={() => void add()}
        onCancel={() => setAddVisible(false)}
        okButtonProps={{ disabled: !addContent.trim() }}
      >
        <div className='flex flex-col gap-12px'>
          <Select value={addKind} onChange={setAddKind}>
            {KINDS.map((k) => (
              <Select.Option key={k} value={k}>
                {t(`nomi.kinds.${k}`)}
              </Select.Option>
            ))}
          </Select>
          <Input.TextArea
            rows={4}
            value={addContent}
            onChange={setAddContent}
            placeholder={t('nomi.memories.addPlaceholder')}
          />
        </div>
      </Modal>
    </div>
  );
};

export default MemoriesTab;
