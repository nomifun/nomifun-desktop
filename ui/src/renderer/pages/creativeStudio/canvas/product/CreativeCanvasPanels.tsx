/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  FileText,
  Group,
  History,
  Info,
  Lock,
  Magic,
  PanoramaHorizontal,
  Pic,
  Redo,
  Robot,
  SettingConfig,
  Undo,
  VideoTwo,
  Voice,
  Workbench,
} from '@icon-park/react';
import classNames from 'classnames';
import React from 'react';

import type { CreativeCanvasNode, CreativeCanvasNodeKind } from '../../domain';
import type { CanvasState } from '../core';

import styles from './CreativeCanvasPanels.module.css';

const iconProps = {
  theme: 'outline' as const,
  size: 16,
  fill: 'currentColor',
  strokeWidth: 2.5,
};

const NODE_KIND_LABELS: Record<CreativeCanvasNodeKind, string> = {
  text: '文本',
  image: '图片',
  panorama: '全景图',
  video: '视频',
  audio: '音频',
  config: '生成配置',
  director: '导演台',
  group: '分组',
};

function nodeKindIcon(kind: CreativeCanvasNodeKind): React.ReactNode {
  switch (kind) {
    case 'text':
      return <FileText {...iconProps} />;
    case 'image':
      return <Pic {...iconProps} />;
    case 'panorama':
      return <PanoramaHorizontal {...iconProps} />;
    case 'video':
      return <VideoTwo {...iconProps} />;
    case 'audio':
      return <Voice {...iconProps} />;
    case 'config':
      return <SettingConfig {...iconProps} />;
    case 'director':
      return <Magic {...iconProps} />;
    case 'group':
      return <Group {...iconProps} />;
  }
}

const compactText = (value: string, maxLength = 54): string => {
  const normalized = value.replace(/\s+/g, ' ').trim();
  if (normalized.length <= maxLength) return normalized;
  return `${normalized.slice(0, maxLength - 1)}…`;
};

/** A label projected only from persisted node data; it never invents generated content. */
export function creativeCanvasNodeDisplayName(node: CreativeCanvasNode): string {
  switch (node.type) {
    case 'text':
      return compactText(node.data.text) || '空文本';
    case 'image':
      return compactText(node.data.caption || node.data.alt) || '图片';
    case 'panorama':
      return node.data.assetId ? '已连接全景素材' : '全景图';
    case 'video':
      return node.data.assetId ? '已连接视频素材' : '视频';
    case 'audio':
      return compactText(node.data.title) || '音频';
    case 'config':
      return compactText(node.data.prompt) || '生成配置';
    case 'director':
      return compactText(node.data.sceneId ?? '') || '导演台';
    case 'group':
      return compactText(node.data.title) || '未命名分组';
  }
}

const sortOutlineNodes = (nodes: readonly CreativeCanvasNode[]): CreativeCanvasNode[] =>
  [...nodes].sort(
    (left, right) =>
      left.zIndex - right.zIndex ||
      Number(left.type !== 'group') - Number(right.type !== 'group') ||
      left.id.localeCompare(right.id)
  );

export interface CreativeCanvasOutlinePanelProps {
  state: CanvasState;
  onSelectNode(nodeId: string, mode: 'replace' | 'toggle'): void;
  onClearSelection?: () => void;
  className?: string;
}

/** Read-only graph outline. Selection is delegated to the canonical editor reducer. */
export const CreativeCanvasOutlinePanel: React.FC<CreativeCanvasOutlinePanelProps> = ({
  state,
  onSelectNode,
  onClearSelection,
  className,
}) => {
  const selected = new Set(state.selection.nodeIds);
  const nodes = sortOutlineNodes(state.document.nodes);

  return (
    <section
      className={classNames(styles.panel, className)}
      data-canvas-product-panel='outline'
      aria-label='画布结构'
    >
      <header className={styles.panelHeader}>
        <div>
          <h2>画布结构</h2>
          <p>
            {nodes.length} 个节点 · {state.document.connections.length} 条连接
          </p>
        </div>
        {selected.size > 0 && onClearSelection ? (
          <button type='button' className={styles.textButton} onClick={onClearSelection}>
            清除选择
          </button>
        ) : null}
      </header>

      {nodes.length === 0 ? (
        <p className={styles.outlineEmpty} role='status'>画布暂无节点</p>
      ) : (
        <div className={styles.outlineList} role='list' aria-label='画布节点'>
          {nodes.map((node) => {
            const isSelected = selected.has(node.id);
            return (
              <button
                key={node.id}
                type='button'
                className={styles.outlineItem}
                data-node-id={node.id}
                data-node-kind={node.type}
                data-node-grouped={node.groupId ? 'true' : undefined}
                data-selected={isSelected || undefined}
                aria-pressed={isSelected}
                onClick={(event) =>
                  onSelectNode(
                    node.id,
                    event.shiftKey || event.ctrlKey || event.metaKey ? 'toggle' : 'replace'
                  )
                }
              >
                <span className={styles.nodeIcon} aria-hidden='true'>
                  {nodeKindIcon(node.type)}
                </span>
                <span className={styles.outlineIdentity}>
                  <strong>{creativeCanvasNodeDisplayName(node)}</strong>
                  <span>
                    {NODE_KIND_LABELS[node.type]}
                    {node.groupId ? ' · 已分组' : ''}
                  </span>
                </span>
                {node.locked ? (
                  <span className={styles.lockIcon} title='节点已锁定' aria-label='节点已锁定'>
                    <Lock {...iconProps} />
                  </span>
                ) : null}
              </button>
            );
          })}
        </div>
      )}
    </section>
  );
};

interface PropertyRowProps {
  label: string;
  value: React.ReactNode;
}

const PropertyRow: React.FC<PropertyRowProps> = ({ label, value }) => (
  <div className={styles.propertyRow}>
    <dt>{label}</dt>
    <dd>{value}</dd>
  </div>
);

const optionalValue = (value: string | null | undefined): string => value?.trim() || '未设置';
const booleanValue = (value: boolean): string => (value ? '是' : '否');
const milliseconds = (value: number | null): string => (value === null ? '未设置' : `${value} ms`);

const NodeDataProperties: React.FC<{ node: CreativeCanvasNode; memberCount: number }> = ({
  node,
  memberCount,
}) => {
  switch (node.type) {
    case 'text':
      return (
        <>
          <PropertyRow label='内容' value={node.data.text || '空文本'} />
          <PropertyRow label='格式' value={node.data.format} />
          <PropertyRow label='字号' value={`${node.data.fontSize}px`} />
          <PropertyRow label='对齐' value={node.data.textAlign} />
        </>
      );
    case 'image':
      return (
        <>
          <PropertyRow label='素材 ID' value={optionalValue(node.data.assetId)} />
          <PropertyRow label='说明' value={optionalValue(node.data.caption)} />
          <PropertyRow label='替代文本' value={optionalValue(node.data.alt)} />
          <PropertyRow label='适配' value={node.data.fit} />
          <PropertyRow
            label='原始尺寸'
            value={node.data.naturalSize ? `${node.data.naturalSize.width} × ${node.data.naturalSize.height}` : '未解析'}
          />
        </>
      );
    case 'panorama':
      return (
        <>
          <PropertyRow label='素材 ID' value={optionalValue(node.data.assetId)} />
          <PropertyRow label='投影' value={node.data.projection} />
          <PropertyRow label='视角' value={`yaw ${node.data.yaw}° · pitch ${node.data.pitch}°`} />
          <PropertyRow label='视野' value={`${node.data.fieldOfView}°`} />
        </>
      );
    case 'video':
      return (
        <>
          <PropertyRow label='素材 ID' value={optionalValue(node.data.assetId)} />
          <PropertyRow label='封面素材' value={optionalValue(node.data.posterAssetId)} />
          <PropertyRow label='自动播放' value={booleanValue(node.data.autoplay)} />
          <PropertyRow label='循环' value={booleanValue(node.data.loop)} />
          <PropertyRow label='静音' value={booleanValue(node.data.muted)} />
          <PropertyRow label='裁剪' value={`${milliseconds(node.data.trimStartMs)} – ${milliseconds(node.data.trimEndMs)}`} />
        </>
      );
    case 'audio':
      return (
        <>
          <PropertyRow label='素材 ID' value={optionalValue(node.data.assetId)} />
          <PropertyRow label='标题' value={optionalValue(node.data.title)} />
          <PropertyRow label='音量' value={`${Math.round(node.data.volume * 100)}%`} />
          <PropertyRow label='循环' value={booleanValue(node.data.loop)} />
          <PropertyRow label='裁剪' value={`${milliseconds(node.data.trimStartMs)} – ${milliseconds(node.data.trimEndMs)}`} />
        </>
      );
    case 'config':
      return (
        <>
          <PropertyRow label='任务' value={node.data.task} />
          <PropertyRow label='能力' value={node.data.capability} />
          <PropertyRow label='提供商' value={optionalValue(node.data.providerId)} />
          <PropertyRow label='模型' value={optionalValue(node.data.model)} />
          <PropertyRow label='状态' value={node.data.status} />
          <PropertyRow label='提示词' value={optionalValue(node.data.prompt)} />
          <PropertyRow label='负面提示词' value={optionalValue(node.data.negativePrompt)} />
          <PropertyRow label='输入素材' value={`${node.data.inputAssetIds.length} 项`} />
          <PropertyRow label='结果素材' value={`${node.data.resultAssetIds.length} 项`} />
          <PropertyRow label='任务 ID' value={optionalValue(node.data.taskId)} />
        </>
      );
    case 'director':
      return (
        <>
          <PropertyRow label='场景 ID' value={optionalValue(node.data.sceneId)} />
          <PropertyRow label='机位 ID' value={optionalValue(node.data.cameraId)} />
          <PropertyRow label='当前时间' value={milliseconds(node.data.timelineMs)} />
          <PropertyRow label='时长' value={milliseconds(node.data.durationMs)} />
        </>
      );
    case 'group':
      return (
        <>
          <PropertyRow label='标题' value={optionalValue(node.data.title)} />
          <PropertyRow label='颜色' value={optionalValue(node.data.color)} />
          <PropertyRow label='已折叠' value={booleanValue(node.data.collapsed)} />
          <PropertyRow label='成员' value={`${memberCount} 个节点`} />
        </>
      );
  }
};

interface PropertyEditorFieldProps {
  label: string;
  children: React.ReactNode;
}

const PropertyEditorField: React.FC<PropertyEditorFieldProps> = ({ label, children }) => (
  <label className={styles.editorField}>
    <span>{label}</span>
    {children}
  </label>
);

interface NodeDataEditorProps {
  node: CreativeCanvasNode;
  onUpdate(node: CreativeCanvasNode, field: string): void;
}

const finiteNumber = (
  value: number,
  fallback: number,
  minimum: number,
  maximum = Number.POSITIVE_INFINITY
): number => (Number.isFinite(value) ? Math.min(maximum, Math.max(minimum, value)) : fallback);

const NodeDataEditor: React.FC<NodeDataEditorProps> = ({ node, onUpdate }) => {
  switch (node.type) {
    case 'text':
      return (
        <>
          <PropertyEditorField label='内容'>
            <textarea
              value={node.data.text}
              rows={5}
              onChange={(event) =>
                onUpdate(
                  { ...node, data: { ...node.data, text: event.currentTarget.value } },
                  'text'
                )
              }
            />
          </PropertyEditorField>
          <PropertyEditorField label='格式'>
            <select
              value={node.data.format}
              onChange={(event) =>
                onUpdate(
                  {
                    ...node,
                    data: {
                      ...node.data,
                      format: event.currentTarget.value as typeof node.data.format,
                    },
                  },
                  'format'
                )
              }
            >
              <option value='plain'>纯文本</option>
              <option value='markdown'>Markdown</option>
            </select>
          </PropertyEditorField>
          <PropertyEditorField label='字号'>
            <input
              type='number'
              min={8}
              max={256}
              value={node.data.fontSize}
              onChange={(event) =>
                onUpdate(
                  {
                    ...node,
                    data: {
                      ...node.data,
                      fontSize: finiteNumber(
                        event.currentTarget.valueAsNumber,
                        node.data.fontSize,
                        8,
                        256
                      ),
                    },
                  },
                  'fontSize'
                )
              }
            />
          </PropertyEditorField>
          <PropertyEditorField label='对齐'>
            <select
              value={node.data.textAlign}
              onChange={(event) =>
                onUpdate(
                  {
                    ...node,
                    data: {
                      ...node.data,
                      textAlign: event.currentTarget.value as typeof node.data.textAlign,
                    },
                  },
                  'textAlign'
                )
              }
            >
              <option value='left'>左对齐</option>
              <option value='center'>居中</option>
              <option value='right'>右对齐</option>
            </select>
          </PropertyEditorField>
        </>
      );
    case 'image':
      return (
        <>
          <PropertyEditorField label='说明'>
            <textarea
              value={node.data.caption}
              rows={3}
              onChange={(event) =>
                onUpdate(
                  { ...node, data: { ...node.data, caption: event.currentTarget.value } },
                  'caption'
                )
              }
            />
          </PropertyEditorField>
          <PropertyEditorField label='替代文本'>
            <input
              value={node.data.alt}
              onChange={(event) =>
                onUpdate(
                  { ...node, data: { ...node.data, alt: event.currentTarget.value } },
                  'alt'
                )
              }
            />
          </PropertyEditorField>
          <PropertyEditorField label='适配方式'>
            <select
              value={node.data.fit}
              onChange={(event) =>
                onUpdate(
                  {
                    ...node,
                    data: {
                      ...node.data,
                      fit: event.currentTarget.value as typeof node.data.fit,
                    },
                  },
                  'fit'
                )
              }
            >
              <option value='contain'>完整显示</option>
              <option value='cover'>填满裁切</option>
            </select>
          </PropertyEditorField>
        </>
      );
    case 'panorama':
      return (
        <>
          {([
            ['水平视角', 'yaw', -360, 360],
            ['垂直视角', 'pitch', -90, 90],
            ['视野', 'fieldOfView', 10, 150],
          ] as const).map(([label, field, min, max]) => (
            <PropertyEditorField key={field} label={label}>
              <input
                type='number'
                min={min}
                max={max}
                value={node.data[field]}
                onChange={(event) =>
                  onUpdate(
                    {
                      ...node,
                      data: {
                        ...node.data,
                        [field]: finiteNumber(
                          event.currentTarget.valueAsNumber,
                          node.data[field],
                          min,
                          max
                        ),
                      },
                    },
                    field
                  )
                }
              />
            </PropertyEditorField>
          ))}
        </>
      );
    case 'video':
      return (
        <>
          {([
            ['自动播放', 'autoplay'],
            ['循环播放', 'loop'],
            ['静音', 'muted'],
          ] as const).map(([label, field]) => (
            <PropertyEditorField key={field} label={label}>
              <input
                type='checkbox'
                checked={node.data[field]}
                onChange={(event) =>
                  onUpdate(
                    { ...node, data: { ...node.data, [field]: event.currentTarget.checked } },
                    field
                  )
                }
              />
            </PropertyEditorField>
          ))}
        </>
      );
    case 'audio':
      return (
        <>
          <PropertyEditorField label='标题'>
            <input
              value={node.data.title}
              onChange={(event) =>
                onUpdate(
                  { ...node, data: { ...node.data, title: event.currentTarget.value } },
                  'title'
                )
              }
            />
          </PropertyEditorField>
          <PropertyEditorField label='循环播放'>
            <input
              type='checkbox'
              checked={node.data.loop}
              onChange={(event) =>
                onUpdate(
                  { ...node, data: { ...node.data, loop: event.currentTarget.checked } },
                  'loop'
                )
              }
            />
          </PropertyEditorField>
          <PropertyEditorField label={`音量 ${Math.round(node.data.volume * 100)}%`}>
            <input
              type='range'
              min={0}
              max={1}
              step={0.01}
              value={node.data.volume}
              onChange={(event) =>
                onUpdate(
                  {
                    ...node,
                    data: {
                      ...node.data,
                      volume: finiteNumber(
                        event.currentTarget.valueAsNumber,
                        node.data.volume,
                        0,
                        1
                      ),
                    },
                  },
                  'volume'
                )
              }
            />
          </PropertyEditorField>
        </>
      );
    case 'config':
      return (
        <>
          <PropertyEditorField label='提示词'>
            <textarea
              value={node.data.prompt}
              rows={5}
              onChange={(event) =>
                onUpdate(
                  { ...node, data: { ...node.data, prompt: event.currentTarget.value } },
                  'prompt'
                )
              }
            />
          </PropertyEditorField>
          <PropertyEditorField label='负面提示词'>
            <textarea
              value={node.data.negativePrompt}
              rows={3}
              onChange={(event) =>
                onUpdate(
                  {
                    ...node,
                    data: { ...node.data, negativePrompt: event.currentTarget.value },
                  },
                  'negativePrompt'
                )
              }
            />
          </PropertyEditorField>
        </>
      );
    case 'director':
      return (
        <>
          <PropertyEditorField label='当前时间 (ms)'>
            <input
              type='number'
              min={0}
              max={node.data.durationMs}
              value={node.data.timelineMs}
              onChange={(event) =>
                onUpdate(
                  {
                    ...node,
                    data: {
                      ...node.data,
                      timelineMs: finiteNumber(
                        event.currentTarget.valueAsNumber,
                        node.data.timelineMs,
                        0,
                        node.data.durationMs
                      ),
                    },
                  },
                  'timelineMs'
                )
              }
            />
          </PropertyEditorField>
          <PropertyEditorField label='时长 (ms)'>
            <input
              type='number'
              min={0}
              value={node.data.durationMs}
              onChange={(event) => {
                const durationMs = finiteNumber(
                  event.currentTarget.valueAsNumber,
                  node.data.durationMs,
                  0
                );
                onUpdate(
                  {
                    ...node,
                    data: {
                      ...node.data,
                      durationMs,
                      timelineMs: Math.min(node.data.timelineMs, durationMs),
                    },
                  },
                  'durationMs'
                );
              }}
            />
          </PropertyEditorField>
        </>
      );
    case 'group':
      return (
        <>
          <PropertyEditorField label='标题'>
            <input
              value={node.data.title}
              onChange={(event) =>
                onUpdate(
                  { ...node, data: { ...node.data, title: event.currentTarget.value } },
                  'title'
                )
              }
            />
          </PropertyEditorField>
          <PropertyEditorField label='颜色'>
            <input
              value={node.data.color ?? ''}
              placeholder='未设置'
              onChange={(event) =>
                onUpdate(
                  {
                    ...node,
                    data: { ...node.data, color: event.currentTarget.value.trim() || null },
                  },
                  'color'
                )
              }
            />
          </PropertyEditorField>
          <PropertyEditorField label='折叠'>
            <input
              type='checkbox'
              checked={node.data.collapsed}
              onChange={(event) =>
                onUpdate(
                  { ...node, data: { ...node.data, collapsed: event.currentTarget.checked } },
                  'collapsed'
                )
              }
            />
          </PropertyEditorField>
        </>
      );
  }
};

export interface CreativeCanvasPropertiesPanelProps {
  state: CanvasState;
  onSelectNode?: (nodeId: string) => void;
  onUpdateNode?: (node: CreativeCanvasNode, field: string) => void;
  className?: string;
}

/** Canonical node inspector. Every edit is delegated to the reducer-owned node/update command. */
export const CreativeCanvasPropertiesPanel: React.FC<CreativeCanvasPropertiesPanelProps> = ({
  state,
  onSelectNode,
  onUpdateNode,
  className,
}) => {
  const selectedIds = new Set(state.selection.nodeIds);
  const selectedNodes = state.document.nodes.filter((node) => selectedIds.has(node.id));

  return (
    <section
      className={classNames(styles.panel, className)}
      data-canvas-product-panel='properties'
      aria-label='节点属性'
    >
      <header className={styles.panelHeader}>
        <div>
          <h2>属性</h2>
          <p>{selectedNodes.length > 0 ? `已选择 ${selectedNodes.length} 个节点` : '选择节点后查看详情'}</p>
        </div>
      </header>

      {selectedNodes.length === 0 ? (
        <PanelEmpty icon={<Info {...iconProps} />} title='未选择节点' description='属性面板只展示当前画布中的真实节点数据。' />
      ) : selectedNodes.length > 1 ? (
        <div className={styles.selectionList} role='list' aria-label='已选择节点'>
          {selectedNodes.map((node) => (
            <button
              key={node.id}
              type='button'
              disabled={!onSelectNode}
              onClick={() => onSelectNode?.(node.id)}
            >
              <span className={styles.nodeIcon}>{nodeKindIcon(node.type)}</span>
              <span>
                <strong>{creativeCanvasNodeDisplayName(node)}</strong>
                <small>{NODE_KIND_LABELS[node.type]}</small>
              </span>
            </button>
          ))}
        </div>
      ) : (
        (() => {
          const node = selectedNodes[0];
          const memberCount = state.document.nodes.filter((candidate) => candidate.groupId === node.id).length;
          return (
            <div className={styles.propertiesBody} data-properties-node-kind={node.type}>
              <div className={styles.inspectorIdentity}>
                <span className={styles.nodeIcon}>{nodeKindIcon(node.type)}</span>
                <div>
                  <strong>{creativeCanvasNodeDisplayName(node)}</strong>
                  <span>{NODE_KIND_LABELS[node.type]}</span>
                </div>
              </div>
              <dl className={styles.propertyList}>
                <PropertyRow label='节点 ID' value={node.id} />
                <PropertyRow label='位置' value={`${node.position.x}, ${node.position.y}`} />
                <PropertyRow label='尺寸' value={`${node.size.width} × ${node.size.height}`} />
                <PropertyRow label='层级' value={node.zIndex} />
                <PropertyRow label='分组 ID' value={optionalValue(node.groupId)} />
                <PropertyRow label='锁定' value={booleanValue(node.locked)} />
                <NodeDataProperties node={node} memberCount={memberCount} />
              </dl>
              {onUpdateNode ? (
                <div className={styles.editorForm} aria-label='编辑节点属性'>
                  <h3>编辑</h3>
                  <PropertyEditorField label='锁定节点'>
                    <input
                      type='checkbox'
                      checked={node.locked}
                      onChange={(event) =>
                        onUpdateNode(
                          { ...node, locked: event.currentTarget.checked },
                          'locked'
                        )
                      }
                    />
                  </PropertyEditorField>
                  <NodeDataEditor node={node} onUpdate={onUpdateNode} />
                </div>
              ) : (
                <p className={styles.readOnlyNote}>当前属性面板未连接 canonical 更新命令。</p>
              )}
            </div>
          );
        })()
      )}
    </section>
  );
};

export interface CreativeCanvasHistoryPanelProps {
  state: CanvasState;
  onUndo(): void;
  onRedo(): void;
  className?: string;
}

/** Shows only reducer snapshot counts; no fabricated action names or timestamps. */
export const CreativeCanvasHistoryPanel: React.FC<CreativeCanvasHistoryPanelProps> = ({
  state,
  onUndo,
  onRedo,
  className,
}) => (
  <section
    className={classNames(styles.panel, styles.historyPanel, className)}
    data-canvas-product-panel='history'
    aria-label='编辑历史'
  >
    <header className={styles.panelHeader}>
      <div>
        <h2>编辑历史</h2>
        <p>当前会话的 reducer 快照</p>
      </div>
    </header>
    <div className={styles.historySummary}>
      <div>
        <span>可撤销</span>
        <strong>{state.history.past.length}</strong>
      </div>
      <div>
        <span>可重做</span>
        <strong>{state.history.future.length}</strong>
      </div>
      <div className={styles.historyActions}>
        <button type='button' disabled={state.history.past.length === 0} onClick={onUndo}>
          <Undo {...iconProps} />
          撤销
        </button>
        <button type='button' disabled={state.history.future.length === 0} onClick={onRedo}>
          <Redo {...iconProps} />
          重做
        </button>
      </div>
    </div>
    <p className={styles.historyDisclosure}>核心只保存文档快照，没有操作名称和时间戳；本面板不会臆造历史记录。</p>
  </section>
);

export type CreativeCanvasUnavailableKind = 'assistant' | 'workflows' | 'generic';

export interface CreativeCanvasUnavailablePanelProps {
  kind?: CreativeCanvasUnavailableKind;
  title: string;
  description: string;
  detail?: string;
  className?: string;
}

function unavailableIcon(kind: CreativeCanvasUnavailableKind): React.ReactNode {
  if (kind === 'assistant') return <Robot {...iconProps} />;
  if (kind === 'workflows') return <Workbench {...iconProps} />;
  return <Info {...iconProps} />;
}

/** Honest empty boundary for capabilities whose production adapter is not connected. */
export const CreativeCanvasUnavailablePanel: React.FC<CreativeCanvasUnavailablePanelProps> = ({
  kind = 'generic',
  title,
  description,
  detail,
  className,
}) => (
  <section
    className={classNames(styles.panel, styles.unavailablePanel, className)}
    data-canvas-product-panel='unavailable'
    data-unavailable-kind={kind}
    role='status'
  >
    <span className={styles.unavailableIcon}>{unavailableIcon(kind)}</span>
    <div>
      <h2>{title}</h2>
      <p>{description}</p>
      {detail ? <small>{detail}</small> : null}
    </div>
  </section>
);

export const CreativeCanvasAssistantUnwiredPanel: React.FC<{ className?: string }> = ({
  className,
}) => (
  <CreativeCanvasUnavailablePanel
    kind='assistant'
    className={className}
    title='创作 Agent 尚未连接'
    description='当前没有可验证的画布专属会话绑定，因此不会发送消息或复用主聊天会话。'
    detail='需要接入 canvas/session resolver、真实消息历史和独占会话所有权后才能启用。'
  />
);

export const CreativeCanvasWorkflowUnwiredPanel: React.FC<{ className?: string }> = ({
  className,
}) => (
  <CreativeCanvasUnavailablePanel
    kind='workflows'
    className={className}
    title='模板尚未连接'
    description='当前画布文档没有模板数据源，本面板不会显示示例模板或虚构运行状态。'
  />
);

interface PanelEmptyProps {
  icon: React.ReactNode;
  title: string;
  description: string;
}

const PanelEmpty: React.FC<PanelEmptyProps> = ({ icon, title, description }) => (
  <div className={styles.emptyState} role='status'>
    <span>{icon}</span>
    <strong>{title}</strong>
    <p>{description}</p>
  </div>
);

export default CreativeCanvasOutlinePanel;
