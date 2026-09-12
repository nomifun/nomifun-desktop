/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate, useParams } from 'react-router-dom';
import {
  Button,
  Checkbox,
  Dropdown,
  Input,
  InputNumber,
  Menu,
  Message,
  Modal,
  Popconfirm,
  Select,
  Spin,
  Switch,
  Table,
  Tag,
} from '@arco-design/web-react';
import { DeleteOne, Down, EditOne, Left, More, Plus, PreviewOpen } from '@icon-park/react';
import { ipcBridge } from '@/common';
import type { ICsAgent, ICsAgentPatch, ICsHandoff, ICsNote } from '@/common/adapter/ipcBridge';
import { parseCsAgentId, type CsAgentId, type KnowledgeBaseId, type ProviderId } from '@/common/types/ids';
import NomiInput from '@/renderer/components/base/NomiInput';
import NomiSelect from '@/renderer/components/base/NomiSelect';
import { useModelsForTask } from '@renderer/hooks/agent/useModelsForTask';
import CsChannelBotsSection from './CsChannelBotsSection';
import styles from './CsAgentDetailPage.module.css';
import { useCsAgent } from './useCsAgents';
import { useKnowledgeBaseOptions } from './useKnowledgeBaseOptions';

/** One titled card section on the detail page. */
const Section: React.FC<{ title: string; extra?: React.ReactNode; children: React.ReactNode }> = ({ title, extra, children }) => (
  <section className={styles.section}>
    <div className={styles.sectionHeader}>
      <span className={styles.sectionTitle}>{title}</span>
      {extra}
    </div>
    {children}
  </section>
);

type NoteModalMode = 'create' | 'edit' | 'view';

const NOTE_PAGE_SIZE = 5;
const EMPTY_NOTE_DRAFT = { kind: 'faq', content: '', aliases: '', shared: false };

/**
 * 客服详情页（/customer-service/:cs_agent_id）：身份与话术编辑、模型与知识库、
 * 渠道机器人绑定管理（复选全量替换）、客服笔记（cs_notes）简表 CRUD。
 */
const CsAgentDetailPage: React.FC = () => {
  const params = useParams<{ cs_agent_id: string }>();
  const csAgentId = useMemo<CsAgentId | null>(() => {
    try {
      return params.cs_agent_id ? parseCsAgentId(params.cs_agent_id) : null;
    } catch {
      return null;
    }
  }, [params.cs_agent_id]);

  return <CsAgentDetailContent key={csAgentId ?? 'invalid'} csAgentId={csAgentId} />;
};

const CsAgentDetailContent: React.FC<{ csAgentId: CsAgentId | null }> = ({ csAgentId }) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const active = useRef(false);
  const busyRef = useRef({ identity: false, note: false, handoff: false, deleting: false });
  const noteConfirmation = useRef<ReturnType<typeof Modal.confirm> | null>(null);
  useLayoutEffect(() => {
    active.current = true;
    return () => { active.current = false; noteConfirmation.current?.close(); };
  }, []);
  const { agent, loading, patch } = useCsAgent(csAgentId);
  // Task-filtered catalog (chat): providers with at least one chat-capable model.
  const { groups: chatGroups } = useModelsForTask('chat');
  const providers = useMemo(() => chatGroups.map((g) => g.provider), [chatGroups]);
  const { options: kbOptions } = useKnowledgeBaseOptions();

  // ── identity draft (explicit save; text fields shouldn't PATCH per keystroke) ──
  const [editedIdentity, setDraft] = useState<Pick<ICsAgent, 'name' | 'greeting' | 'persona' | 'service_policy'> | null>(null);
  const draft = editedIdentity ?? {
    name: agent?.name ?? '', greeting: agent?.greeting ?? '',
    persona: agent?.persona ?? '', service_policy: agent?.service_policy ?? '',
  };
  const [savingIdentity, setSavingIdentity] = useState(false);

  const saveIdentity = async () => {
    if (!active.current || busyRef.current.identity || !draft.name.trim()) return;
    busyRef.current.identity = true;
    setSavingIdentity(true);
    try {
      await patch({
        name: draft.name.trim(),
        greeting: draft.greeting,
        persona: draft.persona,
        service_policy: draft.service_policy,
      });
      if (!active.current) return;
      // A response acknowledges only the submitted draft, not edits made while saving.
      setDraft((current) => current === editedIdentity ? null : current);
      Message.success(t('customerService.detail.saved', { defaultValue: '已保存' }));
    } catch (error) {
      if (active.current) Message.error(error instanceof Error ? error.message : String(error));
    } finally {
      busyRef.current.identity = false;
      if (active.current) setSavingIdentity(false);
    }
  };

  const patchSettings = async (fields: ICsAgentPatch) => {
    if (!active.current) return;
    try {
      await patch(fields);
    } catch (error) {
      if (active.current) Message.error(error instanceof Error ? error.message : String(error));
    }
  };

  // ── notes ────────────────────────────────────────────────────────────
  const [notes, setNotes] = useState<ICsNote[]>([]);
  const [noteModalOpen, setNoteModalOpen] = useState(false);
  const [noteModalMode, setNoteModalMode] = useState<NoteModalMode>('create');
  const [activeNote, setActiveNote] = useState<ICsNote | null>(null);
  const [noteDraft, setNoteDraft] = useState(EMPTY_NOTE_DRAFT);
  const [notePage, setNotePage] = useState(1);
  const [savingNote, setSavingNote] = useState(false);
  const notesRequest = useRef(0);

  const refreshNotes = useCallback(async () => {
    if (!csAgentId || !active.current) return;
    const request = ++notesRequest.current;
    try {
      const notes = (await ipcBridge.customerService.listNotes.invoke({ cs_agent_id: csAgentId })) ?? [];
      if (active.current && request === notesRequest.current) setNotes(notes);
    } catch {
      if (active.current && request === notesRequest.current) setNotes([]);
    }
  }, [csAgentId]);

  useEffect(() => {
    void refreshNotes();
  }, [refreshNotes]);

  useEffect(() => {
    const lastPage = Math.max(1, Math.ceil(notes.length / NOTE_PAGE_SIZE));
    setNotePage((currentPage) => Math.min(currentPage, lastPage));
  }, [notes.length]);

  const openCreateNote = () => {
    if (busyRef.current.note) return;
    setActiveNote(null);
    setNoteModalMode('create');
    setNoteDraft(EMPTY_NOTE_DRAFT);
    setNoteModalOpen(true);
  };

  const openNote = (note: ICsNote, mode: Exclude<NoteModalMode, 'create'>) => {
    if (busyRef.current.note) return;
    setActiveNote(note);
    setNoteModalMode(mode);
    setNoteDraft({
      kind: note.kind,
      content: note.content,
      aliases: note.aliases ?? '',
      shared: note.cs_agent_id === null,
    });
    setNoteModalOpen(true);
  };

  const closeNoteModal = () => {
    if (busyRef.current.note) return;
    setNoteModalOpen(false);
  };

  const saveNote = async () => {
    if (!active.current || busyRef.current.note || !csAgentId || !noteDraft.content.trim() || noteModalMode === 'view') return;
    busyRef.current.note = true;
    setSavingNote(true);
    try {
      if (noteModalMode === 'edit' && activeNote) {
        await ipcBridge.customerService.patchNote.invoke({
          cs_note_id: activeNote.cs_note_id,
          kind: noteDraft.kind,
          content: noteDraft.content,
          aliases: noteDraft.aliases,
        });
        if (!active.current) return;
        Message.success(t('customerService.notes.updated', { defaultValue: '笔记已更新' }));
      } else {
        await ipcBridge.customerService.createNote.invoke({
          cs_agent_id: noteDraft.shared ? null : csAgentId,
          kind: noteDraft.kind,
          content: noteDraft.content,
          aliases: noteDraft.aliases,
          enabled: true,
        });
        if (!active.current) return;
        Message.success(t('customerService.notes.created', { defaultValue: '笔记已创建' }));
      }
      setNoteModalOpen(false);
      setActiveNote(null);
      setNoteDraft(EMPTY_NOTE_DRAFT);
      await refreshNotes();
    } catch (error) {
      if (active.current) Message.error(error instanceof Error ? error.message : String(error));
    } finally {
      busyRef.current.note = false;
      if (active.current) setSavingNote(false);
    }
  };

  const removeNote = (note: ICsNote) => {
    if (!active.current) return;
    let removing = false;
    noteConfirmation.current?.close();
    noteConfirmation.current = Modal.confirm({
      title: t('customerService.notes.deleteConfirm', { defaultValue: '删除该笔记？' }),
      okText: t('customerService.notes.delete', { defaultValue: '删除' }),
      cancelText: t('common.cancel', { defaultValue: '取消' }),
      okButtonProps: { status: 'danger' },
      onOk: async () => {
        if (!active.current || removing) return;
        removing = true;
        try {
          await ipcBridge.customerService.removeNote.invoke({ cs_note_id: note.cs_note_id });
          if (!active.current) return;
          Message.success(t('customerService.notes.deleted', { defaultValue: '笔记已删除' }));
          await refreshNotes();
        } catch (error) {
          if (!active.current) return;
          Message.error(error instanceof Error ? error.message : String(error));
          throw error;
        } finally {
          removing = false;
        }
      },
    });
  };

  const setNoteEnabled = async (note: ICsNote, enabled: boolean) => {
    if (!active.current) return;
    try {
      await ipcBridge.customerService.patchNote.invoke({ cs_note_id: note.cs_note_id, enabled });
      await refreshNotes();
    } catch (error) {
      if (active.current) Message.error(error instanceof Error ? error.message : String(error));
    }
  };

  const handleNoteMenuAction = (key: string, note: ICsNote) => {
    if (key === 'view') {
      openNote(note, 'view');
      return;
    }
    if (key === 'edit') {
      openNote(note, 'edit');
      return;
    }
    if (key === 'delete') removeNote(note);
  };

  // ── durable human handoff queue ───────────────────────────────────
  const [handoffs, setHandoffs] = useState<ICsHandoff[]>([]);
  const [handoffsLoading, setHandoffsLoading] = useState(false);
  const [handoffBusyId, setHandoffBusyId] = useState<string | null>(null);
  const [resolvingHandoff, setResolvingHandoff] = useState<ICsHandoff | null>(null);
  const [handoffResolution, setHandoffResolution] = useState('');
  const handoffsRequest = useRef(0);

  const refreshHandoffs = useCallback(async () => {
    if (!csAgentId || !active.current) return;
    const request = ++handoffsRequest.current;
    const isCurrent = () => active.current && request === handoffsRequest.current;
    setHandoffsLoading(true);
    try {
      const handoffs = await ipcBridge.customerService.listHandoffs.invoke({
        cs_agent_id: csAgentId,
        limit: 100,
      });
      if (isCurrent()) setHandoffs(handoffs);
    } catch (error) {
      if (isCurrent()) {
        setHandoffs([]);
        Message.error(error instanceof Error ? error.message : String(error));
      }
    } finally {
      if (isCurrent()) setHandoffsLoading(false);
    }
  }, [csAgentId]);

  useEffect(() => {
    void refreshHandoffs();
  }, [refreshHandoffs]);

  const updateHandoff = async (handoff: ICsHandoff, action: 'claim' | 'cancel' | 'resolve') => {
    if (!active.current || busyRef.current.handoff || (action === 'resolve' && !handoffResolution.trim())) return;
    busyRef.current.handoff = true;
    setHandoffBusyId(handoff.cs_handoff_id);
    try {
      if (action === 'claim') {
        await ipcBridge.customerService.claimHandoff.invoke({ cs_handoff_id: handoff.cs_handoff_id, expected_status: 'pending' });
      } else if (action === 'cancel') {
        await ipcBridge.customerService.cancelHandoff.invoke({
          cs_handoff_id: handoff.cs_handoff_id,
          expected_status: handoff.status === 'claimed' ? 'claimed' : 'pending',
          resolution: t('customerService.handoffs.cancelledByOwner', { defaultValue: '由主人取消' }),
        });
      } else {
        await ipcBridge.customerService.resolveHandoff.invoke({
          cs_handoff_id: handoff.cs_handoff_id, expected_status: 'claimed', resolution: handoffResolution.trim(),
        });
      }
      if (!active.current) return;
      if (action === 'resolve') { setResolvingHandoff(null); setHandoffResolution(''); }
      Message.success(action === 'claim'
        ? t('customerService.handoffs.claimed', { defaultValue: '已认领人工交接' })
        : action === 'cancel'
          ? t('customerService.handoffs.cancelled', { defaultValue: '交接已取消' })
          : t('customerService.handoffs.resolved', { defaultValue: '交接已完成' }));
    } catch (error) {
      if (active.current) Message.error(error instanceof Error ? error.message : String(error));
    } finally {
      if (active.current) await refreshHandoffs();
      busyRef.current.handoff = false;
      if (active.current) setHandoffBusyId(null);
    }
  };

  const handoffStatusLabel = (status: ICsHandoff['status']): string => {
    if (status === 'pending') return t('customerService.handoffs.statusPending', { defaultValue: '待认领' });
    if (status === 'claimed') return t('customerService.handoffs.statusClaimed', { defaultValue: '处理中' });
    if (status === 'resolved') return t('customerService.handoffs.statusResolved', { defaultValue: '已完成' });
    return t('customerService.handoffs.statusCancelled', { defaultValue: '已取消' });
  };

  // ── delete agent ─────────────────────────────────────────────────────
  const deleteAgent = async () => {
    if (!csAgentId || !active.current || busyRef.current.deleting) return;
    busyRef.current.deleting = true;
    try {
      await ipcBridge.customerService.removeAgent.invoke({ cs_agent_id: csAgentId });
      if (!active.current) return;
      Message.success(t('customerService.detail.deleted', { defaultValue: '客服已删除' }));
      void navigate('/customer-service');
    } catch (error) {
      if (active.current) Message.error(error instanceof Error ? error.message : String(error));
    } finally {
      busyRef.current.deleting = false;
    }
  };

  if (loading) {
    return (
      <div className='flex justify-center py-56px'>
        <Spin />
      </div>
    );
  }
  if (!agent) {
    return (
      <div className='flex flex-col items-center gap-12px py-56px text-t-tertiary'>
        {t('customerService.detail.notFound', { defaultValue: '客服不存在或已删除' })}
        <Button onClick={() => void navigate('/customer-service')}>
          {t('customerService.detail.back', { defaultValue: '返回花名册' })}
        </Button>
      </div>
    );
  }

  const modelOptions = chatGroups.find((g) => g.provider.id === agent.provider_id)?.models ?? [];
  const selectedKnowledgeBases = agent.knowledge_base_ids.map((knowledgeBaseId) => ({
    id: knowledgeBaseId,
    label: kbOptions.find((option) => option.value === knowledgeBaseId)?.label ?? knowledgeBaseId,
  }));

  return (
    <div className={styles.page}>
      <div className={styles.shell}>
        {/* Header */}
        <header className={styles.header}>
          <div className={styles.headerMain}>
            <Button size='small' className='shrink-0' onClick={() => void navigate('/customer-service')}>
              <span className='inline-flex items-center gap-4px'>
                <Left theme='outline' size='14' fill='currentColor' className='block' style={{ lineHeight: 0 }} />
                {t('customerService.detail.back', { defaultValue: '返回花名册' })}
              </span>
            </Button>
            <h1 className={styles.agentName}>{agent.name}</h1>
          </div>
          <div className={styles.headerActions}>
            <span className='text-12px text-t-tertiary'>
              {t('customerService.detail.enabled', { defaultValue: '启用' })}
            </span>
            <Switch
              checked={agent.enabled}
              onChange={(checked: boolean) => void patchSettings({ enabled: checked })}
            />
            <Popconfirm
              title={t('customerService.detail.deleteConfirm', {
                defaultValue: '删除该客服？其绑定、对话记录、人工交接与私有笔记将一并删除。',
              })}
              onOk={() => void deleteAgent()}
            >
              <Button status='danger' size='small'>
                {t('customerService.detail.delete', { defaultValue: '删除' })}
              </Button>
            </Popconfirm>
          </div>
        </header>

        <main className={styles.contentGrid}>
          <div className={`${styles.column} ${styles.leftColumn}`}>
            {/* 模型与知识库 */}
            <Section title={t('customerService.sections.modelKnowledge', { defaultValue: '模型与知识库' })}>
              <div className={styles.modelRow}>
                <span className={styles.fieldLabel}>{t('common.model', { defaultValue: '模型' })}</span>
                <div className={styles.modelControls}>
                  <NomiSelect
                    contentFit
                    size='small'
                    contentMaxWidth='100%'
                    className={styles.modelSelect}
                    value={agent.provider_id ?? undefined}
                    placeholder={t('customerService.fields.provider', { defaultValue: '模型服务商' })}
                    allowClear
                    onChange={(value) => void patchSettings({ provider_id: (value as ProviderId | undefined) ?? null, model: null })}
                  >
                    {providers.map((p) => (
                      <NomiSelect.Option key={p.id} value={p.id}>
                        {p.name}
                      </NomiSelect.Option>
                    ))}
                  </NomiSelect>
                  <NomiSelect
                    contentFit
                    size='small'
                    contentMaxWidth='100%'
                    className={styles.modelSelect}
                    value={agent.model ?? undefined}
                    placeholder={t('customerService.fields.model', { defaultValue: '对话模型' })}
                    allowClear
                    onChange={(value) => void patchSettings({ model: (value as string | undefined) ?? null })}
                  >
                    {modelOptions.map((m) => (
                      <NomiSelect.Option key={m} value={m}>
                        {m}
                      </NomiSelect.Option>
                    ))}
                  </NomiSelect>
                </div>
              </div>

              <div className={styles.divider} />

              <div className={`${styles.inlineField} ${styles.knowledgeField}`}>
                <span className={styles.fieldLabel}>{t('customerService.fields.knowledgeBases', { defaultValue: '知识库' })}</span>
                <div className={styles.fieldControl}>
                  <Select
                    mode='multiple'
                    value={agent.knowledge_base_ids}
                    allowClear
                    triggerElement={
                      <Button long size='small' className={styles.selectTrigger}>
                        <span className={styles.selectTriggerLabel}>
                          {selectedKnowledgeBases.length > 0
                            ? t('customerService.detail.knowledgeBasesMountedCount', {
                                count: selectedKnowledgeBases.length,
                                defaultValue: '已挂载{{count}}个',
                              })
                            : t('customerService.detail.knowledgeBasesPlaceholder', { defaultValue: '选择挂载知识库' })}
                        </span>
                        <Down theme='outline' size='13' fill='currentColor' className='shrink-0' />
                      </Button>
                    }
                    onChange={(value) => void patchSettings({ knowledge_base_ids: (value ?? []) as KnowledgeBaseId[] })}
                  >
                    {kbOptions.map((kb) => (
                      <Select.Option key={kb.value} value={kb.value}>
                        {kb.label}
                      </Select.Option>
                    ))}
                  </Select>
                </div>
              </div>

              {selectedKnowledgeBases.length > 0 && (
                <div className={styles.knowledgeTagsRow}>
                  <div className={styles.knowledgeTags}>
                    {selectedKnowledgeBases.map((knowledgeBase) => (
                      <Tag
                        key={knowledgeBase.id}
                        size='small'
                        closable
                        className={styles.knowledgeTag}
                        onClose={() => {
                          void patchSettings({
                            knowledge_base_ids: agent.knowledge_base_ids.filter((id) => id !== knowledgeBase.id),
                          });
                        }}
                      >
                        {knowledgeBase.label}
                      </Tag>
                    ))}
                  </div>
                </div>
              )}

              <div className={styles.divider} />

              <div className={styles.inlineField}>
                <span className={styles.fieldLabel}>{t('customerService.fields.maxConcurrent', { defaultValue: '并发上限' })}</span>
                <InputNumber
                  className={styles.concurrencyControl}
                  min={1}
                  max={64}
                  precision={0}
                  value={agent.max_concurrent}
                  onChange={(value) => {
                    if (typeof value === 'number' && Number.isInteger(value)) void patchSettings({ max_concurrent: value });
                  }}
                />
              </div>
            </Section>

            {/* 身份与话术 */}
            <Section
              title={t('customerService.sections.identity', { defaultValue: '身份与话术' })}
              extra={
                <Button type='primary' size='small' loading={savingIdentity} disabled={!draft.name.trim()} onClick={() => void saveIdentity()}>
                  {t('customerService.detail.save', { defaultValue: '保存' })}
                </Button>
              }
            >
              <div className={styles.nameField}>
                <span className={styles.fieldLabel}>{t('customerService.fields.name', { defaultValue: '名称' })}</span>
                <NomiInput
                  contentFit
                  contentMinWidth={164}
                  contentMaxWidth={360}
                  className={styles.nameControl}
                  value={draft.name}
                  onChange={(value) => setDraft((d) => ({ ...(d ?? draft), name: value }))}
                />
              </div>
              <div className={styles.divider} />
              <div className={styles.identityFields}>
                <div>
                  <div className={styles.fieldLabel}>{t('customerService.fields.greeting', { defaultValue: '问候语' })}</div>
                  <Input.TextArea rows={2} value={draft.greeting} onChange={(value) => setDraft((d) => ({ ...(d ?? draft), greeting: value }))} />
                </div>
                <div>
                  <div className={styles.fieldLabel}>{t('customerService.fields.persona', { defaultValue: '人设话术' })}</div>
                  <Input.TextArea rows={2} value={draft.persona} onChange={(value) => setDraft((d) => ({ ...(d ?? draft), persona: value }))} />
                </div>
                <div>
                  <div className={styles.fieldLabel}>{t('customerService.fields.servicePolicy', { defaultValue: '服务策略' })}</div>
                  <Input.TextArea rows={2} value={draft.service_policy} onChange={(value) => setDraft((d) => ({ ...(d ?? draft), service_policy: value }))} />
                </div>
              </div>
            </Section>
          </div>

          <div className={styles.column}>
            {/* 绑定管理 — 客服域渠道机器人自闭环（与桌面伙伴渠道分域互斥） */}
            {csAgentId && (
              <Section title={t('customerService.sections.bindings', { defaultValue: '渠道机器人绑定' })}>
                <CsChannelBotsSection csAgentId={csAgentId} />
              </Section>
            )}

            <Section
              title={t('customerService.sections.handoffs', { defaultValue: '人工交接队列' })}
              extra={
                <Button size='small' loading={handoffsLoading} onClick={() => void refreshHandoffs()}>
                  {t('customerService.handoffs.refresh', { defaultValue: '刷新' })}
                </Button>
              }
            >
              <div className={styles.tableScroll}>
                <div className={styles.tableInner}>
                  <Table
                    rowKey='cs_handoff_id'
                    data={handoffs}
                    loading={handoffsLoading}
                    pagination={false}
                    size='small'
                    noDataElement={
                      <span className='text-13px text-t-tertiary'>
                        {t('customerService.handoffs.empty', { defaultValue: '当前没有人工交接' })}
                      </span>
                    }
                    columns={[
                      {
                        title: t('customerService.handoffs.reason', { defaultValue: '原因' }),
                        dataIndex: 'reason',
                        render: (reason: string, handoff: ICsHandoff) => (
                          <div className={styles.handoffReason}>
                            <strong>{reason}</strong>
                            {handoff.summary && <span>{handoff.summary}</span>}
                          </div>
                        ),
                      },
                      {
                        title: t('customerService.handoffs.status', { defaultValue: '状态' }),
                        width: 84,
                        render: (_: unknown, handoff: ICsHandoff) => (
                          <Tag
                            size='small'
                            color={handoff.status === 'pending' ? 'orange' : handoff.status === 'claimed' ? 'blue' : handoff.status === 'resolved' ? 'green' : 'gray'}
                          >
                            {handoffStatusLabel(handoff.status)}
                          </Tag>
                        ),
                      },
                      {
                        title: '',
                        width: 176,
                        render: (_: unknown, handoff: ICsHandoff) => {
                          const busy = handoffBusyId === handoff.cs_handoff_id;
                          if (handoff.status === 'resolved' || handoff.status === 'cancelled') {
                            return handoff.resolution ? <span className={styles.handoffResolution}>{handoff.resolution}</span> : null;
                          }
                          return (
                            <span className='inline-flex items-center gap-6px'>
                              {handoff.status === 'pending' && (
                                <Button size='mini' type='primary' loading={busy} disabled={Boolean(handoffBusyId)} onClick={() => void updateHandoff(handoff, 'claim')}>
                                  {t('customerService.handoffs.claim', { defaultValue: '认领' })}
                                </Button>
                              )}
                              {handoff.status === 'claimed' && (
                                <Button size='mini' type='primary' loading={busy} disabled={Boolean(handoffBusyId)} onClick={() => { if (!busyRef.current.handoff) { setResolvingHandoff(handoff); setHandoffResolution(''); } }}>
                                  {t('customerService.handoffs.resolve', { defaultValue: '完成' })}
                                </Button>
                              )}
                              <Button size='mini' status='danger' disabled={Boolean(handoffBusyId)} onClick={() => void updateHandoff(handoff, 'cancel')}>
                                {t('customerService.handoffs.cancel', { defaultValue: '取消' })}
                              </Button>
                            </span>
                          );
                        },
                      },
                    ]}
                  />
                </div>
              </div>
            </Section>

            {/* 客服笔记 */}
            <Section
              title={t('customerService.sections.notes', { defaultValue: '客服笔记' })}
              extra={
                <Button size='small' onClick={openCreateNote}>
                  <span className='inline-flex items-center gap-4px'>
                    <Plus theme='outline' size='13' fill='currentColor' className='block' style={{ lineHeight: 0 }} />
                    {t('customerService.notes.add', { defaultValue: '新增笔记' })}
                  </span>
                </Button>
              }
            >
              <div className={styles.tableScroll}>
                <div className={styles.tableInner}>
                  <Table
                    rowKey='cs_note_id'
                    data={notes}
                    pagination={{
                      current: notePage,
                      pageSize: NOTE_PAGE_SIZE,
                      total: notes.length,
                      size: 'small',
                      showTotal: true,
                      hideOnSinglePage: notes.length <= NOTE_PAGE_SIZE,
                      onChange: (page) => setNotePage(page),
                    }}
                    size='small'
                    noDataElement={
                      <span className='text-13px text-t-tertiary'>
                        {t('customerService.notes.empty', { defaultValue: '还没有笔记 — FAQ/话术/业务事实都可以放在这里，客服只读引用。' })}
                      </span>
                    }
                    columns={[
                      {
                        title: t('customerService.notes.kind', { defaultValue: '类型' }),
                        dataIndex: 'kind',
                        width: 64,
                      },
                      {
                        title: t('customerService.notes.content', { defaultValue: '内容' }),
                        dataIndex: 'content',
                        render: (content: string) => <span className={styles.noteContent}>{content}</span>,
                      },
                      {
                        title: t('customerService.notes.scope', { defaultValue: '范围' }),
                        width: 72,
                        render: (_: unknown, note: ICsNote) => (
                          <Tag size='small' color={note.cs_agent_id ? 'blue' : 'purple'}>
                            {note.cs_agent_id
                              ? t('customerService.notes.private', { defaultValue: '私有' })
                              : t('customerService.notes.shared', { defaultValue: '共享' })}
                          </Tag>
                        ),
                      },
                      {
                        title: t('customerService.notes.enabled', { defaultValue: '启用' }),
                        width: 60,
                        render: (_: unknown, note: ICsNote) => (
                          <Switch
                            size='small'
                            checked={note.enabled}
                            onChange={(checked: boolean) => void setNoteEnabled(note, checked)}
                          />
                        ),
                      },
                      {
                        title: '',
                        width: 48,
                        align: 'center',
                        render: (_: unknown, note: ICsNote) => (
                          <Dropdown
                            trigger='click'
                            position='br'
                            getPopupContainer={() => document.body}
                            droplist={
                              <Menu onClickMenuItem={(key) => handleNoteMenuAction(String(key), note)}>
                                <Menu.Item key='view'>
                                  <span className='flex items-center gap-8px'>
                                    <PreviewOpen theme='outline' size='14' />
                                    {t('customerService.notes.view', { defaultValue: '查看' })}
                                  </span>
                                </Menu.Item>
                                <Menu.Item key='edit'>
                                  <span className='flex items-center gap-8px'>
                                    <EditOne theme='outline' size='14' />
                                    {t('customerService.notes.edit', { defaultValue: '编辑' })}
                                  </span>
                                </Menu.Item>
                                <Menu.Item key='delete'>
                                  <span className='flex items-center gap-8px text-danger-6'>
                                    <DeleteOne theme='outline' size='14' />
                                    {t('customerService.notes.delete', { defaultValue: '删除' })}
                                  </span>
                                </Menu.Item>
                              </Menu>
                            }
                          >
                            <Button
                              size='mini'
                              type='text'
                              aria-label={t('customerService.notes.more', { defaultValue: '更多操作' })}
                              className={styles.noteMoreButton}
                            >
                              <More theme='outline' size='15' fill='currentColor' className='block' style={{ lineHeight: 0 }} />
                            </Button>
                          </Dropdown>
                        ),
                      },
                    ]}
                  />
                </div>
              </div>
            </Section>
          </div>
        </main>
      </div>

      <Modal
        visible={Boolean(resolvingHandoff)}
        title={t('customerService.handoffs.resolveTitle', { defaultValue: '完成人工交接' })}
        onCancel={() => { if (!busyRef.current.handoff) { setResolvingHandoff(null); setHandoffResolution(''); } }}
        onOk={() => { if (resolvingHandoff) void updateHandoff(resolvingHandoff, 'resolve'); }}
        confirmLoading={Boolean(resolvingHandoff && handoffBusyId === resolvingHandoff.cs_handoff_id)}
        okButtonProps={{ disabled: !handoffResolution.trim() || Boolean(handoffBusyId) }}
        unmountOnExit
      >
        <Input.TextArea
          rows={4}
          value={handoffResolution}
          readOnly={Boolean(handoffBusyId)}
          maxLength={12000}
          showWordLimit
          placeholder={t('customerService.handoffs.resolutionPlaceholder', { defaultValue: '记录处理结果，完成后自动客服将恢复接待后续消息。' })}
          onChange={setHandoffResolution}
        />
      </Modal>

      {/* 新增 / 查看 / 编辑笔记 */}
      <Modal
        visible={noteModalOpen}
        title={
          noteModalMode === 'view'
            ? t('customerService.notes.viewTitle', { defaultValue: '查看笔记' })
            : noteModalMode === 'edit'
              ? t('customerService.notes.editTitle', { defaultValue: '编辑笔记' })
              : t('customerService.notes.add', { defaultValue: '新增笔记' })
        }
        onCancel={closeNoteModal}
        onOk={() => void saveNote()}
        confirmLoading={savingNote}
        okButtonProps={{ disabled: !noteDraft.content.trim() }}
        footer={
          noteModalMode === 'view' ? (
            <Button onClick={closeNoteModal}>
              {t('customerService.notes.close', { defaultValue: '关闭' })}
            </Button>
          ) : undefined
        }
        unmountOnExit
        style={{ width: 'min(460px, calc(100vw - 32px))' }}
      >
        <div className='flex flex-col gap-10px'>
          <Select
            value={noteDraft.kind}
            disabled={noteModalMode === 'view' || savingNote}
            onChange={(value) => setNoteDraft((d) => ({ ...d, kind: value as string }))}
          >
            <Select.Option value='faq'>{t('customerService.notes.kindFaq', { defaultValue: 'FAQ' })}</Select.Option>
            <Select.Option value='script'>{t('customerService.notes.kindScript', { defaultValue: '话术' })}</Select.Option>
            <Select.Option value='fact'>{t('customerService.notes.kindFact', { defaultValue: '业务事实' })}</Select.Option>
          </Select>
          <Input.TextArea
            autoSize={{ minRows: 4, maxRows: 12 }}
            value={noteDraft.content}
            placeholder={t('customerService.notes.contentPlaceholder', { defaultValue: '写下 FAQ / 话术 / 业务事实…' })}
            readOnly={noteModalMode === 'view' || savingNote}
            onChange={(value) => setNoteDraft((d) => ({ ...d, content: value }))}
          />
          <Input.TextArea
            autoSize={{ minRows: 2, maxRows: 6 }}
            value={noteDraft.aliases}
            placeholder={t('customerService.notes.aliasesPlaceholder', {
              defaultValue: '其他问法，每行一个，例如：这个软件\n多少钱',
            })}
            readOnly={noteModalMode === 'view' || savingNote}
            onChange={(value) => setNoteDraft((d) => ({ ...d, aliases: value }))}
          />
          <span className='text-12px text-t-tertiary'>
            {t('customerService.notes.aliasesHint', {
              defaultValue: '每行填一个访客可能的说法。当访客的问法与笔记正文用词完全不同时，靠它才能被搜到。',
            })}
          </span>
          <label className='flex items-center gap-8px text-13px text-t-secondary'>
            <Checkbox
              checked={noteDraft.shared}
              disabled={noteModalMode !== 'create' || savingNote}
              onChange={(checked: boolean) => setNoteDraft((d) => ({ ...d, shared: checked }))}
            />
            {t('customerService.notes.sharedHint', { defaultValue: '共享给全部客服（不勾选则仅本客服可用）' })}
          </label>
          {noteModalMode !== 'create' && (
            <span className='text-12px text-t-tertiary'>
              {t('customerService.notes.scopeLocked', { defaultValue: '笔记范围创建后不可修改' })}
            </span>
          )}
        </div>
      </Modal>
    </div>
  );
};

export default CsAgentDetailPage;
