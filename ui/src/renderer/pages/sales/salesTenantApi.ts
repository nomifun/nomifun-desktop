import { httpRequest } from '@/common/adapter/httpBridge';
import type { SalesWorkspace } from './salesWorkspace';

type SalesWorkspaceResponse = {
  success: boolean;
  workspace: unknown;
  updated_at: number | null;
};

export type SalesAccess = {
  isInstanceOwner: boolean;
};

export type ManagedSalesUser = {
  userId: string;
  username: string;
  isOwner: boolean;
  createdAt: number;
  lastLogin: number | null;
};

type ManagedSalesUserWire = {
  user_id: string;
  username: string;
  is_owner: boolean;
  created_at: number;
  last_login: number | null;
};

const mapManagedUser = (user: ManagedSalesUserWire): ManagedSalesUser => ({
  userId: user.user_id,
  username: user.username,
  isOwner: user.is_owner,
  createdAt: user.created_at,
  lastLogin: user.last_login,
});

export async function loadSalesWorkspace(): Promise<unknown> {
  const response = await httpRequest<SalesWorkspaceResponse>('GET', '/api/sales/workspace');
  return response.workspace;
}

export async function saveSalesWorkspace(workspace: SalesWorkspace, expectedUserId: string): Promise<void> {
  await httpRequest<SalesWorkspaceResponse>('PUT', '/api/sales/workspace', {
    expected_user_id: expectedUserId,
    workspace,
  });
}

export async function loadSalesAccess(): Promise<SalesAccess> {
  const response = await httpRequest<{ success: boolean; is_instance_owner: boolean }>(
    'GET',
    '/api/sales/access'
  );
  return { isInstanceOwner: response.is_instance_owner };
}

export async function listManagedSalesUsers(): Promise<ManagedSalesUser[]> {
  const response = await httpRequest<{ success: boolean; users: ManagedSalesUserWire[] }>(
    'GET',
    '/api/sales/admin/users'
  );
  return response.users.map(mapManagedUser);
}

export async function createManagedSalesUser(username: string, password: string): Promise<ManagedSalesUser> {
  const response = await httpRequest<{ success: boolean; user: ManagedSalesUserWire }>(
    'POST',
    '/api/sales/admin/users',
    { username, password }
  );
  return mapManagedUser(response.user);
}

export async function resetManagedSalesUserPassword(userId: string, password: string): Promise<void> {
  await httpRequest<{ success: boolean }>(
    'POST',
    `/api/sales/admin/users/${encodeURIComponent(userId)}/password`,
    { password }
  );
}
