export type HubExtensionStatus =
  | 'not_installed'
  | 'installing'
  | 'installed'
  | 'install_failed'
  | 'update_available'
  | 'uninstalling';

export interface IHubExtension {
  name: string; // Extension unique ID
  display_name: string; // UI display name
  version?: string;
  description: string;
  author: string;
  icon?: string; // Path relative to extension root
  dist: {
    tarball: string; // Relative path e.g. extensions/ext-claude-code.tgz
    integrity: string; // SHA-512 SRI Hash
    unpackedSize: number;
  };
  engines: {
    nomifun: string; // Minimum APP version requirement
  };
  tags?: string[];
  bundled?: boolean; // Set at runtime by HubIndexManager for local bundled extensions
}

export interface IHubAgentItem extends IHubExtension {
  status: HubExtensionStatus;
  installError?: string; // Error message if install failed
}
