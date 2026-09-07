/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

export type JavaScriptRuntimeSource = 'manual_path' | 'process_path' | 'managed';

export type JavaScriptRuntimeCompatibility =
  | 'recommended'
  | 'compatible'
  | 'incompatible';

export interface JavaScriptRuntimeRef {
  runtime_installation_id: string;
  node_version: string;
  runtime_target: string;
  executable_digest: string;
}

export interface JavaScriptRuntimeProbe {
  source: JavaScriptRuntimeSource;
  executable_path: string;
  compatibility: JavaScriptRuntimeCompatibility;
  runtime?: JavaScriptRuntimeRef;
  error_code?: string;
}

export type JavaScriptRuntimeDownloadState =
  | 'not_installed'
  | 'downloading'
  | 'ready'
  | 'failed';

export interface JavaScriptRuntimeDownload {
  download_revision: number;
  state: JavaScriptRuntimeDownloadState;
  downloaded_bytes?: number;
  total_bytes?: number;
  runtime?: JavaScriptRuntimeRef;
  error_code?: string;
}

export type RuntimeSwitchParticipantKind =
  | 'plugin_mount'
  | 'miniapp_service'
  | 'build_foundation';

export type RuntimeSwitchParticipantStatus =
  | 'passed'
  | 'failed'
  | 'not_covered';

export interface RuntimeSwitchParticipant {
  kind: RuntimeSwitchParticipantKind;
  owner_id: string;
  status: RuntimeSwitchParticipantStatus;
  error_code?: string;
}

export interface JavaScriptRuntimeDownloadOffer {
  offer_digest: string;
  node_version: string;
  runtime_target: string;
  archive_file_name: string;
  archive_size_bytes?: number;
}

export interface JavaScriptRuntimeStatus {
  selection_revision: number;
  selected?: JavaScriptRuntimeRef;
  pending_candidate?: JavaScriptRuntimeRef;
  probes: JavaScriptRuntimeProbe[];
  switch_participants: RuntimeSwitchParticipant[];
  requires_switch_decision: boolean;
  last_error_code?: string;
  non_recommended_warning_acknowledged: string[];
  download_offer?: JavaScriptRuntimeDownloadOffer;
  download: JavaScriptRuntimeDownload;
}

export type ProbeJavaScriptRuntimeRequest =
  | {
      source: 'auto_discover';
      expected_selection_revision: number;
    }
  | {
      source: 'manual_path';
      expected_selection_revision: number;
      executable_path: string;
    }
  | {
      source: 'managed_installation';
      expected_selection_revision: number;
      runtime_installation_id: string;
      expected_executable_digest: string;
    };

export interface ConfirmJavaScriptRuntimeDownloadRequest {
  expected_selection_revision: number;
  expected_offer_digest: string;
}

export interface BeginJavaScriptRuntimeSwitchRequest {
  expected_selection_revision: number;
  expected_selected_runtime_id?: string;
  expected_selected_executable_digest?: string;
  candidate_runtime_id: string;
  expected_candidate_executable_digest: string;
  acknowledge_non_recommended_runtime: boolean;
}

export type RuntimeSwitchDecision =
  | 'commit_candidate'
  | 'abort_and_restore_selected';

export interface DecideJavaScriptRuntimeSwitchRequest {
  expected_selection_revision: number;
  candidate_runtime_id: string;
  expected_candidate_executable_digest: string;
  decision: RuntimeSwitchDecision;
}
