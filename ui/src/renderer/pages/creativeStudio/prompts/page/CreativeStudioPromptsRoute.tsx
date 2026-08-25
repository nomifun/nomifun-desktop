/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Message } from '@arco-design/web-react';
import React, { useCallback, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { copyText } from '@/renderer/utils/ui/clipboard';

import {
  creativeAssetClient,
  invalidateCreativeAssetQueryCache,
  type CreativeAssetLibraryPort,
  type CreativePromptAssetPort,
} from '../../assets';
import {
  createNomiPromptLibraryPort,
  creativePromptCatalogPort,
  promptLibraryItemKey,
  type PromptLibraryItem,
  type PromptLibraryPort,
  type PromptLibrarySelection,
} from '..';
import PromptLibraryDetails, {
  type PromptCopyState,
  type PromptRemoveState,
  type PromptSaveState,
} from './PromptLibraryDetails';
import StandalonePromptLibraryPage from './StandalonePromptLibraryPage';
import {
  copyStandalonePrompt,
  type PromptClipboardWriter,
} from './standaloneSelection';

const defaultNotifySuccess = (message: string): void => {
  Message.success(message);
};
const defaultNotifyError = (message: string): void => {
  Message.error(message);
};

function notifySafely(notify: (message: string) => void, message: string): void {
  try {
    notify(message);
  } catch {
    // A transient toast failure must not change the completed product action.
  }
}

export interface CreativeStudioPromptsRouteProps {
  /** Test/host injection only; production uses the NomiFun preset + asset adapter. */
  port?: PromptLibraryPort;
  assetPort?: CreativeAssetLibraryPort & CreativePromptAssetPort;
  locale?: string;
  writeClipboardText?: PromptClipboardWriter;
  onPromptCopied?: (selection: PromptLibrarySelection) => void;
  /** Test/host notification injection; production defaults to Arco messages. */
  notifySuccess?: (message: string) => void;
  notifyError?: (message: string) => void;
}

/** Standalone `/workshop/prompts` route. It deliberately has no canvas insertion target. */
export const CreativeStudioPromptsRoute: React.FC<CreativeStudioPromptsRouteProps> = ({
  port,
  assetPort = creativeAssetClient,
  locale: localeOverride,
  writeClipboardText = copyText,
  onPromptCopied,
  notifySuccess = defaultNotifySuccess,
  notifyError = defaultNotifyError,
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
  const [removeState, setRemoveState] = useState<PromptRemoveState>('idle');
  const [removeError, setRemoveError] = useState<string | null>(null);
  const selectedPromptKeyRef = useRef<string | null>(null);
  const promptMembershipOverridesRef = useRef(new Map<string, boolean>());
  const promptMutationByKeyRef = useRef(new Map<string, 'saving' | 'removing'>());

  const promptKey = useCallback((item: PromptLibraryItem): string | null => {
    return item.source === 'asset' ? null : promptLibraryItemKey(item);
  }, []);

  const promptIsSaved = useCallback((item: PromptLibraryItem, key: string | null): boolean => {
    if (key === null) return false;
    return promptMembershipOverridesRef.current.get(key) ?? item.savedToAssets;
  }, []);

  const selectPrompt = useCallback((item: PromptLibraryItem) => {
    const key = promptKey(item);
    const mutation = key === null ? undefined : promptMutationByKeyRef.current.get(key);
    selectedPromptKeyRef.current = key;
    setSelected(item);
    setCopyState('idle');
    setCopyError(null);
    setSaveState(
      mutation === 'saving'
        ? 'saving'
        : promptIsSaved(item, key)
          ? 'saved'
          : 'idle'
    );
    setSaveError(null);
    setRemoveState(mutation === 'removing' ? 'removing' : 'idle');
    setRemoveError(null);
  }, [promptIsSaved, promptKey]);

  const closeDetails = useCallback(() => {
    selectedPromptKeyRef.current = null;
    setSelected(null);
    setCopyState('idle');
    setCopyError(null);
    setSaveState('idle');
    setSaveError(null);
    setRemoveState('idle');
    setRemoveError(null);
  }, []);

  const copySelectedPrompt = useCallback(async () => {
    if (!selected || copyState === 'copying') return;
    setCopyState('copying');
    setCopyError(null);
    try {
      const selection = await copyStandalonePrompt(selected, writeClipboardText);
      setCopyState('copied');
      notifySafely(
        notifySuccess,
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
      notifySafely(
        notifyError,
        t('creativeStudio.prompts.copyFailed', {
          defaultValue: 'Could not copy prompt',
        })
      );
    }
  }, [copyState, notifyError, notifySuccess, onPromptCopied, selected, t, writeClipboardText]);

  const saveSelectedPrompt = useCallback(async () => {
    if (!selected || selected.source === 'asset') return;
    const item = selected;
    const key = promptKey(item);
    if (
      key === null ||
      promptIsSaved(item, key) ||
      promptMutationByKeyRef.current.has(key)
    ) {
      return;
    }
    // React state is asynchronous; the synchronous key lock also blocks a
    // same-render double click before the button can re-render as loading.
    promptMutationByKeyRef.current.set(key, 'saving');
    setSaveState('saving');
    setSaveError(null);
    setRemoveState('idle');
    setRemoveError(null);
    try {
      await assetPort.createText({
        title: item.title,
        textContent: item.prompt,
        collection: t('creativeStudio.prompts.collectionLabel', {
          defaultValue: 'Prompts',
        }),
        tags: [...new Set([item.category, ...item.tags].filter((value): value is string => Boolean(value)))],
        inLibrary: true,
        origin:
          item.source === 'catalog'
            ? {
                promptLibrarySource: 'catalog',
                promptLibraryId: item.id,
                promptCatalogId: item.id,
                sourceUrl: item.sourceUrl ?? undefined,
                license: item.license ?? undefined,
                licenseUrl: item.licenseUrl ?? undefined,
              }
            : {
                promptLibrarySource: 'preset',
                promptLibraryId: item.id,
              },
      });
      invalidateCreativeAssetQueryCache(assetPort);
      promptMembershipOverridesRef.current.set(key, true);
      if (selectedPromptKeyRef.current === key) setSaveState('saved');
      notifySafely(
        notifySuccess,
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
      if (selectedPromptKeyRef.current === key) {
        setSaveError(message);
        setSaveState('failed');
      }
      notifySafely(
        notifyError,
        t('creativeStudio.prompts.saveFailed', {
          defaultValue: 'Could not save prompt',
        })
      );
    } finally {
      if (promptMutationByKeyRef.current.get(key) === 'saving') {
        promptMutationByKeyRef.current.delete(key);
      }
    }
  }, [assetPort, notifyError, notifySuccess, promptIsSaved, promptKey, selected, t]);

  const removeSelectedPrompt = useCallback(async () => {
    if (!selected || selected.source === 'asset') return;
    const item = selected;
    const key = promptKey(item);
    if (
      key === null ||
      !promptIsSaved(item, key) ||
      promptMutationByKeyRef.current.has(key)
    ) {
      return;
    }
    promptMutationByKeyRef.current.set(key, 'removing');
    setRemoveState('removing');
    setRemoveError(null);
    try {
      await assetPort.removePromptAsset(
        item.source === 'catalog' ? 'catalog' : 'preset',
        item.id
      );
      invalidateCreativeAssetQueryCache(assetPort);
      promptMembershipOverridesRef.current.set(key, false);
      if (selectedPromptKeyRef.current === key) {
        setSaveState('idle');
        setSaveError(null);
        setRemoveState('removed');
      }
      notifySafely(
        notifySuccess,
        t('creativeStudio.prompts.removeSuccess', {
          defaultValue: 'Removed from my assets',
        })
      );
    } catch (reason) {
      const message =
        reason instanceof Error
          ? reason.message
          : t('creativeStudio.prompts.removeFailedFallback', {
              defaultValue: 'Could not remove this prompt. Try again later.',
            });
      if (selectedPromptKeyRef.current === key) {
        setRemoveError(message);
        setRemoveState('failed');
      }
      notifySafely(
        notifyError,
        t('creativeStudio.prompts.removeFailed', {
          defaultValue: 'Could not remove prompt from my assets',
        })
      );
    } finally {
      if (promptMutationByKeyRef.current.get(key) === 'removing') {
        promptMutationByKeyRef.current.delete(key);
      }
    }
  }, [assetPort, notifyError, notifySuccess, promptIsSaved, promptKey, selected, t]);

  return (
    <>
      <StandalonePromptLibraryPage
        port={promptPort}
        selectedId={selected?.id ?? null}
        selectedSource={selected?.source ?? null}
        onSelect={selectPrompt}
      />
      <PromptLibraryDetails
        item={selected}
        locale={locale}
        copyState={copyState}
        copyError={copyError}
        saveState={saveState}
        saveError={saveError}
        removeState={removeState}
        removeError={removeError}
        onClose={closeDetails}
        onCopy={() => void copySelectedPrompt()}
        onSave={selected?.source === 'asset' ? undefined : () => void saveSelectedPrompt()}
        onRemove={selected?.source === 'asset' ? undefined : () => void removeSelectedPrompt()}
      />
    </>
  );
};

export default CreativeStudioPromptsRoute;
