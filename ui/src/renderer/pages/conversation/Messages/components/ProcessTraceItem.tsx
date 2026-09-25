/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IMessageToolCall, IMessageToolGroup, TMessage } from '@/common/chat/chatLib';
import { toDisplayText } from '@/common/chat/displayText';
import { normalizeToolMessages } from '@/common/chat/normalizeToolCall';
import { useConversationContextSafe } from '@/renderer/hooks/context/ConversationContext';
import { usePreviewLauncher } from '@/renderer/hooks/file/usePreviewLauncher';
import { extractContentFromDiff } from '@/renderer/utils/file/diffUtils';
import { getFileTypeInfo } from '@/renderer/utils/file/fileType';
import MarkdownView from '@renderer/components/Markdown';
import { hasSkillSuggest, stripSkillSuggest } from '@renderer/utils/chat/skillSuggestParser';
import { hasThinkTags, stripThinkTags } from '@renderer/utils/chat/thinkTagFilter';
import { Attention, Code, Edit, Info, Right, Terminal } from '@icon-park/react';
import classNames from 'classnames';
import React, { useCallback, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { FileChangeInfo } from '../MessageFileChanges';
import { isContextCompressionTip } from '../processTipModel';
import { formatFileTargetPreview, formatWorkspaceFileTarget } from '../processFileTargetLabel';
import {
  isFileReceiptRow,
  shouldShowFileListDetail,
  shouldShowToolRowDetail,
} from '../processTraceDisplayModel';
import type { TurnDisclosureProcessState } from '../turnDisclosureModel';
import type { MessageId } from '@/common/types/ids';
import { getProcessItemState, mergeProcessStates } from '../turnProcessState';
import { projectAssistantText, stripInternalToolCallPayload } from '../processTraceDisplayModel';
import AssistantProtocolNotice from './AssistantProtocolNotice';
import { MESSAGE_BODY_FONT_SIZE, MESSAGE_BODY_LINE_HEIGHT } from '../typography';
import {
  buildToolReceiptDetailRows,
  type ToolReceiptAction,
  type ToolReceiptDetailRow,
} from './toolGroupSummaryModel';

type ToolProcessMessage = IMessageToolGroup | IMessageToolCall;

export type ProcessTraceRenderableItem =
  | TMessage
  | {
      type: 'file_summary';
      id: string;
      msg_id?: MessageId;
      diffs: FileChangeInfo[];
      sourceMessageIds: string[];
      created_at: number;
    }
  | {
      type: 'tool_summary';
      id: string;
      msg_id?: MessageId;
      messages: ToolProcessMessage[];
      sourceMessageIds: string[];
      created_at: number;
    };

type TranslationFn = ReturnType<typeof useTranslation>['t'];

type ProcessTraceVariant = 'list' | 'receipt';
type ProcessTraceIconKind = 'system' | 'tool' | 'command' | 'file' | 'edit';
type ProcessTracePresentationState = TurnDisclosureProcessState | 'recovered';
type LabeledToolRow = {
  row: ToolReceiptDetailRow;
  label: string;
  presentationState?: ProcessTracePresentationState;
};

type ProcessTraceRow = {
  key: string;
  label: string;
  title?: string;
  state: ProcessTracePresentationState;
  onClick?: () => void;
  iconKind?: ProcessTraceIconKind;
};

const defaultToolSummaryByState: Record<TurnDisclosureProcessState, string> = {
  completed: 'Ran {{target}}',
  running: 'Running {{target}}',
  failed: 'Failed {{target}}',
  canceled: 'Canceled {{target}}',
};

const compactReceiptText = (value: unknown, fallback: string): string => {
  if (typeof value !== 'string') return fallback;
  const compacted = value.replace(/\s+/g, ' ').trim();
  return compacted || fallback;
};

const getPublicProcessNarration = (value: unknown): string => {
  let content = toDisplayText(value).trim();
  if (hasThinkTags(content)) content = stripThinkTags(content);
  if (hasSkillSuggest(content)) content = stripSkillSuggest(content);
  return stripInternalToolCallPayload(content);
};

const joinCompactText = (parts: Array<string | undefined>): string => parts.filter(Boolean).join(' ');

const compactRepeatedToolRows = (rows: LabeledToolRow[], t: TranslationFn): LabeledToolRow[] => {
  const grouped = new Map<string, { item: LabeledToolRow; count: number }>();
  for (const item of rows) {
    const key = [item.presentationState ?? item.row.state, item.row.action, item.row.target ?? '', item.label].join('\u0000');
    const existing = grouped.get(key);
    if (existing) existing.count += 1;
    else grouped.set(key, { item, count: 1 });
  }
  if (grouped.size === rows.length) return rows;
  return Array.from(grouped.values()).map(({ item, count }) => count === 1 ? item : ({
    ...item,
    label: t('messages.processReceipt.repeatedOperation', {
      label: item.label,
      count,
      defaultValue: '{{label}} · {{count}} times',
    }),
  }));
};

const TraceRowIcon: React.FC<{ kind?: ProcessTraceIconKind }> = ({ kind = 'system' }) => {
  const props = {
    theme: 'outline' as const,
    size: '13',
    fill: 'currentColor',
  };

  return (
    <span className='turn-process-trace__row-icon' aria-hidden='true'>
      {kind === 'command' ? (
        <Terminal {...props} />
      ) : kind === 'file' ? (
        <Code {...props} />
      ) : kind === 'edit' ? (
        <Edit {...props} />
      ) : kind === 'tool' ? (
        <Code {...props} />
      ) : (
        <Info {...props} />
      )}
    </span>
  );
};

const getToolTraceIconKind = (action: ToolReceiptAction): ProcessTraceIconKind => {
  if (action === 'run_commands') return 'command';
  if (action === 'edit_files') return 'edit';
  if (action === 'read_files' || action === 'search_code' || action === 'list_files') return 'file';
  return 'tool';
};

const getToolReceiptDetailDisplayTarget = (row: ToolReceiptDetailRow, workspaceRoots: string[]): string | undefined => {
  if (!row.target) return undefined;
  if (row.action !== 'read_files' && row.action !== 'edit_files') return row.target;
  return formatWorkspaceFileTarget(row.target, { workspaceRoots }).label;
};

const formatToolReceiptDetailLabel = (
  row: ToolReceiptDetailRow,
  t: TranslationFn,
  workspaceRoots: string[]
): string => {
  const displayTarget = getToolReceiptDetailDisplayTarget(row, workspaceRoots);

  if (row.skipped) {
    return t('messages.toolSummary.skipped', {
      target: displayTarget ?? row.title,
      defaultValue: 'Skipped {{target}}',
    });
  }

  if (row.notExecutedReason === 'invalid_arguments') {
    return t('messages.toolSummary.invalidArguments', {
      target: displayTarget ?? row.title,
      defaultValue: 'Arguments did not pass validation; {{target}} was not run',
    });
  }

  if (row.notExecutedReason === 'runtime_preflight') {
    return t('messages.toolSummary.notExecuted', {
      target: displayTarget ?? row.title,
      defaultValue: 'Did not run {{target}}',
    });
  }

  if ((row.state === 'failed' || row.state === 'canceled') && displayTarget) {
    return t(`messages.toolSummary.${row.state}`, {
      target: displayTarget,
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

  if (row.action === 'read_files' && displayTarget) {
    return compactReceiptText(
      t('messages.processReceipt.fileRead', {
        target: displayTarget,
        defaultValue: 'Read {{target}}',
      }),
      displayTarget
    );
  }

  if (row.action === 'edit_files' && displayTarget) {
    return compactReceiptText(
      t('messages.processReceipt.fileChanged', {
        target: displayTarget,
        stats: '',
        defaultValue: 'Edited {{target}}',
      }),
      displayTarget
    );
  }

  return joinCompactText([row.title, displayTarget]);
};

const formatFileChangeStats = (file: FileChangeInfo): string =>
  joinCompactText([
    file.insertions > 0 ? `+${file.insertions}` : undefined,
    file.deletions > 0 ? `-${file.deletions}` : undefined,
  ]);

const formatTargetPreview = (targets: string[], workspaceRoots: string[]): string =>
  formatFileTargetPreview(targets, { workspaceRoots });

const getToolFileListTargets = (rows: ToolReceiptDetailRow[]): string[] =>
  Array.from(new Set(rows.map((row) => row.target).filter((target): target is string => Boolean(target))));

const formatToolFileListLabel = (
  rows: ToolReceiptDetailRow[],
  t: TranslationFn,
  workspaceRoots: string[]
): string => {
  const targets = getToolFileListTargets(rows);
  const targetPreview = formatTargetPreview(targets, workspaceRoots);
  const hasReadRows = rows.some((row) => row.action === 'read_files');
  const hasEditRows = rows.some((row) => row.action === 'edit_files');

  if (hasEditRows && !hasReadRows) {
    return t('messages.processReceipt.fileEdits', {
      count: targets.length,
      defaultValue: 'Edited {{count}} files',
    });
  }

  if (hasReadRows && !hasEditRows) {
    return t('messages.processReceipt.readFiles', {
      count: targets.length,
      defaultValue: 'Read {{count}} files',
    });
  }

  return t('messages.processReceipt.fileTargets', {
    count: targets.length,
    target: targetPreview,
    defaultValue: 'Handled {{count}} files: {{target}}',
  });
};

const ToolFileListDetail: React.FC<{
  rows: ToolReceiptDetailRow[];
  workspaceRoots: string[];
  showLabel?: boolean;
}> = ({
  rows,
  workspaceRoots,
  showLabel = true,
}) => {
  const { t } = useTranslation();
  const targets = getToolFileListTargets(rows);
  if (!targets.length) return null;

  const label = formatToolFileListLabel(rows, t, workspaceRoots);

  return (
    <div className='turn-process-trace-detail'>
      {showLabel && <div className='turn-process-trace-detail__label'>{label}</div>}
      <ul className='turn-process-trace-file-list'>
        {targets.map((target) => {
          const display = formatWorkspaceFileTarget(target, { workspaceRoots });
          return (
            <li key={target} className='turn-process-trace-file-list__item' title={display.title}>
              {display.title}
            </li>
          );
        })}
      </ul>
    </div>
  );
};

const ToolFileGroupTraceRow: React.FC<{ rows: ToolReceiptDetailRow[]; workspaceRoots: string[]; currentActivity?: boolean }> = ({
  rows,
  workspaceRoots,
  currentActivity = false,
}) => {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const targets = getToolFileListTargets(rows);
  if (!targets.length) return null;

  const label = formatToolFileListLabel(rows, t, workspaceRoots);
  const state = mergeProcessStates(rows.map((row) => row.state));

  return (
    <div className='turn-process-trace-tool'>
      <button
        type='button'
        className={classNames(
          'turn-process-trace__row',
          'turn-process-trace-tool__toggle',
          currentActivity && 'turn-process-trace__row--current-activity',
          `turn-process-trace__row--${state}`
        )}
        onClick={() => setExpanded((value) => !value)}
        aria-expanded={expanded}
      >
        <TraceRowIcon kind={getToolTraceIconKind(rows[0]?.action ?? 'read_files')} />
        <span className='turn-process-trace__text' title={targets.join('\n')}>
          {label}
        </span>
        <Right
          theme='outline'
          size='12'
          className={classNames('turn-process-trace-tool__arrow', expanded && 'turn-process-trace-tool__arrow--open')}
        />
      </button>
      {expanded && <ToolFileListDetail rows={rows} workspaceRoots={workspaceRoots} showLabel={false} />}
    </div>
  );
};

const ToolTraceDetailSection: React.FC<{ label: string; value?: string }> = ({ label, value }) => {
  if (!value) return null;
  return (
    <div className='turn-process-trace-detail__section'>
      <div className='turn-process-trace-detail__label'>{label}</div>
      <pre className='turn-process-trace-detail__content'>{value}</pre>
    </div>
  );
};

const ToolTraceDetail: React.FC<{ row: ToolReceiptDetailRow; workspaceRoots: string[] }> = ({ row, workspaceRoots }) => {
  const { t } = useTranslation();
  const command = row.action === 'run_commands' ? row.target : undefined;
  const input = row.input && row.input !== command ? row.input : undefined;

  if (row.attempts?.length) {
    return (
      <div className='turn-process-trace-detail'>
        {row.attempts.map((attempt) => (
          <div key={attempt.key} className='turn-process-trace-detail__attempt'>
            <div className='turn-process-trace-detail__label'>
              {t('messages.toolRetryAttempt', {
                number: attempt.attemptNo,
                defaultValue: 'Attempt {{number}}',
              })}
            </div>
            <>
              <ToolTraceDetailSection
                label={t('messages.toolDetailInput', { defaultValue: 'Input' })}
                value={attempt.input}
              />
              <ToolTraceDetailSection
                label={t('messages.toolDetailOutput', { defaultValue: 'Output' })}
                value={attempt.output}
              />
            </>
            {attempt.truncated && (
              <div className='turn-process-trace-detail__label'>
                {t('messages.toolDetailLoadFailed', { defaultValue: 'Full output was truncated' })}
              </div>
            )}
          </div>
        ))}
      </div>
    );
  }

  if (isFileReceiptRow(row) && row.state !== 'failed' && row.state !== 'canceled') {
    return (
      <div className='turn-process-trace-detail'>
        <ToolFileListDetail rows={[row]} workspaceRoots={workspaceRoots} />
        <ToolTraceDetailSection
          label={t('messages.toolDetailOutput', { defaultValue: 'Output' })}
          value={row.output}
        />
      </div>
    );
  }

  return (
    <div className='turn-process-trace-detail'>
      <ToolTraceDetailSection
        label={t('messages.command', { defaultValue: 'Command:' })}
        value={command}
      />
      <ToolTraceDetailSection
        label={t('messages.toolDetailInput', { defaultValue: 'Input' })}
        value={input}
      />
      <ToolTraceDetailSection
        label={t('messages.toolDetailOutput', { defaultValue: 'Output' })}
        value={row.output}
      />
      {row.truncated && (
        <div className='turn-process-trace-detail__label'>
          {t('messages.toolDetailLoadFailed', { defaultValue: 'Full output was truncated' })}
        </div>
      )}
    </div>
  );
};

const ToolTraceRow: React.FC<{
  row: ToolReceiptDetailRow;
  label: string;
  workspaceRoots: string[];
  fileRowCount?: number;
  currentActivity?: boolean;
  presentationState?: ProcessTracePresentationState;
}> = ({
  row,
  label,
  workspaceRoots,
  fileRowCount,
  currentActivity = false,
  presentationState,
}) => {
  const [expanded, setExpanded] = useState(false);
  const hasDetail = shouldShowToolRowDetail(row, { fileRowCount });
  const visualState = presentationState ?? row.state;
  const rowClassName = classNames(
    'turn-process-trace__row',
    'turn-process-trace-tool__toggle',
    currentActivity && 'turn-process-trace__row--current-activity',
    `turn-process-trace__row--${visualState}`
  );

  if (!hasDetail) {
    return (
      <div className='turn-process-trace-tool'>
        <div className={classNames('turn-process-trace__row', currentActivity && 'turn-process-trace__row--current-activity', `turn-process-trace__row--${visualState}`)}>
          <TraceRowIcon kind={getToolTraceIconKind(row.action)} />
          <span className='turn-process-trace__text' title={row.target ?? label}>
            {label}
          </span>
        </div>
      </div>
    );
  }

  return (
    <div className='turn-process-trace-tool'>
      <button
        type='button'
        className={rowClassName}
        onClick={() => setExpanded((value) => !value)}
        aria-expanded={expanded}
      >
        <TraceRowIcon kind={getToolTraceIconKind(row.action)} />
        <span className='turn-process-trace__text' title={row.target ?? label}>
          {label}
        </span>
        <Right
          theme='outline'
          size='12'
          className={classNames('turn-process-trace-tool__arrow', expanded && 'turn-process-trace-tool__arrow--open')}
        />
      </button>
      {expanded && <ToolTraceDetail row={row} workspaceRoots={workspaceRoots} />}
    </div>
  );
};

const ProcessTraceRows: React.FC<{ rows: ProcessTraceRow[] }> = ({ rows }) => {
  if (!rows.length) return null;

  return (
    <div className='turn-process-trace'>
      {rows.map((row) => {
        const className = classNames('turn-process-trace__row', `turn-process-trace__row--${row.state}`);
        const text = (
          <span className='turn-process-trace__text' title={row.title ?? row.label}>
            {row.label}
          </span>
        );

        if (row.onClick) {
          return (
            <button key={row.key} type='button' className={className} onClick={row.onClick}>
              <TraceRowIcon kind={row.iconKind ?? 'system'} />
              {text}
            </button>
          );
        }

        return (
          <div key={row.key} className={className}>
            <TraceRowIcon kind={row.iconKind ?? 'system'} />
            {text}
          </div>
        );
      })}
    </div>
  );
};

const FailedToolTraceGroup: React.FC<{
  rows: Array<{ row: ToolReceiptDetailRow; label: string }>;
  workspaceRoots: string[];
  recovered?: boolean;
}> = ({ rows, workspaceRoots, recovered = false }) => {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  if (!rows.length) return null;

  return (
    <div
      className={classNames(
        'turn-process-trace-tool',
        recovered ? 'turn-process-trace-tool--recovered-group' : 'turn-process-trace-tool--failed-group'
      )}
    >
      <button
        type='button'
        className={classNames(
          'turn-process-trace__row turn-process-trace-tool__toggle',
          recovered ? 'turn-process-trace__row--recovered' : 'turn-process-trace__row--failed'
        )}
        onClick={() => setExpanded((value) => !value)}
        aria-expanded={expanded}
      >
        <span className='turn-process-trace__row-icon' aria-hidden='true'>
          <Attention theme='outline' size='13' fill='currentColor' />
        </span>
        <span className='turn-process-trace__text'>
          {recovered
            ? t('messages.processReceipt.recoveredOperations', {
                count: rows.length,
                defaultValue: '{{count}} operations encountered an error',
              })
            : t('messages.processReceipt.failedOperations', {
                count: rows.length,
                defaultValue: '{{count}} operations did not complete',
              })}
        </span>
        <Right
          theme='outline'
          size='12'
          className={classNames('turn-process-trace-tool__arrow', expanded && 'turn-process-trace-tool__arrow--open')}
        />
      </button>
      {expanded && (
        <div className='turn-process-trace-detail'>
          {rows.map(({ row, label }) => (
            <ToolTraceRow
              key={row.key}
              row={row}
              label={label}
              workspaceRoots={workspaceRoots}
              presentationState={recovered ? 'recovered' : undefined}
            />
          ))}
        </div>
      )}
    </div>
  );
};

const ToolProcessTraceRows: React.FC<{
  messages: ToolProcessMessage[];
  variant?: ProcessTraceVariant;
  workspaceRoots: string[];
  stateOverride?: TurnDisclosureProcessState;
  recoverFailures?: boolean;
}> = ({
  messages,
  variant = 'list',
  workspaceRoots,
  stateOverride,
  recoverFailures = false,
}) => {
  const { t } = useTranslation();
  const tools = useMemo(() => normalizeToolMessages(messages), [messages]);
  const rows = useMemo(
    () =>
      buildToolReceiptDetailRows(tools).map((row) => {
        // Closed turns settle only stale running rows. Completed results and
        // failures inside a mixed group retain their own lifecycle state.
        const effectiveRow = stateOverride && row.state === 'running' && !row.notExecutedReason
          ? { ...row, state: stateOverride }
          : row;
        const baseLabel = formatToolReceiptDetailLabel(effectiveRow, t, workspaceRoots);
        const recoveredAttemptCount = effectiveRow.attempts?.filter(
          (attempt) => attempt.state === 'failed'
        ).length ?? 0;
        return {
          row: effectiveRow,
          label: recoveredAttemptCount > 0 && effectiveRow.state === 'completed'
            ? t('messages.processReceipt.recoveredAfterRetry', {
                target: getToolReceiptDetailDisplayTarget(effectiveRow, workspaceRoots) ?? effectiveRow.title,
                count: recoveredAttemptCount,
                defaultValue: '{{target}} recovered after {{count}} retries',
              })
            : effectiveRow.retryCount
              ? `${baseLabel} · ${t('messages.toolRetryCount', {
                  count: effectiveRow.retryCount,
                  defaultValue: 'Retried {{count}} times',
                })}`
              : baseLabel,
          ...(recoveredAttemptCount > 0 && effectiveRow.state === 'completed'
            ? { presentationState: 'recovered' as const }
            : recoverFailures && effectiveRow.state === 'failed'
              ? { presentationState: 'recovered' as const }
            : {}),
        };
      }),
    [recoverFailures, stateOverride, t, tools, workspaceRoots]
  );

  const fileRows = rows.filter(({ row }) => isFileReceiptRow(row)).map(({ row }) => row);
  const failedRows = rows.filter(({ row }) => row.state === 'failed');
  const groupFailedRows = variant !== 'receipt' && failedRows.length > 1;
  const ungroupedVisibleRows = groupFailedRows
    ? rows.filter(({ row }) => row.state !== 'failed')
    : rows;
  const visibleRows = compactRepeatedToolRows(ungroupedVisibleRows, t);
  const nonFileRows = visibleRows.filter(({ row }) => !isFileReceiptRow(row));
  const visibleFileRows = visibleRows.filter(({ row }) => isFileReceiptRow(row)).map(({ row }) => row);
  const currentActivityKey = rows.findLast(({ row }) => row.state === 'running')?.row.key;

  if (variant !== 'receipt' && shouldShowFileListDetail(visibleFileRows)) {
    return (
      <div className='turn-process-trace'>
        <ToolFileGroupTraceRow
          rows={visibleFileRows}
          workspaceRoots={workspaceRoots}
          currentActivity={visibleFileRows.some((row) => row.key === currentActivityKey)}
        />
        {nonFileRows.map(({ row, label, presentationState }) => (
          <ToolTraceRow key={row.key} row={row} label={label} workspaceRoots={workspaceRoots} currentActivity={row.key === currentActivityKey} presentationState={presentationState} />
        ))}
        {groupFailedRows && <FailedToolTraceGroup rows={failedRows} workspaceRoots={workspaceRoots} recovered={recoverFailures} />}
      </div>
    );
  }

  return (
    <div className='turn-process-trace'>
      {visibleRows.map(({ row, label, presentationState }) => (
        <ToolTraceRow
          key={row.key}
          row={row}
          label={label}
          workspaceRoots={workspaceRoots}
          fileRowCount={fileRows.length}
          currentActivity={row.key === currentActivityKey}
          presentationState={presentationState}
        />
      ))}
      {groupFailedRows && <FailedToolTraceGroup rows={failedRows} workspaceRoots={workspaceRoots} recovered={recoverFailures} />}
    </div>
  );
};

const FileProcessTraceRows: React.FC<{ diffs: FileChangeInfo[]; workspaceRoots: string[] }> = ({
  diffs,
  workspaceRoots,
}) => {
  const { t } = useTranslation();
  const { launchPreview } = usePreviewLauncher();
  const [expanded, setExpanded] = useState(false);
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

  if (!files.length) return null;
  const targets = files.map((file) => file.fullPath);
  const targetPreview = formatTargetPreview(targets, workspaceRoots);
  const label = files.length === 1
    ? t('messages.processReceipt.fileChanged', {
        target: formatWorkspaceFileTarget(files[0].fullPath, { workspaceRoots }).label,
        stats: '',
        defaultValue: 'Edited {{target}}',
      })
    : t('messages.processReceipt.fileEditTargets', {
        count: files.length,
        target: targetPreview,
        defaultValue: 'Edited {{count}} files: {{target}}',
      });

  return (
    <div className='turn-process-trace-tool'>
      <button
        type='button'
        className='turn-process-trace__row turn-process-trace-tool__toggle turn-process-trace__row--completed'
        onClick={() => setExpanded((value) => !value)}
        aria-expanded={expanded}
      >
        <TraceRowIcon kind='edit' />
        <span className='turn-process-trace__text' title={targets.join('\n')}>
          {compactReceiptText(label, targetPreview)}
        </span>
        <Right
          theme='outline'
          size='12'
          className={classNames('turn-process-trace-tool__arrow', expanded && 'turn-process-trace-tool__arrow--open')}
        />
      </button>
      {expanded && (
        <div className='turn-process-trace-detail'>
          <ul className='turn-process-trace-file-list'>
            {files.map((file) => {
              const target = formatWorkspaceFileTarget(file.fullPath, { workspaceRoots });
              const stats = formatFileChangeStats(file);
              return (
                <li key={file.fullPath} className='turn-process-trace-file-list__item'>
                  <button
                    type='button'
                    className='turn-process-trace-file-list__button'
                    title={file.fullPath}
                    onClick={() => openFile(file)}
                  >
                    <span>{target.label}</span>
                    {stats && <span className='turn-process-trace-file-list__stats'>{stats}</span>}
                  </button>
                </li>
              );
            })}
          </ul>
        </div>
      )}
    </div>
  );
};

const getUnhandledMessageType = (_message: never): string => 'unknown';

const ProcessTraceItem: React.FC<{
  item: ProcessTraceRenderableItem;
  variant?: ProcessTraceVariant;
  workspaceRoots?: string[];
  stateOverride?: TurnDisclosureProcessState;
  recoverFailures?: boolean;
}> = ({
  item,
  variant = 'list',
  workspaceRoots,
  stateOverride,
  recoverFailures = false,
}) => {
  const { t } = useTranslation();
  const conversationContext = useConversationContextSafe();
  const state = stateOverride ?? getProcessItemState(item);
  const resolvedWorkspaceRoots = useMemo(
    () =>
      workspaceRoots && workspaceRoots.length
        ? workspaceRoots
        : conversationContext?.workspace
          ? [conversationContext.workspace]
          : [],
    [conversationContext?.workspace, workspaceRoots]
  );

  if ('type' in item && item.type === 'file_summary') {
    return <FileProcessTraceRows diffs={item.diffs} workspaceRoots={resolvedWorkspaceRoots} />;
  }

  if ('type' in item && item.type === 'tool_summary') {
    return (
      <ToolProcessTraceRows
        messages={item.messages}
        variant={variant}
        workspaceRoots={resolvedWorkspaceRoots}
        stateOverride={stateOverride}
        recoverFailures={recoverFailures}
      />
    );
  }

  switch (item.type) {
    case 'text':
      {
        const content = getPublicProcessNarration(item.content.content);
        const hasToolPayload = projectAssistantText(toDisplayText(item.content.content)).hasToolPayload;
        if (!content && !hasToolPayload) return null;
        return (
          <div className='turn-process-trace__narration'>
            {content && <div className='turn-process-trace__paragraph-row'>
              <div className='turn-process-trace__paragraph' data-testid='process-narration'>
                <MarkdownView
                  fontSize={MESSAGE_BODY_FONT_SIZE}
                  lineHeight={MESSAGE_BODY_LINE_HEIGHT}
                >
                  {content}
                </MarkdownView>
              </div>
            </div>}
            {hasToolPayload && <AssistantProtocolNotice raw={toDisplayText(item.content.content)} />}
          </div>
        );
      }
    case 'thinking':
      {
        const content = toDisplayText(item.content.content).trim();
        if (!content || /^\[Private reasoning omitted(?: from replay)?\]$/i.test(content)) return null;
        return (
          <ProcessTraceRows
            rows={[{
              key: item.id,
              state,
              label: state === 'running'
                ? t('messages.processReceipt.analyzingRequest', { defaultValue: 'Analyzing the request' })
                : state === 'completed'
                  ? t('messages.processReceipt.analyzedRequest', { defaultValue: 'Analyzed the request' })
                  : t('messages.processReceipt.analyzeRequest', { defaultValue: 'Analyze the request' }),
            }]}
          />
        );
      }
    case 'tips':
      if (isContextCompressionTip(item)) {
        return (
          <ProcessTraceRows
            rows={[
              {
                key: item.id,
                state,
                label: t('messages.processReceipt.contextCompressed', { defaultValue: 'Context compressed' }),
              },
            ]}
          />
        );
      }
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
      return (
        <ToolProcessTraceRows
          messages={[item]}
          variant={variant}
          workspaceRoots={resolvedWorkspaceRoots}
          stateOverride={stateOverride}
          recoverFailures={recoverFailures}
        />
      );
    case 'agent_status':
      if (item.content.turn_summary) return null;
      return (
        <ProcessTraceRows
          rows={[
            {
              key: item.id,
              state,
              label:
                item.content.status === 'preparing'
                    ? t('messages.processReceipt.preparingAction', {
                        defaultValue: 'Preparing next action',
                      })
                    : item.content.status === 'prepared'
                      ? t('messages.processReceipt.preparedAction', {
                          defaultValue: 'Prepared next action',
                        })
                      : state === 'failed'
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
    case 'plan':
    case 'available_commands':
      return null;
    default:
      return <div>{t('messages.unknownMessageType', { type: getUnhandledMessageType(item) })}</div>;
  }
};

export default ProcessTraceItem;
