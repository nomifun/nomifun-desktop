import type {
  ChannelPluginId,
  ChannelSessionId,
  ChannelUserId,
  CompanionId,
  ConversationId,
} from '@/common/types/ids';

/**
 * 渠道机器人所有权分域：桌面伙伴域 / 客服域。两域彻底互斥、不共享挑选池 ——
 * 客服在自己页面创建/管理/绑定自己的 bot，伙伴侧 UI 只见 companion 域。
 */
export type ChannelOwnerDomain = 'companion' | 'customer_service';

export interface IChannelPluginStatus {
  /** Stable `channel_plugins.channel_plugin_id` business identity. */
  plugin_id: ChannelPluginId;
  type: string;
  name: string;
  enabled: boolean;
  connected: boolean;
  status?: string;
  last_connected?: number;
  error?: string;
  activeUsers: number;
  botUsername?: string;
  hasToken?: boolean;
  /** 所有权分域；`customer_service` 域的 bot 绝不携带 companionId（后端触发器互斥）。 */
  owner_domain: ChannelOwnerDomain;
  /** 绑定的伙伴（每机器人一宠；UNIQUE(type,bot_key) 保证同一机器人不绑多宠）。 */
  companionId?: CompanionId;
  /** 平台级机器人身份（lark app_id / telegram bot id / ...）。 */
  botKey?: string;
  isExtension?: boolean;
  extensionMeta?: {
    credentialFields?: Array<{
      key: string;
      label: string;
      type: 'text' | 'password' | 'select' | 'number' | 'boolean';
      required?: boolean;
      options?: string[];
      default?: string | number | boolean;
    }>;
    configFields?: Array<{
      key: string;
      label: string;
      type: 'text' | 'password' | 'select' | 'number' | 'boolean';
      required?: boolean;
      options?: string[];
      default?: string | number | boolean;
    }>;
    description?: string;
    extensionName?: string;
    icon?: string;
  };
}

export interface IChannelPairingRequest {
  code: string;
  platformUserId: string;
  platformType: string;
  display_name?: string;
  requestedAt: number;
  expiresAt: number;
  /** 发起/归属的机器人渠道业务 ID。 */
  channel_plugin_id?: ChannelPluginId;
}

export interface IChannelUser {
  channel_user_id: ChannelUserId;
  platformUserId: string;
  platformType: string;
  display_name?: string;
  authorizedAt: number;
  lastActive?: number;
  channel_session_id?: ChannelSessionId;
  /** 发起/归属的机器人渠道业务 ID。 */
  channel_plugin_id?: ChannelPluginId;
}

export interface IChannelSession {
  channel_session_id: ChannelSessionId;
  channel_user_id: ChannelUserId;
  agent_type: string;
  conversation_id?: ConversationId;
  workspace?: string;
  chatId?: string;
  channel_plugin_id?: ChannelPluginId;
  created_at: number;
  lastActivity: number;
}
