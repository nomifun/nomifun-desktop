/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect, useRef, useState } from 'react';

import CreativeAssetPickerModal from './components/CreativeAssetPickerModal';
import type { CreativeAsset, CreativeAssetKind, CreativeAssetLibraryPort } from './types';
import { useCreativeAssets } from './useCreativeAssets';

export interface CreativeAssetPickerRequest {
  acceptedKinds: readonly CreativeAssetKind[];
  initialSelectedIds?: readonly string[];
  selectionLimit?: number;
  title?: string;
}

export interface CreativeAssetPickerDialogController {
  pick(request: CreativeAssetPickerRequest): Promise<string[] | null>;
  dialog: React.ReactElement | null;
  open: boolean;
}

export interface UseCreativeAssetPickerDialogOptions {
  client?: CreativeAssetLibraryPort;
  pageSize?: number;
}

function normalizedSelection(
  selectedIds: readonly string[],
  selectionLimit: number | undefined
): string[] {
  const unique = [...new Set(selectedIds)];
  return selectionLimit === undefined ? unique : unique.slice(0, selectionLimit);
}

export function toggleCreativeAssetPickerSelection(
  selectedIds: readonly string[],
  assetId: string,
  selectionLimit?: number
): string[] {
  if (selectedIds.includes(assetId)) return selectedIds.filter((id) => id !== assetId);
  if (selectionLimit === 1) return [assetId];
  if (selectionLimit !== undefined && selectedIds.length >= selectionLimit) return [...selectedIds];
  return [...selectedIds, assetId];
}

export function useCreativeAssetPickerDialog(
  options: UseCreativeAssetPickerDialogOptions = {}
): CreativeAssetPickerDialogController {
  const assets = useCreativeAssets({
    client: options.client,
    pageSize: options.pageSize ?? 80,
    query: { sort: 'updated_desc' },
  });
  const [request, setRequest] = useState<CreativeAssetPickerRequest | null>(null);
  const [selectedIds, setSelectedIds] = useState<string[]>([]);
  const resolverRef = useRef<((value: string[] | null) => void) | null>(null);

  const settle = useCallback((value: string[] | null) => {
    const resolve = resolverRef.current;
    resolverRef.current = null;
    setRequest(null);
    setSelectedIds([]);
    resolve?.(value);
  }, []);

  useEffect(
    () => () => {
      resolverRef.current?.(null);
      resolverRef.current = null;
    },
    []
  );

  const pick = useCallback((nextRequest: CreativeAssetPickerRequest) => {
    if (nextRequest.acceptedKinds.length === 0) {
      return Promise.reject(new TypeError('Creative asset picker requires an accepted kind'));
    }
    if (
      nextRequest.selectionLimit !== undefined
      && (!Number.isInteger(nextRequest.selectionLimit) || nextRequest.selectionLimit < 1)
    ) {
      return Promise.reject(new TypeError('Creative asset picker selectionLimit must be positive'));
    }
    if (resolverRef.current) {
      return Promise.reject(new Error('Creative asset picker is already open'));
    }
    setRequest({ ...nextRequest, acceptedKinds: [...new Set(nextRequest.acceptedKinds)] });
    setSelectedIds(normalizedSelection(
      nextRequest.initialSelectedIds ?? [],
      nextRequest.selectionLimit
    ));
    return new Promise<string[] | null>((resolve) => {
      resolverRef.current = resolve;
    });
  }, []);

  const toggle = useCallback((asset: CreativeAsset) => {
    if (!request || !request.acceptedKinds.includes(asset.kind)) return;
    setSelectedIds((current) =>
      toggleCreativeAssetPickerSelection(current, asset.id, request.selectionLimit)
    );
  }, [request]);

  return {
    pick,
    open: request !== null,
    dialog: request ? (
      <CreativeAssetPickerModal
        open
        assets={assets.assets}
        acceptedKinds={request.acceptedKinds}
        selectedIds={selectedIds}
        selectionLimit={request.selectionLimit}
        title={request.title}
        loading={assets.loading}
        loadingMore={assets.loadingMore}
        hasMore={assets.hasMore}
        error={assets.error ?? assets.mutationError}
        uploading={assets.mutating}
        onToggle={toggle}
        onLoadMore={() => void assets.loadMore()}
        onRetry={() => void assets.reload()}
        onUploadFiles={(files) => {
          void Promise.all(
            files.map((file) => assets.upload(file, {
              title: file.name,
              tags: ['asset-picker'],
              inLibrary: true,
            }))
          )
            .then((uploaded) => {
              setSelectedIds((current) => uploaded.reduce(
                (selection, asset) => toggleCreativeAssetPickerSelection(
                  selection,
                  asset.id,
                  request.selectionLimit
                ),
                current
              ));
            })
            .catch(() => undefined);
        }}
        onCancel={() => settle(null)}
        onConfirm={() => settle([...selectedIds])}
      />
    ) : null,
  };
}
