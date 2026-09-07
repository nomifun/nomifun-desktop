/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  BeginJavaScriptRuntimeSwitchRequest,
  ConfirmJavaScriptRuntimeDownloadRequest,
  DecideJavaScriptRuntimeSwitchRequest,
  JavaScriptRuntimeStatus,
  ProbeJavaScriptRuntimeRequest,
} from '../types/javascriptRuntime';
import { httpGet, httpPost, httpRequest } from './httpBridge';

export const javascriptRuntime = {
  status: httpGet<JavaScriptRuntimeStatus, void>(
    '/api/javascript-runtime/status'
  ),
  probe: {
    provider: () => {},
    invoke: (request: ProbeJavaScriptRuntimeRequest) =>
      httpRequest<JavaScriptRuntimeStatus>(
        'POST',
        '/api/javascript-runtime/probe',
        request
      ),
  },
  download: httpPost<
    JavaScriptRuntimeStatus,
    ConfirmJavaScriptRuntimeDownloadRequest
  >('/api/javascript-runtime/download'),
  beginSwitch: httpPost<
    JavaScriptRuntimeStatus,
    BeginJavaScriptRuntimeSwitchRequest
  >('/api/javascript-runtime/switch/begin'),
  decideSwitch: httpPost<
    JavaScriptRuntimeStatus,
    DecideJavaScriptRuntimeSwitchRequest
  >('/api/javascript-runtime/switch/decision'),
};
