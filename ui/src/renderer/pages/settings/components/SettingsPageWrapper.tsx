import classNames from 'classnames';
import React from 'react';
import './settings-workspace.css';

interface SettingsPageWrapperProps {
  children: React.ReactNode;
  className?: string;
  contentClassName?: string;
}

const SettingsPageWrapper: React.FC<SettingsPageWrapperProps> = ({ children, className, contentClassName }) => {
  const containerClass = classNames('settings-page-wrapper', className);
  const contentClass = classNames('settings-page-content', contentClassName);

  return (
    <div className={containerClass}>
      <div className={contentClass}>{children}</div>
    </div>
  );
};

export default SettingsPageWrapper;
