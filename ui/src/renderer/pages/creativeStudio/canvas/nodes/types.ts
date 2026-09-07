/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type React from 'react';

import type {
  CreativeCanvasNode,
  CreativeCanvasNodeKind,
  CreativeGenerationStatus,
} from '../../domain/schema';

/** A type-level projection only; persisted node fields remain owned by schema.ts. */
export type CreativeNodeOfKind<K extends CreativeCanvasNodeKind> = Extract<CreativeCanvasNode, { type: K }>;

/** Resolved media supplied by the asset boundary. No URL is persisted in a node. */
export interface CreativeNodeAssetPresentation {
  src: string;
  originalSrc?: string;
  deleted?: boolean;
  posterSrc?: string;
  label?: string;
  alt?: string;
}

/** Transient task state supplied by the runtime; this is never a document shape. */
export interface CreativeNodeRuntimePresentation {
  status: CreativeGenerationStatus;
  progress?: number | null;
  label?: string;
  errorMessage?: string | null;
}

export type CreativeNodePlacement = 'world' | 'contained';

export interface CreativeNodePresentationProps<K extends CreativeCanvasNodeKind> {
  node: CreativeNodeOfKind<K>;
  selected?: boolean;
  placement?: CreativeNodePlacement;
  runtime?: CreativeNodeRuntimePresentation;
  className?: string;
  style?: React.CSSProperties;
  headerActions?: React.ReactNode;
  inputHandle?: React.ReactNode;
  outputHandle?: React.ReactNode;
  onActivate?: (node: CreativeNodeOfKind<K>) => void;
  onOpen?: (node: CreativeNodeOfKind<K>) => void;
  onToggleLock?: (node: CreativeNodeOfKind<K>) => void;
  onPointerDown?: React.PointerEventHandler<HTMLElement>;
  onContextMenu?: React.MouseEventHandler<HTMLElement>;
}
