import { Spin } from '@arco-design/web-react';
import React from 'react';

const AppLoader: React.FC<{ label?: string; loading?: boolean; children?: React.ReactNode }> = ({ label, loading = true, children }) => {
  return (
    <div
      role='status'
      aria-live='polite'
      style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        minHeight: '100vh',
        flexDirection: 'column',
        gap: 12,
        color: 'var(--color-text-2)',
      }}
    >
      {loading && <Spin dot />}
      {label ? <span>{label}</span> : null}
      {children}
    </div>
  );
};

export default AppLoader;
