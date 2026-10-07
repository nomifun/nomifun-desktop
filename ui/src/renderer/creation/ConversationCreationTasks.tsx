import { createContext, useContext, useEffect, useMemo, type ReactNode, useState } from 'react';
import useSWR from 'swr';
import { Alert, Message, Tooltip } from '@arco-design/web-react';
import { Close, Download, EditTwo, Loading, Music, Pic, Refresh, Text, VideoTwo, Voice } from '@icon-park/react';
import type { ConversationId, MessageId } from '@/common/types/ids';
import { conversation as conversationEvents } from '@/common/adapter/ipcBridge';
import ImageLightbox from '@/renderer/components/media/ImageLightbox';
import { creativeAssetClient } from '@/renderer/pages/creativeStudio/assets/client';
import { saveCreativeAssetAs } from '@/renderer/pages/creativeStudio/assets/saveCreativeAsset';
import type { CreativeAsset } from '@/renderer/pages/creativeStudio/assets/types';
import { useCreationComposer } from './CreationComposerContext';
import { cancelCreation, creationTasksKey, listCreationTasks } from './client';
import { creationModeFor, type ConversationCreationTask, type CreationMode } from './types';
import styles from './ConversationCreationTasks.module.css';
import { recallCreationTask } from './recallTask';
import { useConversationContextSafe } from '@/renderer/hooks/context/ConversationContext';
import { addEventListener } from '@/renderer/utils/emitter';
import { isAuthoritativeCompletionRuntimeIdle } from '@/renderer/pages/conversation/platforms/authoritativeTurnLifecyclePolicy';

function useTasks(id: ConversationId, enabled: boolean) {
  const conversation = useConversationContextSafe();
  const tasks = useSWR(enabled ? creationTasksKey(id) : null, () => listCreationTasks(id), {
    refreshInterval: data => conversation?.isProcessing || data?.some(task => task.status === 'queued' || task.status === 'running') ? 2000 : 15000,
    revalidateOnFocus: true, revalidateOnReconnect: true,
  });
  const { mutate } = tasks;
  useEffect(() => {
    if (!enabled) return;
    let disposed = false;
    let reading = false;
    const refresh = () => {
      // WebSocket completion and HTTP idle reconciliation can announce the
      // same Turn together. One pending read covers both delivery paths.
      if (disposed || reading) return;
      reading = true;
      void mutate().catch(() => {}).finally(() => { reading = false; });
    };
    const offSettled = addEventListener('conversation.turn.settled', conversationId => {
      if (conversationId === id) refresh();
    });
    const offCompleted = conversationEvents.turnCompleted.on(event => {
      if (event.conversation_id === id && isAuthoritativeCompletionRuntimeIdle(event.runtime)) refresh();
    });
    return () => { disposed = true; offSettled(); offCompleted(); };
  }, [id, enabled, mutate]);
  return tasks;
}
const TaskContext = createContext<ReturnType<typeof useTasks> | null>(null);
export function ConversationCreationTasksProvider({ conversationId, enabled, children }: { conversationId: ConversationId; enabled: boolean; children: ReactNode }) {
  const tasks = useTasks(conversationId, enabled);
  return <TaskContext.Provider value={tasks}>{children}{tasks.error && <Alert type='warning' content={<span>生成任务状态读取失败。<button type='button' onClick={() => void tasks.mutate()}>重试</button></span>} />}</TaskContext.Provider>;
}

const statusLabel = { queued: '排队中', running: '生成中', succeeded: '已完成', failed: '生成失败', canceled: '已取消' };

function TaskAction({ label, children, disabled, onClick }: { label: string; children: ReactNode; disabled?: boolean; onClick(): void }) {
  return <Tooltip content={label} mini trigger={['hover', 'focus']}>
    <button type='button' className={styles.action} aria-label={label} aria-disabled={disabled || undefined} onClick={() => { if (!disabled) onClick(); }}>{children}</button>
  </Tooltip>;
}

export function ConversationCreationTaskCards({ messageId }: { messageId: MessageId }) {
  const tasks = useContext(TaskContext);
  const items = tasks?.data?.filter(task => task.owner.message_id === messageId) || [];
  return items.length ? <div className={styles.cards}>{items.map(task => <TaskCard key={task.creation_task_id} task={task} refresh={() => tasks?.mutate()} />)}</div> : null;
}

export function useConversationCreationTaskOwnerMessageIds(): ReadonlySet<MessageId> {
  const tasks = useContext(TaskContext);
  const ownerMessageIds = Array.from(
    new Set((tasks?.data ?? []).map(task => task.owner.message_id))
  ).sort();
  const ownerMessageIdKey = ownerMessageIds.join('\0');
  return useMemo(
    () => new Set<MessageId>(
      ownerMessageIdKey
        ? ownerMessageIdKey.split('\0').map(messageId => messageId as MessageId)
        : []
    ),
    [ownerMessageIdKey]
  );
}

function TaskCard({ task, refresh }: { task: ConversationCreationTask; refresh(): unknown }) {
  const creation = useCreationComposer();
  const readOnly = useConversationContextSafe()?.readOnly === true;
  const [canceling, setCanceling] = useState(false);
  const mode = creationModeFor(task.capability);
  const recall = async (target: CreationMode | null = mode, result?: CreativeAsset) => {
    if (readOnly || !creation || !target || !mode) return;
    try {
      const assets = result ? [result] : await Promise.all((task.inputs || []).map(input => creativeAssetClient.get(input.asset_id)));
      creation.update(draft => recallCreationTask(draft, task, assets, target, Boolean(result)));
      creation.selectMode(target);
    } catch (error) { Message.error(error instanceof Error ? error.message : String(error)); }
  };
  const cancel = async () => {
    if (readOnly) return;
    setCanceling(true);
    try { await cancelCreation(task.owner.conversation_id, task.creation_task_id); await refresh(); }
    catch (error) { Message.error(error instanceof Error ? error.message : String(error)); }
    finally { setCanceling(false); }
  };
  const active = task.status === 'queued' || task.status === 'running';
  const title = task.capability === 'text' ? '文本创作' : mode === 'image' ? '图片生成' : mode === 'video' ? '视频生成' : mode === 'music' ? '音乐生成' : '语音合成';
  const details = `${task.model}${task.params.size ? ` · ${task.params.size}` : ''}${task.params.seconds ? ` · ${task.params.seconds}s` : ''}`;
  return <article className={styles.card} data-creation-task={task.creation_task_id}>
    <div className={styles.header}>
      <strong>{title}</strong>
      <div className={styles.headerActions}>
        <span role='status' aria-live='polite' aria-atomic='true'>{statusLabel[task.status]}</span>
        {!readOnly && (active ? <TaskAction label={canceling ? '取消中…' : '取消任务'} disabled={canceling} onClick={() => void cancel()}><Close size={16} /></TaskAction>
          : creation && mode && <TaskAction label={task.status === 'failed' ? '调整后重试' : '再次创作'} onClick={() => void recall()}><Refresh size={16} /></TaskAction>)}
      </div>
    </div>
    <div className={styles.meta} title={details}>{details}</div>
    {active && <PendingCreationTask task={task} title={title} mode={mode} />}
    {task.error && <p role='alert'>{task.error.message || task.error.kind || '生成失败，请检查模型和参数后重试。'}</p>}
    <div className={styles.results}>{task.result_asset_ids.map(id => <ResultAsset key={id} id={id} onRecall={!readOnly && creation && mode ? recall : undefined} />)}</div>
  </article>;
}

function PendingCreationTask({ task, title, mode }: { task: ConversationCreationTask; title: string; mode: CreationMode | null }) {
  const queued = task.status === 'queued';
  const output = task.capability === 'text' ? '文本' : mode === 'image' ? '图片' : mode === 'video' ? '视频' : mode === 'music' ? '音乐' : '语音';
  const Icon = task.capability === 'text' ? Text : mode === 'image' ? Pic : mode === 'video' ? VideoTwo : mode === 'music' ? Music : Voice;
  return <div
    className={`${styles.pending}${mode === 'image' ? ` ${styles.imagePending}` : mode === 'video' ? ` ${styles.videoPending}` : ''}`}
    role='group'
    aria-label={`${title}：${statusLabel[task.status]}`}
    aria-busy='true'
    data-creation-pending={task.status}
  >
    <span className={styles.pendingIcon} aria-hidden='true'>
      {queued ? <Icon theme='outline' size={28} fill='currentColor' /> : <Loading className={styles.spin} theme='outline' size={28} fill='currentColor' />}
    </span>
    <strong className={styles.pendingLabel}>{queued ? '等待生成' : '正在生成'}{output}</strong>
    <p className={styles.pendingHint}>{queued ? '任务已排队，生成结果会显示在这里。' : '生成完成后，结果会显示在这里。'}</p>
  </div>;
}

function ResultAsset({ id, onRecall }: { id: string; onRecall?(mode: CreationMode, asset: CreativeAsset): Promise<void> }) {
  const { data: asset, error, mutate } = useSWR(['conversation-result-asset', id], () => creativeAssetClient.get(id));
  const [saving, setSaving] = useState(false);
  if (error) return <button type='button' onClick={() => void mutate()}>素材读取失败，重试</button>;
  if (!asset) return <span>加载素材…</span>;
  if (asset.deletedAt) return <span>此素材已删除</span>;
  const saveAs = async () => {
    if (saving) return;
    setSaving(true);
    try {
      await saveCreativeAssetAs(asset, '另存为');
    } catch (reason) {
      Message.error(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setSaving(false);
    }
  };
  return <div className={`${styles.result}${asset.kind === 'image' ? ` ${styles.imageResult}` : ''}`}>
    {asset.kind === 'image' ? <ConversationImagePreview src={asset.originalUrl} title={asset.title} onSaveAs={saveAs} /> : asset.kind === 'video' ? <video controls preload='metadata' src={asset.originalUrl} /> : asset.kind === 'audio' ? <audio controls preload='metadata' src={asset.originalUrl} /> : <p>{asset.textContent}</p>}
    <div className={styles.actions}>
      {asset.kind !== 'text' ? <TaskAction label={saving ? '正在另存为…' : '另存为'} disabled={saving} onClick={() => void saveAs()}><Download size={16} fill='currentColor' /></TaskAction> : null}
      {asset.kind === 'image' && onRecall && <>
        <TaskAction label='编辑图片' onClick={() => void onRecall('image', asset)}><EditTwo size={16} fill='currentColor' /></TaskAction>
        <TaskAction label='转为视频' onClick={() => void onRecall('video', asset)}><VideoTwo size={16} fill='currentColor' /></TaskAction>
      </>}
    </div>
  </div>;
}

export function ConversationImagePreview({ src, title, onSaveAs }: { src: string; title: string; onSaveAs?: () => Promise<unknown> }) {
  const [visible, setVisible] = useState(false);
  return <>
    <button type='button' className={styles.previewTrigger} aria-label={`预览图片：${title}`} onClick={() => setVisible(true)}>
      <img className={styles.image} src={src} alt={title} />
    </button>
    {visible ? <ImageLightbox src={src} title={title} onSaveAs={onSaveAs} onClose={() => setVisible(false)} /> : null}
  </>;
}
