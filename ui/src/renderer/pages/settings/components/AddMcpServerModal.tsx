import type { IMcpServer } from '@/common/config/storage';
import React from 'react';
import JsonImportModal from './JsonImportModal';
import OneClickImportModal from './OneClickImportModal';

interface AddMcpServerModalProps {
  visible: boolean;
  server?: IMcpServer;
  existingServerNames?: string[];
  onCancel: () => void;
  onSubmit: (server: Omit<IMcpServer, 'mcp_server_id' | 'created_at' | 'updated_at'>) => Promise<void> | void;
  onBatchImport?: (
    servers: Omit<IMcpServer, 'mcp_server_id' | 'created_at' | 'updated_at'>[]
  ) => Promise<IMcpServer[] | void> | IMcpServer[] | void;
  importMode?: 'json' | 'oneclick';
}

const AddMcpServerModal: React.FC<AddMcpServerModalProps> = ({
  visible,
  server,
  existingServerNames = [],
  onCancel,
  onSubmit,
  onBatchImport,
  importMode = 'json',
}) => {
  if (!visible) return null;

  // The selected entry determines the dialog. Agent availability is checked
  // by OneClickImportModal itself, with a visible failure and retry action.
  if (!server && importMode === 'oneclick') {
    return (
      <OneClickImportModal
        visible
        existingServerNames={existingServerNames}
        onCancel={onCancel}
        onBatchImport={onBatchImport}
      />
    );
  }

  return (
    <JsonImportModal
      visible
      server={server}
      onCancel={onCancel}
      onSubmit={onSubmit}
      onBatchImport={onBatchImport}
    />
  );
};

export default AddMcpServerModal;
