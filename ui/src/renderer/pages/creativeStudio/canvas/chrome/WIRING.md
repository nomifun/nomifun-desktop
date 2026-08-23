# Creative Canvas chrome wiring

`CreativeCanvasChrome` is a controlled, source-shaped three-column shell. The
left and right panels consume real layout width, so the center Canvas becomes
narrower exactly as it does in the reference product. The bottom tool dock then
scrolls horizontally when the remaining center column is compact.

Compose the editor through the `canvas` slot and inject product surfaces into
the panel slots:

```tsx
<CreativeCanvasChrome
  canvasTitle={canvas.title}
  saveStatus={save.status}
  tool={tool}
  background={background}
  canUndo={canUndo}
  canRedo={canRedo}
  leftView={leftView}
  rightView={rightView}
  bottomView={bottomView}
  backgroundMenuOpen={backgroundMenuOpen}
  slots={{
    canvas: (
      <CreativeCanvasEditor
        isMiniMapOpen={miniMapOpen}
        onToggleMiniMap={toggleMiniMap}
        {...editorProps}
      />
    ),
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

The editor's built-in zoom dock is the only owner of fit-view and minimap
actions. The outer chrome intentionally omits those controls so the bottom
surface has no duplicate entry. The Canvas route/controller translates the
remaining callbacks into canonical core commands and constructs new canonical
nodes. This layer never creates IDs, persists a Canvas document, invokes a
model, resolves assets, or fabricates panel content.

The hand icon is a pressed-state toggle: pressed selects the existing pan tool,
while unpressed leaves the editor in its default selection mode. The bottom
toolbar exposes one History icon for opening or closing the shared bottom
panel; History and Timeline remain tabs inside that panel.

The bottom dock exposes text, image, video, audio, panorama, Director, and
generation-config creation directly in the reference order. Group creation is
intentionally not a node-creation tool; it remains a selection action.

`rightView` and `bottomView` use `null` for a closed panel. Menu visibility is
controlled for the remaining background picker so route transitions can close
transient UI deterministically.

Canvas owner identity is supplied by the route. When an owner is needed, use
the canonical `CanvasNode { canvasId, nodeId }` shape and serialize it as
`{ canvas_id, node_id }` at the HTTP boundary; the chrome does not invent either
identifier.

## Compatibility only

Legacy `nomifun.creative-studio/v1` project-document and repository-adapter
names may remain below this boundary during migration. They are compatibility
facts only and are not Canvas product concepts or chrome callbacks.
