/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { createInstance } from "i18next";
import React from "react";
import { I18nextProvider, initReactI18next } from "react-i18next";

const canvasTestI18n = createInstance();

await canvasTestI18n.use(initReactI18next).init({
  lng: "en-US",
  fallbackLng: false,
  resources: {
    "en-US": {
      translation: {
        creativeStudio: {
          canvas: {
            zoom: {
              resetViewCurrent:
                "creativeStudio.canvas.zoom.resetViewCurrent {{percentage}}",
            },
            chrome: {
              backgroundLabel:
                "creativeStudio.canvas.chrome.backgroundLabel {{background}}",
              resetRightPanelWidth:
                "creativeStudio.canvas.chrome.resetRightPanelWidth",
              resizeRightPanel:
                "creativeStudio.canvas.chrome.resizeRightPanel",
            },
            editor: {
              resizeHandle:
                "creativeStudio.canvas.editor.resizeHandle {{corner}}",
            },
            imageTools: {
              crop: {
                previewAlt:
                  "creativeStudio.canvas.imageTools.crop.previewAlt {{title}}",
                resizeBox:
                  "creativeStudio.canvas.imageTools.crop.resizeBox {{handle}}",
                metrics: {
                  size: "creativeStudio.canvas.imageTools.crop.metrics.size {{value}}",
                  ratio:
                    "creativeStudio.canvas.imageTools.crop.metrics.ratio {{value}}",
                  original:
                    "creativeStudio.canvas.imageTools.crop.metrics.original {{value}}",
                },
              },
              mask: {
                previewAlt:
                  "creativeStudio.canvas.imageTools.mask.previewAlt {{title}}",
              },
              split: {
                summary:
                  "creativeStudio.canvas.imageTools.split.summary {{count}}",
                previewAlt:
                  "creativeStudio.canvas.imageTools.split.previewAlt {{title}}",
                lineLabel:
                  "creativeStudio.canvas.imageTools.split.lineLabel {{axis}} {{index}} {{percentage}}",
                pieceCount:
                  "creativeStudio.canvas.imageTools.split.pieceCount {{count}}",
              },
            },
            nodes: {
              panorama: {
                fieldOfView:
                  "creativeStudio.canvas.nodes.panorama.fieldOfView {{value}}",
                orientation:
                  "creativeStudio.canvas.nodes.panorama.orientation {{yaw}} {{pitch}}",
              },
              config: {
                summary:
                  "creativeStudio.canvas.nodes.config.summary {{parameters}} {{inputs}}",
              },
            },
          },
        },
      },
    },
  },
  interpolation: { escapeValue: false },
});

export const withCanvasTestI18n = (content: React.ReactNode) => (
  <I18nextProvider i18n={canvasTestI18n}>{content}</I18nextProvider>
);
