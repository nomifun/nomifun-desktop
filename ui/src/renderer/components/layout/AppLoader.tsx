import { Spin } from '@arco-design/web-react';
import React from 'react';

const AppLoader: React.FC<{ label?: string }> = ({ label }) => {
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
      <Spin dot />
      {label ? <span>{label}</span> : null}
    </div>
  );
};

export default AppLoader;
