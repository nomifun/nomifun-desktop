import React from 'react';
import { Navigate, useLocation } from 'react-router-dom';
import { legacyEnhancedToolsUrl, type EnhancedToolsCategory } from './routes';

const LegacyEnhancedToolsRedirect: React.FC<{ category: EnhancedToolsCategory }> = ({ category }) => {
  const { search, hash, state } = useLocation();
  return <Navigate to={legacyEnhancedToolsUrl(category, search, hash)} state={state} replace />;
};

export default LegacyEnhancedToolsRedirect;
