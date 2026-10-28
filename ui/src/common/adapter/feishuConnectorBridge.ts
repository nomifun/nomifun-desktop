import { httpRequest } from './httpBridge';
import type {
  AuthStartRequest, AuthorizationFlow, ConnectionUpdate, LogoutRequest, FeishuConnection, FeishuOperation, FeishuProfile, FeishuQuery,
  FeishuQueryResult, FeishuStatus, ExecuteRequest,
  RuntimeInfo, RuntimeUpdateInfo,
} from '../types/feishuConnector';

const root = '/api/feishu-connector';
const operationPath = (id: string) => root + '/operations/' + encodeURIComponent(id);

/** A closed business API. The renderer never supplies CLI arguments or shell text. */
export const feishuConnector = {
  status: (signal?: AbortSignal) => httpRequest<FeishuStatus>('GET', root + '/status', undefined, { signal }),
  profiles: (signal?: AbortSignal) => httpRequest<FeishuProfile[]>('GET', root + '/profiles', undefined, { signal }),
  prepare: () => httpRequest<RuntimeInfo>('POST', root + '/runtime/prepare'),
  repair: () => httpRequest<RuntimeInfo>('POST', root + '/runtime/repair'),
  checkUpdate: () => httpRequest<RuntimeUpdateInfo>('POST', root + '/runtime/check-update', {}),
  selectRuntime: (executable: string | null) => httpRequest<RuntimeInfo>('POST', root + '/runtime/select', { executable }),
  updateConnection: (body: ConnectionUpdate) => httpRequest<FeishuConnection>('PUT', root + '/connection', body),
  startAuth: (body: AuthStartRequest) => httpRequest<AuthorizationFlow>('POST', root + '/auth', body),
  logout: (body: LogoutRequest) => httpRequest<void>('POST', root + '/auth/logout', body),
  auth: (id: string, signal?: AbortSignal) => httpRequest<AuthorizationFlow>('GET', root + '/auth/' + encodeURIComponent(id), undefined, { signal }),
  cancelAuth: (id: string) => httpRequest<AuthorizationFlow>('DELETE', root + '/auth/' + encodeURIComponent(id)),
  query: (body: FeishuQuery, signal?: AbortSignal) => httpRequest<FeishuQueryResult>('POST', root + '/query', body, { signal }),
  execute: (body: ExecuteRequest) => httpRequest<FeishuOperation>('POST', root + '/operations', body, { idempotencyKey: body.request_id }),
  operation: (id: string, signal?: AbortSignal) => httpRequest<FeishuOperation>('GET', operationPath(id), undefined, { signal }),
  request: (id: string, signal?: AbortSignal) => httpRequest<FeishuOperation>('GET', root + '/operations/request/' + encodeURIComponent(id), undefined, { signal }),
  reconcile: (id: string) => httpRequest<FeishuOperation>('POST', operationPath(id) + '/reconcile', {}),
};
