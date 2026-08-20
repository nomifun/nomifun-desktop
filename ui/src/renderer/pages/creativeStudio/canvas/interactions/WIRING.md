# Canvas interaction integration

This directory is deliberately DOM-, storage-, and API-free. It projects
keyboard/pointer input into canonical `CanvasCommand`s and typed product
intents. The current integration points are:

- `CreativeCanvasEditor.handleKeyDown`: replace the local shortcut switch with
  `resolveCanvasKeyboardInput`, dispatch returned commands in order, and send
  intents to a ProductRoute callback. Keep `isCanvasKeyboardTarget` as the DOM
  adapter for the controller's `editable` flag.
- `CreativeCanvasEditor` node render context: expose context-menu,
  double-click, lock, four resize handles, and input/output connection handles.
  Pointer capture stays owned by the editor; use `start/update/finish` from the
  resize and connection controllers.
- `CreativeCanvasProductRoute`: render context/create-node overlays for typed
  intents. For `asset/import-file`, upload the exact `File` through the real
  asset port, wait for the returned `CreativeAsset`, then call the existing
  product node factory. Never create a URL or asset id in this layer.
- `CreativeCanvasProductRoute` node/edge presentation: use
  `deriveCanvasGraphHighlight` with its default one-hop depth for source parity.

Unresolved integration gaps are exported as
`CANVAS_INTERACTION_INTEGRATION_BLOCKERS`. In particular, system clipboard
media and blank-canvas connection creation cannot be completed safely by core
commands alone, and manual audio upload is not supported by the current NomiFun
asset contract.
