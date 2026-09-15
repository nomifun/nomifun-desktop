import { createContext, useContext, type ReactNode, useState } from 'react';
import useSWR from 'swr';
import { Alert, Image, Message } from '@arco-design/web-react';
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
  return <article className={styles.card} data-creation-task={task.creation_task_id}>
    <div className={styles.header}><strong>{mode === 'image' ? '图片生成' : mode === 'video' ? '视频生成' : mode === 'music' ? '音乐生成' : '语音合成'}</strong><span role='status'>{statusLabel[task.status]}</span></div>
    <div className={styles.meta}>{task.model}{task.params.size ? ` · ${task.params.size}` : ''}{task.params.seconds ? ` · ${task.params.seconds}s` : ''}</div>
    {active && <p>任务已提交，完成后会显示在本条消息中。</p>}
    {task.error && <p role='alert'>{task.error.message || task.error.kind || '生成失败，请检查模型和参数后重试。'}</p>}
    <div className={styles.results}>{task.result_asset_ids.map(id => <ResultAsset key={id} id={id} onRecall={recall} />)}</div>
    <div className={styles.actions}>{active ? <button type='button' disabled={canceling} onClick={() => void cancel()}>{canceling ? '取消中…' : '取消任务'}</button> : creation && mode && <button type='button' onClick={() => void recall()}>{task.status === 'failed' ? '调整后重试' : '再次创作'}</button>}</div>
  </article>;
}

function ResultAsset({ id, onRecall }: { id: string; onRecall(mode: CreationMode, asset: CreativeAsset): Promise<void> }) {
  const { data: asset, error, mutate } = useSWR(['conversation-result-asset', id], () => creativeAssetClient.get(id));
  if (error) return <button type='button' onClick={() => void mutate()}>素材读取失败，重试</button>;
  if (!asset) return <span>加载素材…</span>;
  if (asset.deletedAt) return <span>此素材已删除</span>;
  const save = async () => {
    try { await creativeAssetClient.update(asset.id, { inLibrary: true }); await mutate(); Message.success('已保存到我的素材'); }
    catch (error) { Message.error(error instanceof Error ? error.message : String(error)); }
  };
  return <div className={styles.result}>
    {asset.kind === 'image' ? <Image src={asset.originalUrl} alt={asset.title} /> : asset.kind === 'video' ? <video controls preload='metadata' src={asset.originalUrl} /> : asset.kind === 'audio' ? <audio controls preload='metadata' src={asset.originalUrl} /> : <p>{asset.textContent}</p>}
    <div className={styles.actions}><a href={asset.originalUrl} download={asset.title}>下载</a><button type='button' disabled={asset.inLibrary} onClick={() => void save()}>{asset.inLibrary ? '已保存到素材' : '保存到素材'}</button>{asset.kind === 'image' && <><button type='button' onClick={() => void onRecall('image', asset)}>编辑图片</button><button type='button' onClick={() => void onRecall('video', asset)}>转为视频</button></>}</div>
  </div>;
}
