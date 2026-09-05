import { Button, Skeleton } from '@arco-design/web-react';
import React from 'react';
import { Outlet } from 'react-router-dom';
import { SalesWorkspaceProvider, useSalesWorkspace } from './SalesWorkspaceContext';
import './sales.css';

const SalesShellContent: React.FC = () => {
  const { workspaceReady, workspaceLoaded, workspaceError, retryWorkspaceLoad } = useSalesWorkspace();

  if (!workspaceReady) {
    return (
      <main className='sales-shell'>
        <div className='sales-page sales-workspace-loading' aria-label='正在加载当前账号的销售工作台'>
          <Skeleton text={{ rows: 6 }} animation />
        </div>
      </main>
    );
  }

  if (!workspaceLoaded) {
    return (
      <main className='sales-shell'>
        <div className='sales-workspace-blocked' role='alert'>
          <strong>当前账号的销售工作台未加载</strong>
          <span>{workspaceError || '请重新加载后再操作，避免把数据保存到错误账号。'}</span>
          <Button onClick={retryWorkspaceLoad}>重新加载</Button>
        </div>
      </main>
    );
  }

  return (
    <main className='sales-shell'>
      {workspaceError ? (
        <div className='sales-workspace-error' role='alert'>
          <span>当前账号的数据尚未同步：{workspaceError}</span>
          <Button size='mini' onClick={retryWorkspaceLoad}>重试</Button>
        </div>
      ) : null}
      <Outlet />
    </main>
  );
};

const SalesShell: React.FC = () => (
  <SalesWorkspaceProvider>
    <SalesShellContent />
  </SalesWorkspaceProvider>
);

export default SalesShell;
