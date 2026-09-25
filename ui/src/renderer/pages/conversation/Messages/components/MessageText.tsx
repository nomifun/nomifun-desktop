/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IMessageText } from '@/common/chat/chatLib';
import { toDisplayText } from '@/common/chat/displayText';
import { useConversationContextSafe } from '@/renderer/hooks/context/ConversationContext';
import { iconColors } from '@/renderer/styles/colors';
import { Alert, Message, Tooltip } from '@arco-design/web-react';
import { Copy, Edit } from '@icon-park/react';
import classNames from 'classnames';
import React, { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { copyText } from '@/renderer/utils/ui/clipboard';
import { emitter } from '@/renderer/utils/emitter';
import { useMessageList } from '../hooks';
import CollapsibleContent from '@renderer/components/chat/CollapsibleContent';
import FilePreview from '@renderer/components/media/FilePreview';
import HorizontalFileList from '@renderer/components/media/HorizontalFileList';
import MarkdownView from '@renderer/components/Markdown';
import { stripThinkTags, hasThinkTags } from '@renderer/utils/chat/thinkTagFilter';
import { stripSkillSuggest, hasSkillSuggest } from '@renderer/utils/chat/skillSuggestParser';
import { MESSAGE_BODY_CLASS_NAME, MESSAGE_BODY_FONT_SIZE, MESSAGE_BODY_LINE_HEIGHT } from '../typography';
import { parseMessageFileMarker } from './messageFileMarker';
import { projectAssistantText } from '../processTraceDisplayModel';
import AssistantProtocolNotice from './AssistantProtocolNotice';

/**
 * Format a timestamp for message display.
 * Today: "HH:mm", older: "MM-DD HH:mm".
 */
export const formatMessageTime = (timestamp: number): string => {
  const date = new Date(timestamp);
  const now = new Date();
  const hours = date.getHours().toString().padStart(2, '0');
  const minutes = date.getMinutes().toString().padStart(2, '0');
  const time = `${hours}:${minutes}`;

  if (
    date.getFullYear() !== now.getFullYear() ||
    date.getMonth() !== now.getMonth() ||
    date.getDate() !== now.getDate()
  ) {
    const month = (date.getMonth() + 1).toString().padStart(2, '0');
    const day = date.getDate().toString().padStart(2, '0');
    return `${month}-${day} ${time}`;
  }
  return time;
};
import MessageCronBadge from './MessageCronBadge';
import { getAgentLogo } from '@/renderer/utils/model/agentLogo';
import AgentMessageAvatar from './AgentMessageAvatar';

const CODE_STYLE = { marginTop: 4, marginBlock: 4 };

const isAbsoluteMessageFilePath = (file_path: string): boolean =>
  file_path.startsWith('/') || /^[A-Za-z]:/.test(file_path);

export const resolveMessageFilePath = (file_path: string, workspace?: string): string => {
  if (!file_path || isAbsoluteMessageFilePath(file_path) || !workspace) {
    return file_path;
  }

  const normalizedWorkspace = workspace.replace(/[\\/]+$/, '').replace(/\\/g, '/');
  const normalizedFilePath = file_path.replace(/^\.?[\\/]+/, '').replace(/\\/g, '/');
  return `${normalizedWorkspace}/${normalizedFilePath}`.replace(/\/+/g, '/');
};

const useFormatContent = (content: string) => {
  return useMemo(() => {
    try {
      const json = JSON.parse(content);
      const isJson = typeof json === 'object';
      return {
        json: isJson,
        data: isJson ? json : content,
      };
    } catch {
      return { data: content };
    }
  }, [content]);
};

const MessageText: React.FC<{
  message: IMessageText;
  hideActions?: boolean;
  actionsOnly?: boolean;
}> = ({ message, hideActions = false, actionsOnly = false }) => {
  const { t } = useTranslation();
  // Filter think tags from content before rendering
  // 在渲染前过滤 think 标签
  const presentation = useMemo(() => {
    let content = toDisplayText(message.content.content);
    if (hasThinkTags(content)) {
      content = stripThinkTags(content);
    }
    // Strip any inline [SKILL_SUGGEST] blocks (now handled via separate skill_suggest message type)
    if (hasSkillSuggest(content)) {
      content = stripSkillSuggest(content);
    }
    return message.position === 'left'
      ? projectAssistantText(content)
      : { text: content, hasToolPayload: false };
  }, [message.content.content, message.position]);
  const contentToRender = presentation.text;

  const { text, files } = parseMessageFileMarker(contentToRender, message.position);
  const { data, json } = useFormatContent(text);
  const [showCopyAlert, setShowCopyAlert] = useState(false);
  const isUserMessage = message.position === 'right';
  const isAgentMessage = message.position === 'left' && message.content.agentMessage === true;
  const shouldRenderPlainText = isUserMessage;
  const conversationContext = useConversationContextSafe();
  const shouldShowActions = !hideActions && Boolean(contentToRender.trim());
  const resolvedFiles = useMemo(
    () => files.map((file_path) => resolveMessageFilePath(file_path, conversationContext?.workspace)),
    [conversationContext?.workspace, files]
  );

  // 仅 Nomi、且为最近一条用户文本消息时可编辑（与后端"仅最近一条"对齐）。
  const messageList = useMessageList();
  const editableMessageId = message.message_id ?? message.msg_id;
  const isLatestUserMessage = useMemo(() => {
    if (!isUserMessage || !editableMessageId) return false;
    const lastRight = [...messageList].reverse().find((m) => m.position === 'right' && m.type === 'text');
    return (lastRight?.message_id ?? lastRight?.msg_id) === editableMessageId;
  }, [editableMessageId, isUserMessage, messageList]);

  // 过滤空内容，避免渲染空DOM
  const hasRenderableContent = contentToRender.trim().length > 0;

  if (!hasRenderableContent && !presentation.hasToolPayload) {
    return null;
  }

  const handleCopy = () => {
    const baseText = shouldRenderPlainText ? text : json ? JSON.stringify(data, null, 2) : text;
    const fileList = files.length ? `Files:\n${files.map((path) => `- ${path}`).join('\n')}\n\n` : '';
    const textToCopy = fileList + baseText;
    copyText(textToCopy)
      .then(() => {
        setShowCopyAlert(true);
        setTimeout(() => setShowCopyAlert(false), 2000);
      })
      .catch(() => {
        Message.error(t('common.copyFailed'));
      });
  };

  const copyButton = (
    <Tooltip content={t('common.copy', { defaultValue: 'Copy' })}>
      <div
        data-testid='message-copy-action'
        className='flex h-24px w-24px shrink-0 items-center justify-center rd-4px cursor-pointer text-t-secondary hover:bg-3 transition-colors'
        onClick={handleCopy}
        style={{ lineHeight: 0 }}
      >
        <Copy theme='outline' size='16' fill='currentColor' />
      </div>
    </Tooltip>
  );

  // 编辑（仅 Nomi 原生、且为最近一条用户文本消息）：把原文回填输入框并截断本地后续消息。
  const canEdit =
    conversationContext?.type === 'nomi' &&
    conversationContext.isProcessing !== true &&
    isUserMessage &&
    message.type === 'text' &&
    editableMessageId != null &&
    message.created_at != null &&
    isLatestUserMessage;

  const handleEdit = () => {
    if (!editableMessageId || message.created_at == null) return;
    const rawContent = typeof message.content?.content === 'string' ? message.content.content : '';
    const { text: editText } = parseMessageFileMarker(rawContent, 'right');
    emitter.emit('sendbox.edit', { msgId: editableMessageId, createdAt: message.created_at, content: editText });
  };

  const editButton = canEdit ? (
    <Tooltip content={t('conversation.editMessage.action', { defaultValue: 'Edit' })}>
      <div
        className='p-4px rd-4px cursor-pointer hover:bg-3 transition-colors opacity-0 pointer-events-none group-hover:opacity-100 group-hover:pointer-events-auto focus-within:opacity-100 focus-within:pointer-events-auto'
        onClick={handleEdit}
        style={{ lineHeight: 0 }}
      >
        <Edit theme='outline' size='16' fill={iconColors.secondary} />
      </div>
    </Tooltip>
  ) : null;

  const cronMeta = message.content.cronMeta;
  const senderName = message.content.senderName;
  const senderAgentType = message.content.senderAgentType;
  const senderConversationId = message.content.senderConversationId;
  const fallbackBackendLogo = senderAgentType ? getAgentLogo(senderAgentType) : null;
  const actionsRow = shouldShowActions ? (
    <div
      data-testid='message-actions'
      className={classNames('message-text-actions h-28px flex items-center mt-4px gap-6px text-t-secondary', {
        'flex-row-reverse': isUserMessage,
      })}
    >
      {copyButton}
      {editButton}
      {message.created_at && (
        <span className='text-12px leading-20px text-inherit select-none'>
          {formatMessageTime(message.content.display_at_ms ?? message.created_at)}
        </span>
      )}
    </div>
  ) : null;
  const copyAlert = showCopyAlert ? (
    <Alert
      type='success'
      content={t('messages.copySuccess')}
      showIcon
      className='fixed top-20px left-50% transform -translate-x-50% z-9999 w-max max-w-[80%]'
      style={{ boxShadow: '0px 2px 12px rgba(0,0,0,0.12)' }}
      closable={false}
    />
  ) : null;

  if (actionsOnly) {
    return (
      <>
        {actionsRow}
        {copyAlert}
      </>
    );
  }

  return (
    <>
      <div className={classNames('min-w-0 flex flex-col group', isUserMessage ? 'items-end' : 'items-start')}>
        {cronMeta && <MessageCronBadge meta={cronMeta} />}
        {message.content.interaction?.kind === 'robot' && (
          <span className='mb-4px text-11px text-t-tertiary' title={message.content.interaction.robot_id}>
            {t('nomi.robot.speechSource')}
          </span>
        )}
        {isAgentMessage && senderName && (
          <div className='flex items-center gap-6px mb-4px'>
            <AgentMessageAvatar
              senderName={senderName}
              senderConversationId={senderConversationId}
              backendLogo={fallbackBackendLogo}
            />
            <span className='text-12px text-t-secondary'>{senderName}</span>
          </div>
        )}
        {files.length > 0 && (
          <div className={classNames('mt-6px', { 'self-end': isUserMessage })}>
            {resolvedFiles.length === 1 ? (
              <div className='flex items-center'>
                <FilePreview path={resolvedFiles[0]} onRemove={() => undefined} readonly />
              </div>
            ) : (
              <HorizontalFileList>
                {resolvedFiles.map((path) => (
                  <FilePreview key={path} path={path} onRemove={() => undefined} readonly />
                ))}
              </HorizontalFileList>
            )}
          </div>
        )}
        {hasRenderableContent && (
          <div
            className={classNames('min-w-0 max-w-full [&>p:first-child]:mt-0px [&>p:last-child]:mb-0px', {
              'bg-aou-2 p-6px md:p-8px': isUserMessage || cronMeta,
              'bg-3 p-6px md:p-8px': isAgentMessage,
              'w-full': !(isUserMessage || cronMeta || isAgentMessage),
            })}
            style={{
              ...(isUserMessage || cronMeta
                ? { borderRadius: '8px 0 8px 8px', color: 'var(--text-primary)' }
                : isAgentMessage
                  ? { borderRadius: '0 8px 8px 8px' }
                  : undefined),
            }}
          >
            {/* JSON 内容使用折叠组件 Use CollapsibleContent for JSON content */}
            {shouldRenderPlainText ? (
              <div className={MESSAGE_BODY_CLASS_NAME} data-testid='message-text-content'>
                {text}
              </div>
            ) : json ? (
              <CollapsibleContent maxHeight={200} defaultCollapsed={true}>
                <div data-testid='message-text-content'>
                  <MarkdownView
                    codeStyle={CODE_STYLE}
                    fontSize={MESSAGE_BODY_FONT_SIZE}
                    lineHeight={MESSAGE_BODY_LINE_HEIGHT}
                    allowUnverifiedImages={isUserMessage}
                  >{`\`\`\`json\n${JSON.stringify(data, null, 2)}\n\`\`\``}</MarkdownView>
                </div>
              </CollapsibleContent>
            ) : (
              <div data-testid='message-text-content'>
                <MarkdownView
                  codeStyle={CODE_STYLE}
                  fontSize={MESSAGE_BODY_FONT_SIZE}
                  lineHeight={MESSAGE_BODY_LINE_HEIGHT}
                  allowUnverifiedImages={isUserMessage}
                >
                  {data}
                </MarkdownView>
              </div>
            )}
          </div>
        )}
        {presentation.hasToolPayload && <AssistantProtocolNotice raw={toDisplayText(message.content.content)} />}
        {message.content.observations?.map((observation) => (
          <div key={observation.image.id} className='mt-8px flex max-w-full flex-col gap-6px rd-8px border border-solid border-arco-2 p-10px'>
            <span className='text-12px text-t-secondary'>{t('nomi.robot.visualObservation')}</span>
            <FilePreview path={observation.image.path} readonly />
            <span className='text-13px whitespace-pre-wrap break-words'>{observation.answer}</span>
          </div>
        ))}
        {actionsRow}
      </div>
      {copyAlert}
    </>
  );
};

export default MessageText;
