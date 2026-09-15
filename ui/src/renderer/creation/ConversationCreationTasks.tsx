import { createContext, useContext, type ReactNode, useState } from 'react';
import useSWR from 'swr';
import { Alert, Image, Message, Tooltip } from '@arco-design/web-react';
import { Close, Download, EditTwo, Refresh, VideoTwo } from '@icon-park/react';
import type { ConversationId, MessageId } from '@/common/types/ids';
import { creativeAssetClient } from '@/renderer/pages/creativeStudio/assets/client';
import type { CreativeAsset } from '@/renderer/pages/creativeStudio/assets/types';
import { useCreationComposer } from './CreationComposerContext';
import { cancelCreation, creationTasksKey, listCreationTasks } from './client';
import { creationModeFor, type ConversationCreationTask, type CreationMode } from './types';
import styles from './ConversationCreationTasks.module.css';
import { recallCreationTask } from './recallTask';
import { useConversationContextSafe } from '@/renderer/hooks/context/ConversationContext';

function useTasks(id: ConversationId, enabled: boolean) {
  const conversation = useConversationContextSafe();
  return useSWR(enabled ? creationTasksKey(id) : null, () => listCreationTasks(id), {
    refreshInterval: data => conversation?.isProcessing || data?.some(task => task.status === 'queued' || task.status === 'running') ? 2000 : 15000,
    revalidateOnFocus: true, revalidateOnReconnect: true,
  });
}
const TaskContext = createContext<ReturnType<typeof useTasks> | null>(null);
export function ConversationCreationTasksProvider({ conversationId, children }: { conversationId: ConversationId; children: ReactNode }) {
  const creation = useCreationComposer();
  const tasks = useTasks(conversationId, Boolean(creation));
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

function TaskCard({ task, refresh }: { task: ConversationCreationTask; refresh(): unknown }) {
  const creation = useCreationComposer();
  const [canceling, setCanceling] = useState(false);
  const mode = creationModeFor(task.capability);
  const recall = async (target: CreationMode | null = mode, result?: CreativeAsset) => {
    if (!creation || !target || !mode) return;
    try {
      const assets = result ? [result] : await Promise.all((task.inputs || []).map(input => creativeAssetClient.get(input.asset_id)));
      creation.update(draft => recallCreationTask(draft, task, assets, target, Boolean(result)));
      creation.selectMode(target);
    } catch (error) { Message.error(error instanceof Error ? error.message : String(error)); }
  };
  const cancel = async () => {
    setCanceling(true);
    try { await cancelCreation(task.owner.conversation_id, task.creation_task_id); await refresh(); }
    catch (error) { Message.error(error instanceof Error ? error.message : String(error)); }
    finally { setCanceling(false); }
  };
  const active = task.status === 'queued' || task.status === 'running';
  const details = `${task.model}${task.params.size ? ` · ${task.params.size}` : ''}${task.params.seconds ? ` · ${task.params.seconds}s` : ''}`;
  return <article className={styles.card} data-creation-task={task.creation_task_id}>
    <div className={styles.header}>
      <strong>{mode === 'image' ? '图片生成' : mode === 'video' ? '视频生成' : mode === 'music' ? '音乐生成' : '语音合成'}</strong>
      <div className={styles.headerActions}>
        <span role='status'>{statusLabel[task.status]}</span>
        {active ? <TaskAction label={canceling ? '取消中…' : '取消任务'} disabled={canceling} onClick={() => void cancel()}><Close size={16} /></TaskAction>
          : creation && mode && <TaskAction label={task.status === 'failed' ? '调整后重试' : '再次创作'} onClick={() => void recall()}><Refresh size={16} /></TaskAction>}
      </div>
    </div>
    <div className={styles.meta} title={details}>{details}</div>
    {active && <p>任务已提交，完成后会显示在本条消息中。</p>}
    {task.error && <p role='alert'>{task.error.message || task.error.kind || '生成失败，请检查模型和参数后重试。'}</p>}
    <div className={styles.results}>{task.result_asset_ids.map(id => <ResultAsset key={id} id={id} onRecall={recall} />)}</div>
  </article>;
}

function ResultAsset({ id, onRecall }: { id: string; onRecall(mode: CreationMode, asset: CreativeAsset): Promise<void> }) {
  const { data: asset, error, mutate } = useSWR(['conversation-result-asset', id], () => creativeAssetClient.get(id));
  if (error) return <button type='button' onClick={() => void mutate()}>素材读取失败，重试</button>;
  if (!asset) return <span>加载素材…</span>;
  if (asset.deletedAt) return <span>此素材已删除</span>;
  return <div className={`${styles.result}${asset.kind === 'image' ? ` ${styles.imageResult}` : ''}`}>
    {asset.kind === 'image' ? <Image className={styles.image} src={asset.originalUrl} alt={asset.title} /> : asset.kind === 'video' ? <video controls preload='metadata' src={asset.originalUrl} /> : asset.kind === 'audio' ? <audio controls preload='metadata' src={asset.originalUrl} /> : <p>{asset.textContent}</p>}
    <div className={styles.actions}>
      <Tooltip content='下载' mini trigger={['hover', 'focus']}><a className={styles.action} href={asset.originalUrl} download={asset.title} aria-label='下载'><Download size={16} fill='currentColor' /></a></Tooltip>
      {asset.kind === 'image' && <>
        <TaskAction label='编辑图片' onClick={() => void onRecall('image', asset)}><EditTwo size={16} fill='currentColor' /></TaskAction>
        <TaskAction label='转为视频' onClick={() => void onRecall('video', asset)}><VideoTwo size={16} fill='currentColor' /></TaskAction>
      </>}
    </div>
  </div>;
}
