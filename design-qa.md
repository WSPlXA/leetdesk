# Reading-first native workspace — design QA

Source: [selected reference](design/reading-first-reference.png), 1488 × 1057.
Implementation: the actual Rust/GPUI application, captured from its own GPU renderer. Isolated offline fixture data; no personal account or workspace data is used.

Final result: **pass**. The revised native captures were compared with the selected reference in the same combined image, followed by navigation, article, and toolbar crops. No open P0/P1/P2 findings remain for this implementation scope.

## Evidence and normalization

- First exploratory native capture was 1904 × 1015 because the Windows display constrained initial bounds. It was not used for a pixel-aligned comparison.
- Explicitly resizing the native content viewport produced [native v2](design/reading-first-native-v2.png), exactly 1488 × 1057 at scale 1.
- [Combined comparison](design/comparison-full.png) places source and native output in the same image, without stretching either. Additional navigation, article, and toolbar crops are saved beside it.
- Matching state: Chinese, beige, problem 994, lesson tab, initial reading position. Fixture plan is **All problems**, preserving real Hot 100 membership rather than falsely presenting problems 127/752/1091 as Hot 100 members. Full product navigation retains the 12 existing chapters.

## Findings and verified fixes

| Priority | Finding | Fix |
| --- | --- | --- |
| P1 | Native button labels centered chapter and method titles. | Use left-aligned full-width child labels and explicit accessible names. |
| P1 | The article view overrode the state's scrolling flag; the outline did not move the article. | Enable scrolling on the rendered TextView itself. A native interaction test now asserts the actual list scroll offset changes without changing selection. |
| P2 | Primary CTA remained black despite the red theme. | Synchronize Component/Base theme tokens and set the separate button-primary palette. |
| P2 | Heading, tab, and CTA type was smaller than the reference. | Increase the problem heading, section headings, tab text, and CTA text; preserve the native Windows font. |
| P2 | Empty methods consumed sidebar space. | Show loaded methods and explicitly expanded methods; retain chapter overview access. |
| P2 | Table rows were too tall; header text used a different color. | Reduce cell vertical padding and use the normal ink color. |
| P2 | Content could paint across the status bar. | Clip the bounded article viewport. |
| P2 | Coding-mode article touched the pane divider. | Restore 16 px content gutters in coding mode; recaptured and inspected the revised native coding window. |

Required fidelity surfaces reviewed: typography, layout rhythm, theme tokens, supplied logo/library icon quality, and real problem copy. Functional content intentionally includes the full explanation and exact verified Go implementation, rather than replacing it with screenshot text.

## Final comparison

- [Final native reading screen](design/reading-first-native.png): 1488 × 1057, scale 1. [Full paired comparison](design/comparison-full.png), [navigation crop](design/comparison-navigation.png), [article crop](design/comparison-article.png), [toolbar crop](design/comparison-toolbar.png).
- Typography: 40 px problem title, 26 px section headings, 18 px reading body, 16 px tabs and problem rows. Prefer installed Noto Sans SC; fall back to the existing platform UI font. Body, headings and table text explicitly use the same UI family. Code stays monospace and follows the selected high-contrast palette.
- Layout: 320 px unified sidebar, article starting at x=400, 248 px outline, 56 px top bar, bounded independent article scroll. Selected rows use a vertical marker and weight in addition to color. Problem rows now follow the curated practice order, with full titles available on hover.
- Colors: warm flat surfaces, dark ink and crimson actions; primary button tokens now agree with the application palette. Table header/body are distinct, and code remains red with weight cues. Decorative paper grain from the generated mock is intentionally omitted.
- Image/icon quality: original supplied application logo and bundled vector icons; no screenshot text or controls are rasterized into the interface.
- Copy/content: the actual detailed explanations, verified Go source, exact four-minute example table, category guide and imported material remain live content. The proof includes the layer-size invariant and unreachable case, so its fold differs slightly from the shorter concept. The native reader uses syntax-highlighted fenced code with copy actions; line numbers remain in the code editor.

Additional states inspected after capture:

- [1120 × 800 reading window](design/reading-narrow-native.png): outline collapses, article and sidebar remain bounded; long problem names ellipsize and expose full names on hover.
- [White / English reading window](design/reading-white-english-native.png): English navigation, headings and outline; no layout overlap.
- [Dark / English coding window](design/coding-dark-english-native.png): final 16 px document gutter, editor, visible test input, translated Diagnostics tab and high-contrast red syntax.

Native interaction tests verify article anchors by checking the actual list scroll position, persisted mode, code/cursor/notes/test draft preservation, keyboard mode shortcuts, and search focus. Full regression: 45 core + 46 desktop/login tests passed; the reading-mode interaction test also passed after the final coding-gutter adjustment. Strict Clippy and both native builds passed. Six final smoke starts cover both binaries across dark/China, white/Global and beige/Global preferences.

This is a Windows native implementation review, not a claim of pixel identity with generated artwork or a macOS visual certification.
