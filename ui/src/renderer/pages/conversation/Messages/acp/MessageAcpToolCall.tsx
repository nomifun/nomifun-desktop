/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IMessageAcpToolCall } from '@/common/chat/chatLib';
import { toDisplayText } from '@/common/chat/displayText';
import FileChangesPanel from '@/renderer/components/base/FileChangesPanel';
import { useDiffPreviewHandlers } from '@/renderer/hooks/file/useDiffPreviewHandlers';
import { parseDiff } from '@/renderer/utils/file/diffUtils';
import { Card, Tag } from '@arco-design/web-react';
import { createTwoFilesPatch } from 'diff';
import React, { useMemo } from 'react';
import MarkdownView from '@renderer/components/Markdown';
import LocalImageView from '@/renderer/components/media/LocalImageView';
import { MESSAGE_BODY_FONT_SIZE, MESSAGE_BODY_LINE_HEIGHT } from '../typography';

const StatusTag: React.FC<{ status: string }> = ({ status }) => {
  const statusText = toDisplayText(status);
  const getTagProps = () => {
    switch (statusText) {
      case 'pending':
        return { color: 'blue', text: 'Pending' };
      case 'in_progress':
        return { color: 'orange', text: 'In Progress' };
      case 'completed':
        return { color: 'green', text: 'Completed' };
      case 'failed':
        return { color: 'red', text: 'Failed' };
      default:
        return { color: 'gray', text: statusText };
    }
  };

  const { color, text } = getTagProps();
  return <Tag color={color}>{text}</Tag>;
};

// Diff content display as a separate component to ensure hooks are called unconditionally
const DiffContentView: React.FC<{ old_text: string; new_text: string; path: string }> = ({
  old_text,
  new_text,
  path,
}) => {
  const display_name = path.split(/[/\\]/).pop() || path || 'Unknown file';
  const formattedDiff = useMemo(
    () => createTwoFilesPatch(display_name, display_name, old_text, new_text, '', '', { context: 3 }),
    [display_name, old_text, new_text]
  );
  const fileInfo = useMemo(() => parseDiff(formattedDiff, display_name), [formattedDiff, display_name]);
  const { handleFileClick, handleDiffClick } = useDiffPreviewHandlers({
    diffText: formattedDiff,
    display_name,
    file_path: path || display_name,
  });

  return (
    <FileChangesPanel
      title={display_name}
      files={[fileInfo]}
      onFileClick={handleFileClick}
      onDiffClick={handleDiffClick}
      defaultExpanded={true}
    />
  );
};

const ContentView: React.FC<{
  content: NonNullable<IMessageAcpToolCall['content']['update']['content']>[number];
  terminalSuccess: boolean;
}> = ({ content, terminalSuccess }) => {
  if (content.type === 'diff') {
    if (!terminalSuccess) return null;
    return (
      <DiffContentView
        old_text={toDisplayText(content.old_text)}
        new_text={toDisplayText(content.new_text)}
        path={toDisplayText(content.path)}
      />
    );
  }

  if (content.type === 'artifact') {
    if (!terminalSuccess) return null;
    const { artifact } = content;
    return (
      <div className='mt-3 rounded border overflow-hidden'>
        {artifact.kind === 'image' && (
          <LocalImageView
            src={artifact.path}
            alt={artifact.relative_path || 'Generated image'}
            className='block max-w-full max-h-420px object-contain'
          />
        )}
        <code className='block bg-1 p-2 text-xs break-all' title={artifact.path}>
          {artifact.path}
        </code>
      </div>
    );
  }

  if (content.type === 'resource_link') {
    if (!terminalSuccess) return null;
    return (
      <div className='mt-3 bg-1 p-3 rounded border break-all'>
        <a href={content.uri} title={content.description || content.uri}>
          {content.title || content.name || content.uri}
        </a>
      </div>
    );
  }

  if (content.type === 'artifact_error') {
    return <div className='mt-3 bg-red-1 color-red-6 p-3 rounded border'>{content.message}</div>;
  }

  if (content.type === 'terminal') {
    return <code className='block mt-3 bg-1 p-2 rounded'>Terminal: {content.terminal_id}</code>;
  }

  // 处理 content 类型，包含 text 内容
  if (content.type === 'content' && content.content && content.content.type === 'text' && content.content.text) {
    return (
      <div className='mt-3'>
        <div className='bg-1 p-3 rounded border overflow-hidden'>
          <div className='overflow-x-auto break-words'>
            <MarkdownView
              fontSize={MESSAGE_BODY_FONT_SIZE}
              lineHeight={MESSAGE_BODY_LINE_HEIGHT}
              allowUnverifiedImages={false}
            >
              {toDisplayText(content.content.text)}
            </MarkdownView>
          </div>
        </div>
      </div>
    );
  }

  return null;
};

const MessageAcpToolCall: React.FC<{ message: IMessageAcpToolCall }> = ({ message }) => {
  const { content } = message;
  if (!content?.update) {
    return null;
  }
  const { update } = content;
  const { kind, title, status, rawInput, content: diffContent } = update;

  const getKindDisplayName = (kind: string) => {
    switch (kind) {
      case 'edit':
        return 'File Edit';
      case 'read':
        return 'File Read';
      case 'execute':
        return 'Shell Command';
      default:
        return kind;
    }
  };

  return (
    <Card className='w-full mb-2' size='small' bordered>
      <div className='flex items-start gap-3'>
        <div className='flex-1 min-w-0'>
          <div className='flex items-center gap-2 mb-2'>
            <span className='font-medium text-t-primary'>{toDisplayText(title) || getKindDisplayName(toDisplayText(kind))}</span>
            <StatusTag status={toDisplayText(status)} />
          </div>
          {rawInput && (
            <div className='text-sm'>
              {typeof rawInput === 'string' ? (
                <MarkdownView fontSize={MESSAGE_BODY_FONT_SIZE} lineHeight={MESSAGE_BODY_LINE_HEIGHT}>
                  {`\`\`\`\n${rawInput}\n\`\`\``}
                </MarkdownView>
              ) : (
                <pre className='bg-1 p-2 rounded text-xs overflow-x-auto'>{toDisplayText(rawInput)}</pre>
              )}
            </div>
          )}
          {diffContent && diffContent.length > 0 && (
            <div>
              {diffContent.map((content, index) => (
                <ContentView key={index} content={content} terminalSuccess={status === 'completed'} />
              ))}
            </div>
          )}
        </div>
      </div>
    </Card>
  );
};

export default MessageAcpToolCall;
