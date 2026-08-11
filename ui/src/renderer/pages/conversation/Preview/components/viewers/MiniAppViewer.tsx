/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { IApiMiniApp } from '@/common/adapter/ipcBridge';
import type { ConversationId, MiniAppId } from '@/common/types/ids';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import { getConversationOrNull } from '@/renderer/pages/conversation/utils/conversationCache';
import { MINI_APP_IFRAME_SANDBOX } from '@renderer/pages/miniApps/contract';
import { Refresh, SaveOne } from '@icon-park/react';
import React, { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import useSWR from 'swr';
import {
  PREVIEW_TOOLBAR_BTN_ACTIVE_CLASS,
  PREVIEW_TOOLBAR_BTN_CLASS,
} from '../PreviewPanel/PreviewToolbar';
import { usePreviewToolbarExtras } from '../../context/PreviewToolbarExtrasContext';
import MiniAppSolidifyModal, { type MiniAppPublishResult } from './MiniAppSolidifyModal';

/**
 * How long `content` must hold still before the running document is swapped.
 *
 * The preview context streams the file while the agent rewrites it, and every
 * reload of the iframe destroys the app's in-memory state (a half-filled form, a
 * running timer). Waiting for quiet means the user sees the finished revision
 * once, not each intermediate write.
 */
const MINI_APP_CONTENT_SETTLE_MS = 1200;

interface MiniAppViewerProps {
  /** Full `miniapp.html` body kept fresh by the preview context. */
  content: string;
  file_path?: string;
  workspace?: string;
  /** Owning conversation, stamped into the tab metadata by the opener. */
  conversation_id?: ConversationId;
}

/**
 * Pending publish request: the HTML snapshot plus the library it may replace into.
 *
 * The whole library rides along, not just this conversation's own row: 「替换已有
 * 小程序」 lets the user pick the target (spec D20), and `defaultTargetId` is only
 * which one is selected first.
 */
interface PublishRequest {
  html: string;
  apps: IApiMiniApp[];
  defaultTargetId: MiniAppId | null;
}

/**
 * 小程序预览器 / Mini-app viewer.
 *
 * Renders the conversation's single self-contained `miniapp.html` in a sandboxed
 * iframe, and owns the one way that document reaches the library: 「发布为小程序」,
 * which offers either a new app or an explicit replacement of one the user picks
 * (spec D20). Unlike the generic {@link HTMLRenderer} it deliberately drops both the
 * typing animation (a live app must not type itself in) and the relative-resource
 * inlining pass (spec D1: a mini-app inlines its own CSS/JS and may only reach
 * out to CDNs). The rendered document lags `content` by a quiet period so an
 * iterating agent does not keep resetting the running app; the refresh button
 * jumps to the latest body and remounts the iframe, which is also how a user
 * resets an app's in-memory state on demand.
 */
const MiniAppViewer: React.FC<MiniAppViewerProps> = ({ content, file_path, workspace, conversation_id }) => {
  const { t } = useTranslation();
  const [messageApi, messageContextHolder] = useArcoMessage();
  const [refreshKey, setRefreshKey] = useState(0);
  const [publishRequest, setPublishRequest] = useState<PublishRequest | null>(null);
  /** The body currently mounted in the iframe — advanced only when `content` settles. */
  const [doc, setDoc] = useState(content);
  // Latest body, readable without making the toolbar callbacks change identity
  // (a new `handleSolidify` on every content tick re-publishes the toolbar and
  // re-renders the whole preview panel).
  const contentRef = useRef(content);
  contentRef.current = content;
  // Ref rather than state: an in-flight guard must not re-render (and re-publish)
  // the toolbar it is reachable from.
  const preparingPublishRef = useRef(false);
  const toolbarExtrasContext = usePreviewToolbarExtras();
  const usePortalToolbar = Boolean(toolbarExtrasContext);

  // Same SWR key the conversation route already populates, so this resolves from
  // cache instead of adding a round-trip. Only used to prefill the save form.
  const { data: conversation } = useSWR(
    conversation_id ? `conversation/${conversation_id}` : null,
    () => getConversationOrNull(conversation_id!)
  );

  useEffect(() => {
    if (content === doc) return;
    const timer = setTimeout(() => setDoc(content), MINI_APP_CONTENT_SETTLE_MS);
    return () => clearTimeout(timer);
  }, [content, doc]);

  const handleRefresh = useCallback(() => {
    // Manual refresh means "show me the newest version, now" — skip the wait.
    setDoc(contentRef.current);
    setRefreshKey((prev) => prev + 1);
  }, []);

  const handlePublish = useCallback(async () => {
    if (preparingPublishRef.current) return;
    preparingPublishRef.current = true;
    try {
      // Read the file rather than trusting the rendered tab: the agent may have
      // rewritten it between the last poll and this click.
      const html = file_path
        ? await ipcBridge.fs.readFile.invoke({ path: file_path, workspace })
        : contentRef.current;
      if (!html) {
        messageApi.error(t('miniApps.preview.readError'));
        return;
      }
      // The library is fetched HERE rather than inside the dialog so a failure to
      // read it costs a toast instead of an open modal with an empty picker.
      const apps = await ipcBridge.miniapps.list.invoke();
      const defaultTargetId = conversation_id
        ? (apps.find((item) => item.source_conversation_id === conversation_id)?.miniapp_id ?? null)
        : null;
      setPublishRequest({ html, apps, defaultTargetId });
    } catch (error) {
      console.error('[MiniAppViewer] Failed to prepare the publish dialog:', error);
      messageApi.error(t('miniApps.preview.readError'));
    } finally {
      preparingPublishRef.current = false;
    }
  }, [file_path, workspace, conversation_id, messageApi, t]);

  const handleSaved = useCallback(
    (result: MiniAppPublishResult) => {
      setPublishRequest(null);
      messageApi.success(
        result.mode === 'replace'
          ? t('miniApps.save.replaceSuccess', { name: result.name })
          : t('miniApps.save.success')
      );
    },
    [messageApi, t]
  );

  const handleSaveError = useCallback(() => {
    messageApi.error(t('miniApps.save.error'));
  }, [messageApi, t]);

  // Toolbar slot (mirrors PDFViewer): label on the left, refresh + publish on the
  // right, wearing the shared PreviewToolbar button tokens.
  useEffect(() => {
    if (!usePortalToolbar || !toolbarExtrasContext) return;
    toolbarExtrasContext.setExtras({
      left: (
        <div className='flex items-center gap-8px'>
          <span className='text-13px text-t-secondary'>🧩 {t('miniApps.preview.title')}</span>
        </div>
      ),
      right: (
        <div className='flex items-center gap-4px'>
          <button
            type='button'
            className={PREVIEW_TOOLBAR_BTN_CLASS}
            onClick={handleRefresh}
            title={t('miniApps.preview.refresh')}
            data-testid='miniapp-preview-refresh'
          >
            <Refresh theme='outline' size={12} fill='currentColor' />
            <span>{t('miniApps.preview.refresh')}</span>
          </button>
          <button
            type='button'
            className={`${PREVIEW_TOOLBAR_BTN_CLASS} ${PREVIEW_TOOLBAR_BTN_ACTIVE_CLASS}`}
            onClick={() => void handlePublish()}
            title={t('miniApps.preview.publish')}
            data-testid='miniapp-preview-publish'
          >
            <SaveOne theme='outline' size={12} fill='currentColor' />
            <span>{t('miniApps.preview.publish')}</span>
          </button>
        </div>
      ),
    });
    return () => toolbarExtrasContext.setExtras(null);
  }, [usePortalToolbar, toolbarExtrasContext, t, handleRefresh, handlePublish]);

  return (
    <div className='h-full w-full overflow-hidden bg-white relative'>
      {messageContextHolder}
      <iframe
        key={`miniapp-${refreshKey}`}
        srcDoc={doc}
        title={t('miniApps.preview.title')}
        className='w-full h-full border-0'
        style={{ display: 'block', width: '100%', height: '100%' }}
        sandbox={MINI_APP_IFRAME_SANDBOX}
      />
      <MiniAppSolidifyModal
        visible={publishRequest !== null}
        html={publishRequest?.html ?? ''}
        apps={publishRequest?.apps ?? []}
        defaultTargetId={publishRequest?.defaultTargetId ?? null}
        conversation_id={conversation_id}
        defaultName={conversation?.name ?? ''}
        onCancel={() => setPublishRequest(null)}
        onSaved={handleSaved}
        onError={handleSaveError}
      />
    </div>
  );
};

export default MiniAppViewer;
