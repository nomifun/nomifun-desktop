import { httpRequest } from './httpBridge';
import type {
  AuthStartRequest, AuthorizationFlow, ConnectionUpdate, LogoutRequest, OfficeConnection, OfficeOperation, OfficeProfile, OfficeQuery,
  OfficeQueryResult, OfficeStatus, ExecuteRequest,
  RuntimeInfo, RuntimeUpdateInfo,
} from '../types/officeHub';

const root = '/api/office-hub';
const operationPath = (id: string) => root + '/operations/' + encodeURIComponent(id);

/** A closed business API. The renderer never supplies CLI arguments or shell text. */
export const officeHub = {
  status: (signal?: AbortSignal) => httpRequest<OfficeStatus>('GET', root + '/status', undefined, { signal }),
  profiles: (signal?: AbortSignal) => httpRequest<OfficeProfile[]>('GET', root + '/profiles', undefined, { signal }),
  prepare: () => httpRequest<RuntimeInfo>('POST', root + '/runtime/prepare'),
  checkUpdate: () => httpRequest<RuntimeUpdateInfo>('POST', root + '/runtime/check-update', {}),
  selectRuntime: (executable: string | null) => httpRequest<RuntimeInfo>('POST', root + '/runtime/select', { executable }),
  updateConnection: (body: ConnectionUpdate) => httpRequest<OfficeConnection>('PUT', root + '/connection', body),
  startAuth: (body: AuthStartRequest) => httpRequest<AuthorizationFlow>('POST', root + '/auth', body),
  logout: (body: LogoutRequest) => httpRequest<void>('POST', root + '/auth/logout', body),
  auth: (id: string, signal?: AbortSignal) => httpRequest<AuthorizationFlow>('GET', root + '/auth/' + encodeURIComponent(id), undefined, { signal }),
  cancelAuth: (id: string) => httpRequest<AuthorizationFlow>('DELETE', root + '/auth/' + encodeURIComponent(id)),
  query: (body: OfficeQuery, signal?: AbortSignal) => httpRequest<OfficeQueryResult>('POST', root + '/query', body, { signal }),
  execute: (body: ExecuteRequest) => httpRequest<OfficeOperation>('POST', root + '/operations', body, { idempotencyKey: body.request_id }),
  operation: (id: string, signal?: AbortSignal) => httpRequest<OfficeOperation>('GET', operationPath(id), undefined, { signal }),
  request: (id: string, signal?: AbortSignal) => httpRequest<OfficeOperation>('GET', root + '/operations/request/' + encodeURIComponent(id), undefined, { signal }),
  reconcile: (id: string) => httpRequest<OfficeOperation>('POST', operationPath(id) + '/reconcile', {}),
};
