# Window controls and redesign baseline

The native main window now keeps a transparent system titlebar, restoring AppKit resizable and minimizable style flags. macOS traffic lights provide close, minimize and fullscreen; a separate maximize/restore action uses native window zoom. The shared account shell owns the titlebar when a workspace is embedded.

Native close still flushes drafts, and a save failure keeps the window open. Targeted regression tests cover native capabilities, one titlebar owner, offline course/article entry, native close success/failure, and reading/code draft preservation.

`current-native.png` is a capture of the actual GPUI course screen using an isolated offline fixture. System screen capture failed with ScreenCaptureKit error -3811, so actual clicks on macOS traffic lights were not visually verified. The temporary review process was stopped; the user's running LeetDesk was left alone.

Product Design exploration will use this captured screen, the existing cream/crimson palette, Chinese reading hierarchy and bundled logo as the visual baseline. Design choices are pending selection; the existing course content is preserved.
