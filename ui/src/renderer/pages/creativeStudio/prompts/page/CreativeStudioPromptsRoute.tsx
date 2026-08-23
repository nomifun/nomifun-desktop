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
  const { t, i18n } = useTranslation();
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
      Message.success(
        t('creativeStudio.prompts.copySuccess', {
          defaultValue: 'Prompt copied',
        })
      );
      onPromptCopied?.(selection);
    } catch (reason) {
      const message =
        reason instanceof Error
          ? reason.message
          : t('creativeStudio.prompts.copyFailedFallback', {
              defaultValue: 'Copy failed. Check clipboard permissions.',
            });
      setCopyError(message);
      setCopyState('failed');
      Message.error(
        t('creativeStudio.prompts.copyFailed', {
          defaultValue: 'Could not copy prompt',
        })
      );
    }
  }, [copyState, onPromptCopied, selected, t, writeClipboardText]);

  const saveSelectedPrompt = useCallback(async () => {
    if (!selected || saveState === 'saving' || saveState === 'saved') return;
    setSaveState('saving');
    setSaveError(null);
    try {
      await assetPort.createText({
        title: selected.title,
        textContent: selected.prompt,
        collection: t('creativeStudio.prompts.collectionLabel', {
          defaultValue: 'Prompts',
        }),
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
      Message.success(
        t('creativeStudio.prompts.saveSuccess', {
          defaultValue: 'Added to my assets',
        })
      );
    } catch (reason) {
      const message =
        reason instanceof Error
          ? reason.message
          : t('creativeStudio.prompts.saveFailedFallback', {
              defaultValue: 'Save failed. Try again later.',
            });
      setSaveError(message);
      setSaveState('failed');
      Message.error(
        t('creativeStudio.prompts.saveFailed', {
          defaultValue: 'Could not save prompt',
        })
      );
    }
  }, [assetPort, saveState, selected, t]);

  return (
    <>
      <StandalonePromptLibraryPage
        port={promptPort}
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
