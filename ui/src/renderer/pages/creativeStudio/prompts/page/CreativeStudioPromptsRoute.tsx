/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Message } from '@arco-design/web-react';
import React, { useCallback, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { copyText } from '@/renderer/utils/ui/clipboard';

import {
  creativeAssetClient,
  type CreativeAssetLibraryPort,
} from '../../assets';
import {
  createNomiPromptLibraryPort,
  creativePromptCatalogPort,
  type PromptLibraryItem,
  type PromptLibraryPort,
  type PromptLibrarySelection,
} from '..';
import PromptLibraryDetails, {
  type PromptCopyState,
  type PromptSaveState,
} from './PromptLibraryDetails';
import StandalonePromptLibraryPage from './StandalonePromptLibraryPage';
import {
  copyStandalonePrompt,
  type PromptClipboardWriter,
} from './standaloneSelection';

export interface CreativeStudioPromptsRouteProps {
  /** Test/host injection only; production uses the NomiFun preset + asset adapter. */
  port?: PromptLibraryPort;
  assetPort?: CreativeAssetLibraryPort;
  locale?: string;
  writeClipboardText?: PromptClipboardWriter;
  onPromptCopied?: (selection: PromptLibrarySelection) => void;
}

/** Standalone `/workshop/prompts` route. It deliberately has no canvas insertion target. */
export const CreativeStudioPromptsRoute: React.FC<CreativeStudioPromptsRouteProps> = ({
  port,
  assetPort = creativeAssetClient,
  locale: localeOverride,
  writeClipboardText = copyText,
  onPromptCopied,
}) => {
  const { i18n } = useTranslation();
  const locale = localeOverride ?? i18n.resolvedLanguage ?? i18n.language ?? 'zh-CN';
  const promptPort = useMemo(
    () =>
      port ??
      createNomiPromptLibraryPort({
        locale,
        assets: assetPort,
        catalog: creativePromptCatalogPort,
      }),
    [assetPort, locale, port]
  );
  const [selected, setSelected] = useState<PromptLibraryItem | null>(null);
  const [copyState, setCopyState] = useState<PromptCopyState>('idle');
  const [copyError, setCopyError] = useState<string | null>(null);
  const [saveState, setSaveState] = useState<PromptSaveState>('idle');
  const [saveError, setSaveError] = useState<string | null>(null);

  const selectPrompt = useCallback((item: PromptLibraryItem) => {
    setSelected(item);
    setCopyState('idle');
    setCopyError(null);
    setSaveState('idle');
    setSaveError(null);
  }, []);

  const closeDetails = useCallback(() => {
    setSelected(null);
    setCopyState('idle');
    setCopyError(null);
    setSaveState('idle');
    setSaveError(null);
  }, []);

  const copySelectedPrompt = useCallback(async () => {
    if (!selected || copyState === 'copying') return;
    setCopyState('copying');
    setCopyError(null);
    try {
      const selection = await copyStandalonePrompt(selected, writeClipboardText);
      setCopyState('copied');
      Message.success('提示词已复制');
      onPromptCopied?.(selection);
    } catch (reason) {
      const message = reason instanceof Error ? reason.message : '复制失败，请检查剪贴板权限。';
      setCopyError(message);
      setCopyState('failed');
      Message.error('提示词复制失败');
    }
  }, [copyState, onPromptCopied, selected, writeClipboardText]);

  const saveSelectedPrompt = useCallback(async () => {
    if (!selected || saveState === 'saving' || saveState === 'saved') return;
    setSaveState('saving');
    setSaveError(null);
    try {
      await assetPort.createText({
        title: selected.title,
        textContent: selected.prompt,
        collection: '提示词',
        tags: [...new Set([selected.category, ...selected.tags].filter((value): value is string => Boolean(value)))],
        inLibrary: true,
        origin:
          selected.source === 'catalog' &&
          selected.sourceUrl &&
          selected.license &&
          selected.licenseUrl
            ? {
                promptCatalogId: selected.id,
                sourceUrl: selected.sourceUrl,
                license: selected.license,
                licenseUrl: selected.licenseUrl,
              }
            : undefined,
      });
      setSaveState('saved');
      Message.success('已加入我的素材');
    } catch (reason) {
      const message = reason instanceof Error ? reason.message : '保存失败，请稍后重试。';
      setSaveError(message);
      setSaveState('failed');
      Message.error('保存提示词失败');
    }
  }, [assetPort, saveState, selected]);

  return (
    <>
      <StandalonePromptLibraryPage
        port={promptPort}
        title='提示词中心'
        selectedId={selected?.id ?? null}
        onSelect={selectPrompt}
      />
      <PromptLibraryDetails
        item={selected}
        locale={locale}
        copyState={copyState}
        copyError={copyError}
        saveState={saveState}
        saveError={saveError}
        onClose={closeDetails}
        onCopy={() => void copySelectedPrompt()}
        onSave={selected?.source === 'asset' ? undefined : () => void saveSelectedPrompt()}
      />
    </>
  );
};

export default CreativeStudioPromptsRoute;
