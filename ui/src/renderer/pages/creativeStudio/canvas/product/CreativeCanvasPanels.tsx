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
import type { TFunction } from 'i18next';
import React from 'react';
import { useTranslation } from 'react-i18next';

import type { CreativeCanvasNode, CreativeCanvasNodeKind } from '../../domain';
import type { CanvasState } from '../core';

import styles from './CreativeCanvasPanels.module.css';

const iconProps = {
  theme: 'outline' as const,
  size: 16,
  fill: 'currentColor',
  strokeWidth: 2.5,
};

const NODE_KIND_LABEL_KEYS: Record<CreativeCanvasNodeKind, string> = {
  text: 'creativeStudio.canvas.nodeKinds.text',
  image: 'creativeStudio.canvas.nodeKinds.image',
  panorama: 'creativeStudio.canvas.nodeKinds.panorama',
  video: 'creativeStudio.canvas.nodeKinds.video',
  audio: 'creativeStudio.canvas.nodeKinds.audio',
  config: 'creativeStudio.canvas.nodeKinds.config',
  director: 'creativeStudio.canvas.nodeKinds.director',
  group: 'creativeStudio.canvas.nodeKinds.group',
};

const NODE_KIND_LABEL_FALLBACKS: Record<CreativeCanvasNodeKind, string> = {
  text: '文本',
  image: '图片',
  panorama: '全景图',
  video: '视频',
  audio: '音频',
  config: '生成配置',
  director: '导演台',
  group: '分组',
};

const fallbackTranslate = (
  key: string,
  options?: { defaultValue?: unknown }
): string => String(options?.defaultValue ?? key);

const nodeKindLabel = (
  kind: CreativeCanvasNodeKind,
  t: TFunction = fallbackTranslate as TFunction
): string =>
  t(NODE_KIND_LABEL_KEYS[kind], {
    defaultValue: NODE_KIND_LABEL_FALLBACKS[kind],
  });

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
export function creativeCanvasNodeDisplayName(
  node: CreativeCanvasNode,
  t: TFunction = fallbackTranslate as TFunction
): string {
  switch (node.type) {
    case 'text':
      return (
        compactText(node.data.text) ||
        t('creativeStudio.canvas.nodes.emptyText', {
          defaultValue: '空文本',
        })
      );
    case 'image':
      return compactText(node.data.caption || node.data.alt) || nodeKindLabel('image', t);
    case 'panorama':
      return node.data.assetId
        ? t('creativeStudio.canvas.nodes.connectedPanorama', {
            defaultValue: '已连接全景素材',
          })
        : nodeKindLabel('panorama', t);
    case 'video':
      return node.data.assetId
        ? t('creativeStudio.canvas.nodes.connectedVideo', {
            defaultValue: '已连接视频素材',
          })
        : nodeKindLabel('video', t);
    case 'audio':
      return compactText(node.data.title) || nodeKindLabel('audio', t);
    case 'config':
      return compactText(node.data.prompt) || nodeKindLabel('config', t);
    case 'director':
      return compactText(node.data.sceneId ?? '') || nodeKindLabel('director', t);
    case 'group':
      return (
        compactText(node.data.title) ||
        t('creativeStudio.canvas.nodes.unnamedGroup', {
          defaultValue: '未命名分组',
        })
      );
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
  const { t } = useTranslation();
  const selected = new Set(state.selection.nodeIds);
  const nodes = sortOutlineNodes(state.document.nodes);

  return (
    <section
      className={classNames(styles.panel, className)}
      data-canvas-product-panel='outline'
      aria-label={t('creativeStudio.canvas.outline.label', {
        defaultValue: '画布结构',
      })}
    >
      <header className={styles.panelHeader}>
        <div>
          <h2>
            {t('creativeStudio.canvas.outline.title', {
              defaultValue: '画布结构',
            })}
          </h2>
          <p>
            {t('creativeStudio.canvas.outline.summary', {
              nodeCount: nodes.length,
              connectionCount: state.document.connections.length,
              defaultValue: `${nodes.length} 个节点 · ${state.document.connections.length} 条连接`,
            })}
          </p>
        </div>
        {selected.size > 0 && onClearSelection ? (
          <button type='button' className={styles.textButton} onClick={onClearSelection}>
            {t('creativeStudio.canvas.outline.clearSelection', {
              defaultValue: '清除选择',
            })}
          </button>
        ) : null}
      </header>

      {nodes.length === 0 ? (
        <p className={styles.outlineEmpty} role='status'>
          {t('creativeStudio.canvas.outline.empty', {
            defaultValue: '画布暂无节点',
          })}
        </p>
      ) : (
        <div
          className={styles.outlineList}
          role='list'
          aria-label={t('creativeStudio.canvas.outline.nodesLabel', {
            defaultValue: '画布节点',
          })}
        >
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
                  <strong>{creativeCanvasNodeDisplayName(node, t)}</strong>
                  <span>
                    {nodeKindLabel(node.type, t)}
                    {node.groupId
                      ? t('creativeStudio.canvas.outline.groupedSuffix', {
                          defaultValue: ' · 已分组',
                        })
                      : ''}
                  </span>
                </span>
                {node.locked ? (
                  <span
                    className={styles.lockIcon}
                    title={t('creativeStudio.canvas.nodes.locked', {
                      defaultValue: '节点已锁定',
                    })}
                    aria-label={t('creativeStudio.canvas.nodes.locked', {
                      defaultValue: '节点已锁定',
                    })}
                  >
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

const optionalValue = (
  value: string | null | undefined,
  t: TFunction
): string =>
  value?.trim() ||
  t('creativeStudio.canvas.values.unset', {
    defaultValue: '未设置',
  });
const booleanValue = (value: boolean, t: TFunction): string =>
  value
    ? t('creativeStudio.canvas.values.yes', { defaultValue: '是' })
    : t('creativeStudio.canvas.values.no', { defaultValue: '否' });
const milliseconds = (value: number | null, t: TFunction): string =>
  value === null
    ? t('creativeStudio.canvas.values.unset', { defaultValue: '未设置' })
    : t('creativeStudio.canvas.values.milliseconds', {
        value,
        defaultValue: '{{value}} ms',
      });
const textFormatValue = (
  value: 'plain' | 'markdown',
  t: TFunction
): string =>
  value === 'markdown'
    ? t('creativeStudio.canvas.editor.markdown', { defaultValue: 'Markdown' })
    : t('creativeStudio.canvas.editor.plainText', { defaultValue: '纯文本' });
const textAlignmentValue = (
  value: 'left' | 'center' | 'right',
  t: TFunction
): string => {
  if (value === 'center') {
    return t('creativeStudio.canvas.editor.alignCenter', {
      defaultValue: '居中',
    });
  }
  if (value === 'right') {
    return t('creativeStudio.canvas.editor.alignRight', {
      defaultValue: '右对齐',
    });
  }
  return t('creativeStudio.canvas.editor.alignLeft', {
    defaultValue: '左对齐',
  });
};
const imageFitValue = (value: 'contain' | 'cover', t: TFunction): string =>
  value === 'cover'
    ? t('creativeStudio.canvas.editor.fitCover', {
        defaultValue: '填满裁切',
      })
    : t('creativeStudio.canvas.editor.fitContain', {
        defaultValue: '完整显示',
      });
const generationStatusValue = (
  value: 'idle' | 'queued' | 'running' | 'succeeded' | 'failed' | 'canceled',
  t: TFunction
): string =>
  t(`creativeStudio.canvas.nodes.status.${value}`, {
    defaultValue: value,
  });

const NodeDataProperties: React.FC<{ node: CreativeCanvasNode; memberCount: number }> = ({
  node,
  memberCount,
}) => {
  const { t } = useTranslation();
  switch (node.type) {
    case 'text':
      return (
        <>
          <PropertyRow
            label={t('creativeStudio.canvas.properties.content', {
              defaultValue: '内容',
            })}
            value={
              node.data.text ||
              t('creativeStudio.canvas.nodes.emptyText', {
                defaultValue: '空文本',
              })
            }
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.format', {
              defaultValue: '格式',
            })}
            value={textFormatValue(node.data.format, t)}
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.fontSize', {
              defaultValue: '字号',
            })}
            value={`${node.data.fontSize}px`}
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.alignment', {
              defaultValue: '对齐',
            })}
            value={textAlignmentValue(node.data.textAlign, t)}
          />
        </>
      );
    case 'image':
      return (
        <>
          <PropertyRow
            label={t('creativeStudio.canvas.properties.assetId', {
              defaultValue: '素材 ID',
            })}
            value={optionalValue(node.data.assetId, t)}
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.caption', {
              defaultValue: '说明',
            })}
            value={optionalValue(node.data.caption, t)}
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.altText', {
              defaultValue: '替代文本',
            })}
            value={optionalValue(node.data.alt, t)}
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.fit', {
              defaultValue: '适配',
            })}
            value={imageFitValue(node.data.fit, t)}
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.naturalSize', {
              defaultValue: '原始尺寸',
            })}
            value={
              node.data.naturalSize
                ? `${node.data.naturalSize.width} × ${node.data.naturalSize.height}`
                : t('creativeStudio.canvas.values.unresolved', {
                    defaultValue: '未解析',
                  })
            }
          />
        </>
      );
    case 'panorama':
      return (
        <>
          <PropertyRow
            label={t('creativeStudio.canvas.properties.assetId', {
              defaultValue: '素材 ID',
            })}
            value={optionalValue(node.data.assetId, t)}
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.projection', {
              defaultValue: '投影',
            })}
            value={t('creativeStudio.canvas.editor.projectionEquirectangular', {
              defaultValue: '等距柱状投影',
            })}
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.viewAngle', {
              defaultValue: '视角',
            })}
            value={t('creativeStudio.canvas.nodes.panorama.orientation', {
              yaw: node.data.yaw,
              pitch: node.data.pitch,
              defaultValue: '偏航 {{yaw}}° · 俯仰 {{pitch}}°',
            })}
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.fieldOfView', {
              defaultValue: '视野',
            })}
            value={`${node.data.fieldOfView}°`}
          />
        </>
      );
    case 'video':
      return (
        <>
          <PropertyRow
            label={t('creativeStudio.canvas.properties.assetId', {
              defaultValue: '素材 ID',
            })}
            value={optionalValue(node.data.assetId, t)}
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.posterAsset', {
              defaultValue: '封面素材',
            })}
            value={optionalValue(node.data.posterAssetId, t)}
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.autoplay', {
              defaultValue: '自动播放',
            })}
            value={booleanValue(node.data.autoplay, t)}
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.loop', {
              defaultValue: '循环',
            })}
            value={booleanValue(node.data.loop, t)}
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.muted', {
              defaultValue: '静音',
            })}
            value={booleanValue(node.data.muted, t)}
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.trim', {
              defaultValue: '裁剪',
            })}
            value={`${milliseconds(node.data.trimStartMs, t)} – ${milliseconds(
              node.data.trimEndMs,
              t
            )}`}
          />
        </>
      );
    case 'audio':
      return (
        <>
          <PropertyRow
            label={t('creativeStudio.canvas.properties.assetId', {
              defaultValue: '素材 ID',
            })}
            value={optionalValue(node.data.assetId, t)}
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.title', {
              defaultValue: '标题',
            })}
            value={optionalValue(node.data.title, t)}
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.volume', {
              defaultValue: '音量',
            })}
            value={`${Math.round(node.data.volume * 100)}%`}
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.loop', {
              defaultValue: '循环',
            })}
            value={booleanValue(node.data.loop, t)}
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.trim', {
              defaultValue: '裁剪',
            })}
            value={`${milliseconds(node.data.trimStartMs, t)} – ${milliseconds(
              node.data.trimEndMs,
              t
            )}`}
          />
        </>
      );
    case 'config':
      return (
        <>
          <PropertyRow
            label={t('creativeStudio.canvas.properties.task', {
              defaultValue: '任务',
            })}
            value={node.data.task}
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.capability', {
              defaultValue: '能力',
            })}
            value={node.data.capability}
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.provider', {
              defaultValue: '提供商',
            })}
            value={optionalValue(node.data.providerId, t)}
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.model', {
              defaultValue: '模型',
            })}
            value={optionalValue(node.data.model, t)}
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.status', {
              defaultValue: '状态',
            })}
            value={generationStatusValue(node.data.status, t)}
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.prompt', {
              defaultValue: '提示词',
            })}
            value={optionalValue(node.data.prompt, t)}
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.negativePrompt', {
              defaultValue: '负面提示词',
            })}
            value={optionalValue(node.data.negativePrompt, t)}
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.inputAssets', {
              defaultValue: '输入素材',
            })}
            value={t('creativeStudio.canvas.values.itemCount', {
              count: node.data.inputAssetIds.length,
              defaultValue: '{{count}} 项',
            })}
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.resultAssets', {
              defaultValue: '结果素材',
            })}
            value={t('creativeStudio.canvas.values.itemCount', {
              count: node.data.resultAssetIds.length,
              defaultValue: '{{count}} 项',
            })}
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.taskId', {
              defaultValue: '任务 ID',
            })}
            value={optionalValue(node.data.taskId, t)}
          />
        </>
      );
    case 'director':
      return (
        <>
          <PropertyRow
            label={t('creativeStudio.canvas.properties.sceneId', {
              defaultValue: '场景 ID',
            })}
            value={optionalValue(node.data.sceneId, t)}
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.cameraId', {
              defaultValue: '机位 ID',
            })}
            value={optionalValue(node.data.cameraId, t)}
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.currentTime', {
              defaultValue: '当前时间',
            })}
            value={milliseconds(node.data.timelineMs, t)}
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.duration', {
              defaultValue: '时长',
            })}
            value={milliseconds(node.data.durationMs, t)}
          />
        </>
      );
    case 'group':
      return (
        <>
          <PropertyRow
            label={t('creativeStudio.canvas.properties.title', {
              defaultValue: '标题',
            })}
            value={optionalValue(node.data.title, t)}
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.color', {
              defaultValue: '颜色',
            })}
            value={optionalValue(node.data.color, t)}
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.collapsed', {
              defaultValue: '已折叠',
            })}
            value={booleanValue(node.data.collapsed, t)}
          />
          <PropertyRow
            label={t('creativeStudio.canvas.properties.members', {
              defaultValue: '成员',
            })}
            value={t('creativeStudio.canvas.values.nodeCount', {
              count: memberCount,
              defaultValue: '{{count}} 个节点',
            })}
          />
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
  const { t } = useTranslation();
  switch (node.type) {
    case 'text':
      return (
        <>
          <PropertyEditorField
            label={t('creativeStudio.canvas.properties.content', {
              defaultValue: '内容',
            })}
          >
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
          <PropertyEditorField
            label={t('creativeStudio.canvas.properties.format', {
              defaultValue: '格式',
            })}
          >
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
              <option value='plain'>
                {t('creativeStudio.canvas.editor.plainText', {
                  defaultValue: '纯文本',
                })}
              </option>
              <option value='markdown'>
                {t('creativeStudio.canvas.editor.markdown', {
                  defaultValue: 'Markdown',
                })}
              </option>
            </select>
          </PropertyEditorField>
          <PropertyEditorField
            label={t('creativeStudio.canvas.properties.fontSize', {
              defaultValue: '字号',
            })}
          >
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
          <PropertyEditorField
            label={t('creativeStudio.canvas.properties.alignment', {
              defaultValue: '对齐',
            })}
          >
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
              <option value='left'>
                {t('creativeStudio.canvas.editor.alignLeft', {
                  defaultValue: '左对齐',
                })}
              </option>
              <option value='center'>
                {t('creativeStudio.canvas.editor.alignCenter', {
                  defaultValue: '居中',
                })}
              </option>
              <option value='right'>
                {t('creativeStudio.canvas.editor.alignRight', {
                  defaultValue: '右对齐',
                })}
              </option>
            </select>
          </PropertyEditorField>
        </>
      );
    case 'image':
      return (
        <>
          <PropertyEditorField
            label={t('creativeStudio.canvas.properties.caption', {
              defaultValue: '说明',
            })}
          >
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
          <PropertyEditorField
            label={t('creativeStudio.canvas.properties.altText', {
              defaultValue: '替代文本',
            })}
          >
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
          <PropertyEditorField
            label={t('creativeStudio.canvas.editor.fitMode', {
              defaultValue: '适配方式',
            })}
          >
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
              <option value='contain'>
                {t('creativeStudio.canvas.editor.fitContain', {
                  defaultValue: '完整显示',
                })}
              </option>
              <option value='cover'>
                {t('creativeStudio.canvas.editor.fitCover', {
                  defaultValue: '填满裁切',
                })}
              </option>
            </select>
          </PropertyEditorField>
        </>
      );
    case 'panorama':
      return (
        <>
          {([
            [
              t('creativeStudio.canvas.editor.horizontalAngle', {
                defaultValue: '水平视角',
              }),
              'yaw',
              -360,
              360,
            ],
            [
              t('creativeStudio.canvas.editor.verticalAngle', {
                defaultValue: '垂直视角',
              }),
              'pitch',
              -90,
              90,
            ],
            [
              t('creativeStudio.canvas.properties.fieldOfView', {
                defaultValue: '视野',
              }),
              'fieldOfView',
              10,
              150,
            ],
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
            [
              t('creativeStudio.canvas.properties.autoplay', {
                defaultValue: '自动播放',
              }),
              'autoplay',
            ],
            [
              t('creativeStudio.canvas.editor.loopPlayback', {
                defaultValue: '循环播放',
              }),
              'loop',
            ],
            [
              t('creativeStudio.canvas.properties.muted', {
                defaultValue: '静音',
              }),
              'muted',
            ],
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
          <PropertyEditorField
            label={t('creativeStudio.canvas.properties.title', {
              defaultValue: '标题',
            })}
          >
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
          <PropertyEditorField
            label={t('creativeStudio.canvas.editor.loopPlayback', {
              defaultValue: '循环播放',
            })}
          >
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
          <PropertyEditorField
            label={t('creativeStudio.canvas.editor.volumePercent', {
              percent: Math.round(node.data.volume * 100),
              defaultValue: '音量 {{percent}}%',
            })}
          >
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
          <PropertyEditorField
            label={t('creativeStudio.canvas.properties.prompt', {
              defaultValue: '提示词',
            })}
          >
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
          <PropertyEditorField
            label={t('creativeStudio.canvas.properties.negativePrompt', {
              defaultValue: '负面提示词',
            })}
          >
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
          <PropertyEditorField
            label={t('creativeStudio.canvas.editor.currentTimeMs', {
              defaultValue: '当前时间 (ms)',
            })}
          >
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
          <PropertyEditorField
            label={t('creativeStudio.canvas.editor.durationMs', {
              defaultValue: '时长 (ms)',
            })}
          >
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
          <PropertyEditorField
            label={t('creativeStudio.canvas.properties.title', {
              defaultValue: '标题',
            })}
          >
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
          <PropertyEditorField
            label={t('creativeStudio.canvas.properties.color', {
              defaultValue: '颜色',
            })}
          >
            <input
              value={node.data.color ?? ''}
              placeholder={t('creativeStudio.canvas.values.unset', {
                defaultValue: '未设置',
              })}
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
          <PropertyEditorField
            label={t('creativeStudio.canvas.editor.collapse', {
              defaultValue: '折叠',
            })}
          >
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
  const { t } = useTranslation();
  const selectedIds = new Set(state.selection.nodeIds);
  const selectedNodes = state.document.nodes.filter((node) => selectedIds.has(node.id));

  return (
    <section
      className={classNames(styles.panel, className)}
      data-canvas-product-panel='properties'
      aria-label={t('creativeStudio.canvas.properties.label', {
        defaultValue: '节点属性',
      })}
    >
      <header className={styles.panelHeader}>
        <div>
          <h2>
            {t('creativeStudio.canvas.properties.title', {
              defaultValue: '属性',
            })}
          </h2>
          <p>
            {selectedNodes.length > 0
              ? t('creativeStudio.canvas.properties.selectedCount', {
                  count: selectedNodes.length,
                  defaultValue: `已选择 ${selectedNodes.length} 个节点`,
                })
              : t('creativeStudio.canvas.properties.selectHint', {
                  defaultValue: '选择节点后查看详情',
                })}
          </p>
        </div>
      </header>

      {selectedNodes.length === 0 ? (
        <PanelEmpty
          icon={<Info {...iconProps} />}
          title={t('creativeStudio.canvas.properties.emptyTitle', {
            defaultValue: '未选择节点',
          })}
          description={t('creativeStudio.canvas.properties.emptyDescription', {
            defaultValue: '属性面板只展示当前画布中的真实节点数据。',
          })}
        />
      ) : selectedNodes.length > 1 ? (
        <div
          className={styles.selectionList}
          role='list'
          aria-label={t('creativeStudio.canvas.properties.selectedNodesLabel', {
            defaultValue: '已选择节点',
          })}
        >
          {selectedNodes.map((node) => (
            <button
              key={node.id}
              type='button'
              disabled={!onSelectNode}
              onClick={() => onSelectNode?.(node.id)}
            >
              <span className={styles.nodeIcon}>{nodeKindIcon(node.type)}</span>
              <span>
                <strong>{creativeCanvasNodeDisplayName(node, t)}</strong>
                <small>{nodeKindLabel(node.type, t)}</small>
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
                  <strong>{creativeCanvasNodeDisplayName(node, t)}</strong>
                  <span>{nodeKindLabel(node.type, t)}</span>
                </div>
              </div>
              <dl className={styles.propertyList}>
                <PropertyRow
                  label={t('creativeStudio.canvas.properties.nodeId', {
                    defaultValue: '节点 ID',
                  })}
                  value={node.id}
                />
                <PropertyRow
                  label={t('creativeStudio.canvas.properties.position', {
                    defaultValue: '位置',
                  })}
                  value={`${node.position.x}, ${node.position.y}`}
                />
                <PropertyRow
                  label={t('creativeStudio.canvas.properties.size', {
                    defaultValue: '尺寸',
                  })}
                  value={`${node.size.width} × ${node.size.height}`}
                />
                <PropertyRow
                  label={t('creativeStudio.canvas.properties.layer', {
                    defaultValue: '层级',
                  })}
                  value={node.zIndex}
                />
                <PropertyRow
                  label={t('creativeStudio.canvas.properties.groupId', {
                    defaultValue: '分组 ID',
                  })}
                  value={optionalValue(node.groupId, t)}
                />
                <PropertyRow
                  label={t('creativeStudio.canvas.properties.locked', {
                    defaultValue: '锁定',
                  })}
                  value={booleanValue(node.locked, t)}
                />
                <NodeDataProperties node={node} memberCount={memberCount} />
              </dl>
              {onUpdateNode ? (
                <div
                  className={styles.editorForm}
                  aria-label={t('creativeStudio.canvas.properties.editLabel', {
                    defaultValue: '编辑节点属性',
                  })}
                >
                  <h3>
                    {t('creativeStudio.canvas.properties.editTitle', {
                      defaultValue: '编辑',
                    })}
                  </h3>
                  <PropertyEditorField
                    label={t('creativeStudio.canvas.properties.lockNode', {
                      defaultValue: '锁定节点',
                    })}
                  >
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
                <p className={styles.readOnlyNote}>
                  {t('creativeStudio.canvas.properties.readOnlyNote', {
                    defaultValue: '当前属性面板未连接 canonical 更新命令。',
                  })}
                </p>
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
}) => {
  const { t } = useTranslation();
  return (
    <section
      className={classNames(styles.panel, styles.historyPanel, className)}
      data-canvas-product-panel='history'
      aria-label={t('creativeStudio.canvas.history.label', {
        defaultValue: '编辑历史',
      })}
    >
      <header className={styles.panelHeader}>
        <div>
          <h2>
            {t('creativeStudio.canvas.history.title', {
              defaultValue: '编辑历史',
            })}
          </h2>
          <p>
            {t('creativeStudio.canvas.history.subtitle', {
              defaultValue: '当前会话的 reducer 快照',
            })}
          </p>
        </div>
      </header>
      <div className={styles.historySummary}>
        <div>
          <span>
            {t('creativeStudio.canvas.history.undoable', {
              defaultValue: '可撤销',
            })}
          </span>
          <strong>{state.history.past.length}</strong>
        </div>
        <div>
          <span>
            {t('creativeStudio.canvas.history.redoable', {
              defaultValue: '可重做',
            })}
          </span>
          <strong>{state.history.future.length}</strong>
        </div>
        <div className={styles.historyActions}>
          <button
            type='button'
            disabled={state.history.past.length === 0}
            onClick={onUndo}
          >
            <Undo {...iconProps} />
            {t('creativeStudio.canvas.history.undo', {
              defaultValue: '撤销',
            })}
          </button>
          <button
            type='button'
            disabled={state.history.future.length === 0}
            onClick={onRedo}
          >
            <Redo {...iconProps} />
            {t('creativeStudio.canvas.history.redo', {
              defaultValue: '重做',
            })}
          </button>
        </div>
      </div>
      <p className={styles.historyDisclosure}>
        {t('creativeStudio.canvas.history.disclosure', {
          defaultValue:
            '核心只保存文档快照，没有操作名称和时间戳；本面板不会臆造历史记录。',
        })}
      </p>
    </section>
  );
};

export type CreativeCanvasUnavailableKind = 'assistant' | 'templates' | 'generic';

export interface CreativeCanvasUnavailablePanelProps {
  kind?: CreativeCanvasUnavailableKind;
  title: string;
  description: string;
  detail?: string;
  className?: string;
}

function unavailableIcon(kind: CreativeCanvasUnavailableKind): React.ReactNode {
  if (kind === 'assistant') return <Robot {...iconProps} />;
  if (kind === 'templates') return <Workbench {...iconProps} />;
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
}) => {
  const { t } = useTranslation();
  return (
    <CreativeCanvasUnavailablePanel
      kind='assistant'
      className={className}
      title={t('creativeStudio.canvas.unavailable.agentTitle', {
        defaultValue: '创作 Agent 尚未连接',
      })}
      description={t('creativeStudio.canvas.unavailable.agentDescription', {
        defaultValue:
          '当前没有可验证的画布专属会话绑定，因此不会发送消息或复用主聊天会话。',
      })}
      detail={t('creativeStudio.canvas.unavailable.agentDetail', {
        defaultValue:
          '需要接入 canvas/session resolver、真实消息历史和独占会话所有权后才能启用。',
      })}
    />
  );
};

export const CreativeCanvasTemplateUnwiredPanel: React.FC<{ className?: string }> = ({
  className,
}) => {
  const { t } = useTranslation();
  return (
    <CreativeCanvasUnavailablePanel
      kind='templates'
      className={className}
      title={t('creativeStudio.canvas.unavailable.templatesTitle', {
        defaultValue: '模板尚未连接',
      })}
      description={t(
        'creativeStudio.canvas.unavailable.templatesDescription',
        {
          defaultValue:
            '当前画布文档没有模板数据源，本面板不会显示示例模板或虚构运行状态。',
        }
      )}
    />
  );
};

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
