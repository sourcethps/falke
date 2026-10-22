# egui-d3d9 (vendored)

Copied from https://github.com/sourcethps/egui-d3d9, branch
`feature/fix-alt-tab-crash` (commit c2e2bdd), MIT licensed (see LICENSE).

Changes for falke:

- `src/state.rs`: before drawing, extra render targets (1-3), the depth
  buffer and instanced stream frequencies are unbound, and put back after.
  State blocks don't cover these. A game or mod that ends the frame with a
  second render target, a depth buffer of another size / multisampling, or
  instancing on made every UI draw fail with D3DERR_INVALIDCALL
  (0x8876086C, "unable to draw indexed prims").
- `src/state.rs`: the viewport is clamped to the back buffer size (a
  window's client area can be larger than the back buffer).
- `src/app.rs`: a failed draw skips the rest of that frame's UI instead of
  panicking (and taking the game down).
