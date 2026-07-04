/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IConversationArtifact } from '@/common/adapter/ipcBridge';
import type { IMessageAcpToolCall, IMessageToolCall, IMessageToolGroup, TMessage } from '@/common/chat/chatLib';
import { normalizeToolMessages } from '@/common/chat/normalizeToolCall';
import { usePreviewLauncher } from '@/renderer/hooks/file/usePreviewLauncher';
import { extractContentFromDiff } from '@/renderer/utils/file/diffUtils';
import { getFileTypeInfo } from '@/renderer/utils/file/fileType';
import MessageAcpPermission from '@renderer/pages/conversation/Messages/acp/MessageAcpPermission';
import classNames from 'classnames';
import React, { useCallback, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import type { FileChangeInfo } from '../MessageFileChanges';
import type { TurnDisclosureProcessState } from '../turnDisclosureModel';
import { getProcessItemState } from '../turnProcessState';
import MessagePermission from './MessagePermission';
import { buildToolReceiptDetailRows, type ToolReceiptDetailRow } from './toolGroupSummaryModel';

type ToolProcessMessage = IMessageToolGroup | IMessageAcpToolCall | IMessageToolCall;

export type ProcessTraceRenderableItem =
  | TMessage
  | {
      type: 'file_summary';
      id: string;
      diffs: FileChangeInfo[];
    }
  | {
      type: 'tool_summary';
      id: string;
      messages: ToolProcessMessage[];
    }
  | {
      type: 'artifact';
      id: string;
      artifact: IConversationArtifact;
    };

type TranslationFn = ReturnType<typeof useTranslation>['t'];

type ProcessTraceRow = {
  key: string;
  label: string;
  state: TurnDisclosureProcessState;
  onClick?: () => void;
};

const defaultToolSummaryByState: Record<TurnDisclosureProcessState, string> = {
  completed: 'Ran {{target}}',
  running: 'Running {{target}}',
  waiting: 'Waiting to confirm {{target}}',
  failed: 'Failed {{target}}',
  canceled: 'Canceled {{target}}',
};

const compactReceiptText = (value: unknown, fallback: string): string => {
  if (typeof value !== 'string') return fallback;
  const compacted = value.replace(/\s+/g, ' ').trim();
  return compacted || fallback;
};

const joinCompactText = (parts: Array<string | undefined>): string => parts.filter(Boolean).join(' ');

export const formatProcessDuration = (ms: number, t: TranslationFn): string => {
  const totalSeconds = Math.max(0, Math.round(ms / 1000));
  const sUnit = t('common.unit.second_short', { defaultValue: 's' });
  const mUnit = t('common.unit.minute_short', { defaultValue: 'm' });
  const hUnit = t('common.unit.hour_short', { defaultValue: 'h' });

  if (totalSeconds < 60) return `${totalSeconds}${sUnit}`;
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  if (minutes < 60) return `${minutes}${mUnit} ${seconds}${sUnit}`;
  const hours = Math.floor(minutes / 60);
  const remainingMinutes = minutes % 60;
  return `${hours}${hUnit} ${remainingMinutes}${mUnit}`;
};

const formatToolReceiptDetailLabel = (row: ToolReceiptDetailRow, t: TranslationFn): string => {
  if ((row.state === 'failed' || row.state === 'canceled') && row.target) {
    return t(`messages.toolSummary.${row.state}`, {
      target: row.target,
      defaultValue: defaultToolSummaryByState[row.state],
    });
  }

  if (row.action === 'run_commands' && row.target) {
    return t(`messages.toolSummary.${row.state}`, {
      target: row.target,
      defaultValue: defaultToolSummaryByState[row.state],
    });
  }

  if (row.action === 'search_code') {
    return row.target
      ? t('messages.processReceipt.searchedTarget', {
          target: row.target,
          defaultValue: 'Searched {{target}}',
        })
      : t('messages.processReceipt.searchedCode', { defaultValue: 'Searched code' });
  }

  if (row.action === 'list_files') {
    return row.target
      ? t('messages.processReceipt.listedTarget', {
          target: row.target,
          defaultValue: 'Listed {{target}}',
        })
      : t('messages.processReceipt.listedFiles', { defaultValue: 'Listed files' });
  }

  if (row.action === 'load_tools') {
    return row.target
      ? t('messages.processReceipt.loadedTarget', {
          target: row.target,
          defaultValue: 'Loaded {{target}}',
        })
      : t('messages.processReceipt.loadedTools', {
          count: 1,
          defaultValue: 'Loaded {{count}} tools',
        });
  }

  return joinCompactText([row.title, row.target]);
};

const formatFileChangeStats = (file: FileChangeInfo): string =>
  joinCompactText([
    file.insertions > 0 ? `+${file.insertions}` : undefined,
    file.deletions > 0 ? `-${file.deletions}` : undefined,
  ]);

const ProcessTraceRows: React.FC<{ rows: ProcessTraceRow[] }> = ({ rows }) => {
  if (!rows.length) return null;

  return (
    <div className='turn-process-trace'>
      {rows.map((row) => {
        const className = classNames('turn-process-trace__row', `turn-process-trace__row--${row.state}`);
        const text = (
          <span className='turn-process-trace__text' title={row.label}>
            {row.label}
          </span>
        );

        if (row.onClick) {
          return (
            <button key={row.key} type='button' className={className} onClick={row.onClick}>
              {text}
            </button>
          );
        }

        return (
          <div key={row.key} className={className}>
            {text}
          </div>
        );
      })}
    </div>
  );
};

const ToolProcessTraceRows: React.FC<{ messages: ToolProcessMessage[] }> = ({ messages }) => {
  const { t } = useTranslation();
  const tools = useMemo(() => normalizeToolMessages(messages), [messages]);
  const rows = useMemo<ProcessTraceRow[]>(
    () =>
      buildToolReceiptDetailRows(tools).map((row) => ({
        key: row.key,
        state: row.state,
        label: formatToolReceiptDetailLabel(row, t),
      })),
    [t, tools]
  );

  return <ProcessTraceRows rows={rows} />;
};

const FileProcessTraceRows: React.FC<{ diffs: FileChangeInfo[] }> = ({ diffs }) => {
  const { t } = useTranslation();
  const { launchPreview } = usePreviewLauncher();
  const files = useMemo(() => Array.from(new Map(diffs.map((file) => [file.fullPath, file])).values()), [diffs]);

  const openFile = useCallback(
    (file: FileChangeInfo) => {
      const { contentType, editable, language } = getFileTypeInfo(file.file_name);
      void launchPreview({
        relativePath: file.fullPath,
        file_name: file.file_name,
        contentType,
        editable,
        language,
        fallbackContent: editable ? extractContentFromDiff(file.diff) : undefined,
        diffContent: file.diff,
      });
    },
    [launchPreview]
  );

  const rows = useMemo<ProcessTraceRow[]>(
    () =>
      files.map((file) => {
        const stats = formatFileChangeStats(file);
        return {
          key: file.fullPath,
          state: 'completed',
          label: compactReceiptText(
            t('messages.processReceipt.fileChanged', {
              target: file.fullPath,
              stats,
              defaultValue: 'Edited {{target}} {{stats}}',
            }),
            file.fullPath
          ),
          onClick: () => openFile(file),
        };
      }),
    [files, openFile, t]
  );

  return <ProcessTraceRows rows={rows} />;
};

const ThinkingProcessTraceRows: React.FC<{ message: Extract<TMessage, { type: 'thinking' }> }> = ({ message }) => {
  const { t } = useTranslation();
  const state = getProcessItemState(message);
  const duration = message.content.duration ?? (message.content as { duration_ms?: number }).duration_ms;
  const label =
    state === 'running'
      ? compactReceiptText(
          message.content.subject,
          t('messages.processReceipt.thinkingRunning', { defaultValue: 'Thinking' })
        )
      : typeof duration === 'number'
        ? t('messages.processReceipt.thinkingCompletedWithDuration', {
            duration: formatProcessDuration(duration, t),
            defaultValue: 'Thought complete · {{duration}}',
          })
        : t('messages.processReceipt.thinkingCompleted', { defaultValue: 'Thought' });
  const content = message.content.content.trim();

  return (
    <div className='turn-process-trace'>
      <div className={classNames('turn-process-trace__row', `turn-process-trace__row--${state}`)}>
        <span className='turn-process-trace__text' title={label}>
          {label}
        </span>
      </div>
      {content && <div className='turn-process-trace__paragraph'>{content}</div>}
    </div>
  );
};

const getUnhandledMessageType = (_message: never): string => 'unknown';

const ProcessTraceItem: React.FC<{ item: ProcessTraceRenderableItem }> = ({ item }) => {
  const { t } = useTranslation();
  const state = getProcessItemState(item);

  if ('type' in item && item.type === 'artifact') {
    const target =
      item.artifact.kind === 'cron_trigger' ? item.artifact.payload.cron_job_name : item.artifact.payload.name;
    return (
      <ProcessTraceRows
        rows={[
          {
            key: item.id,
            state,
            label: t('messages.processReceipt.status', { target, defaultValue: '{{target}}' }),
          },
        ]}
      />
    );
  }

  if ('type' in item && item.type === 'file_summary') {
    return <FileProcessTraceRows diffs={item.diffs} />;
  }

  if ('type' in item && item.type === 'tool_summary') {
    return <ToolProcessTraceRows messages={item.messages} />;
  }

  switch (item.type) {
    case 'text':
      return (
        <div className='turn-process-trace'>
          <div className='turn-process-trace__paragraph'>{item.content.content}</div>
        </div>
      );
    case 'tips':
      return (
        <ProcessTraceRows
          rows={[
            {
              key: item.id,
              state,
              label: compactReceiptText(
                item.content.content,
                t('messages.processReceipt.status', {
                  target: t('messages.processing'),
                  defaultValue: '{{target}}',
                })
              ),
            },
          ]}
        />
      );
    case 'tool_call':
    case 'tool_group':
    case 'acp_tool_call':
      return <ToolProcessTraceRows messages={[item]} />;
    case 'agent_status':
      return (
        <ProcessTraceRows
          rows={[
            {
              key: item.id,
              state,
              label:
                state === 'failed'
                  ? t('messages.processReceipt.agentFailed', {
                      target: item.content.agent_name || item.content.backend,
                      defaultValue: '{{target}} failed',
                    })
                  : t('messages.processReceipt.agentConnecting', {
                      target: item.content.agent_name || item.content.backend,
                      defaultValue: 'Connecting {{target}}',
                    }),
            },
          ]}
        />
      );
    case 'permission':
      if (state === 'waiting') return <MessagePermission message={item} />;
      return (
        <ProcessTraceRows
          rows={[
            {
              key: item.id,
              state,
              label: t('messages.processReceipt.waitingPermission', {
                target: compactReceiptText(
                  item.content.title || item.content.description,
                  t('messages.permissionRequest')
                ),
                defaultValue: 'Waiting to confirm {{target}}',
              }),
            },
          ]}
        />
      );
    case 'acp_permission':
      if (state === 'waiting') return <MessageAcpPermission message={item} />;
      return (
        <ProcessTraceRows
          rows={[
            {
              key: item.id,
              state,
              label: t('messages.processReceipt.waitingPermission', {
                target: compactReceiptText(
                  item.content.tool_call?.title ||
                    item.content.tool_call?.raw_input?.command ||
                    item.content.tool_call?.raw_input?.description,
                  t('messages.permissionRequest')
                ),
                defaultValue: 'Waiting to confirm {{target}}',
              }),
            },
          ]}
        />
      );
    case 'thinking':
      return <ThinkingProcessTraceRows message={item} />;
    case 'plan':
    case 'available_commands':
      return null;
    default:
      return <div>{t('messages.unknownMessageType', { type: getUnhandledMessageType(item) })}</div>;
  }
};

export default ProcessTraceItem;
