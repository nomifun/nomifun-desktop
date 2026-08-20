# Creative canvas chrome wiring

`CreativeCanvasChrome` is a controlled, source-shaped three-column shell. The
left and right panels consume real layout width, so the center canvas becomes
narrower exactly as it does in the reference product. The bottom tool dock then
scrolls horizontally when the remaining center column is compact.

Compose the editor through the `canvas` slot and inject product surfaces into
the panel slots:

```tsx
<CreativeCanvasChrome
  projectTitle={project.title}
  saveStatus={save.status}
  tool={tool}
  background={background}
  canUndo={canUndo}
  canRedo={canRedo}
  isMiniMapOpen={miniMapOpen}
  leftView={leftView}
  rightView={rightView}
  bottomView={bottomView}
  nodeMenuOpen={nodeMenuOpen}
  backgroundMenuOpen={backgroundMenuOpen}
  slots={{
    canvas: <CreativeCanvasEditor showZoomControls={false} {...editorProps} />,
    left: {
      canvas: <CanvasOutline />,
      assets: <CreativeAssetLibrary {...assetProps} />,
      prompts: <PromptLibrarySurface {...promptProps} />,
      workflows: <WorkflowLibrary />,
    },
    right: {
      assistant: <CreativeStudioAgentPanel {...agentProps} />,
      properties: <CreativeNodeProperties />,
    },
    bottom: {
      history: <CreativeCanvasHistory />,
      timeline: <CreativeCanvasTimeline />,
    },
  }}
  {...callbacks}
/>
```

Disable the editor's built-in zoom dock when using this chrome's fit and
minimap actions, so those actions are not duplicated. The route/controller is
responsible for translating callbacks into canonical core commands and for
constructing new canonical nodes. This layer never creates IDs, persists a
document, invokes a model, resolves assets, or fabricates panel content.

`rightView` and `bottomView` use `null` for a closed panel. Menu visibility is
also controlled so route transitions can close transient UI deterministically.
