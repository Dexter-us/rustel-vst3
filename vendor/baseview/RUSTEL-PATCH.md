# Embedded Windows DPI compatibility

Source: https://github.com/RustAudio/baseview
Revision: `9a0b42c09d712777b2edb4c5e0cb6baf21e988f0`
License: MIT OR Apache-2.0; original license files are retained.

Local change: `src/win/window.rs` no longer calls
`SetProcessDpiAwarenessContext` when opening a parented window. A plugin must
not change DPI awareness for the entire host process after its window exists.
Standalone window behavior is unchanged. No public APIs were changed.

This targets a reported first-open content offset in VSTHost. It is a candidate
fix, not a claim of visual verification in that host. Related upstream report:
https://github.com/RustAudio/baseview/issues/107

## Windows regression check

1. Completely exit VSTHost so its process-wide DPI state is reset.
2. Install the new bundle, launch VSTHost and load a fresh plugin instance.
3. On the very first editor opening, verify the header starts at the top,
   the score fills the left pane, and the right-hand controls are not clipped.
4. Verify that clicks and text selection line up with the drawn controls.
5. Close and reopen the editor several times; placement should stay identical.
   Also remove the plugin entirely and load a new instance without restarting
   VSTHost; the first instance and replacement must have identical placement.
6. Repeat at 100%, 125%, and 150% Windows display scaling, restarting VSTHost
   between runs. If available, repeat on monitors with different scaling.
7. Check that opening Rustel does not change the host or other plugins' scaling.
