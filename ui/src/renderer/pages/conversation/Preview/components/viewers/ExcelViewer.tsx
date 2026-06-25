/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 * Based on AionUi (https://github.com/iOfficeAI/AionUi)
 */

import React from 'react';
import OfficeWatchViewer from './OfficeWatchViewer';

interface ExcelPreviewProps {
  file_path?: string;
  content?: string;
  workspace?: string;
}

const ExcelPreview: React.FC<ExcelPreviewProps> = (props) => <OfficeWatchViewer docType='excel' {...props} />;

export default ExcelPreview;
