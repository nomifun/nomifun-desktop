/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { LocalModelErrorKind, LocalModelInstallPhase } from './localModelService';

/** Components in the curated PP-OCR bundle, as serialized by the Rust API. */
export type OcrModelComponent =
  | 'detector'
  | 'detector_config'
  | 'recognizer'
  | 'recognizer_config';

/** Immutable, path-free metadata for a managed OCR bundle. */
export interface OcrModelCatalogEntry {
  id: string;
  name: string;
  description: string;
  format: string;
  downloadSizeBytes: number;
  requiredMemoryBytes: number;
  license: string;
  source: string;
  components: OcrModelComponent[];
  recommended: boolean;
}

/** Current component transfer plus aggregate progress for the whole bundle. */
export interface OcrModelTransferProgress {
  component: OcrModelComponent;
  downloadedBytes: number;
  totalBytes: number;
  overallDownloadedBytes: number;
  overallTotalBytes: number;
  bytesPerSecond: number;
}

export interface OcrModelState {
  modelId: string;
  installPhase: LocalModelInstallPhase;
  progress: OcrModelTransferProgress | null;
  installedBytes: number;
  errorKind: LocalModelErrorKind | null;
  /** Backend-sanitized user-facing detail; never contains paths or URLs. */
  message: string | null;
}

/**
 * Artifact readiness and inference readiness are deliberately independent.
 * Downloaded ONNX files must not be presented as usable until the runtime is
 * wired and reports `inferenceReady`.
 */
export interface OcrModelServiceStatus {
  protocolVersion: string;
  artifactsReady: boolean;
  inferenceReady: boolean;
  models: OcrModelState[];
  lastError: string | null;
}

export interface OcrModelIdRequest {
  id: string;
}
