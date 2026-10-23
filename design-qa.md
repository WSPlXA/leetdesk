# Reading-first native workspace — design QA

Source: [selected reference](design/reading-first-reference.png), 1488 × 1057.
Implementation: the actual Rust/GPUI application, captured from its own GPU renderer. Isolated offline fixture data; no personal account or workspace data is used.

Final result: **blocked — awaiting the revised visual comparison**.

## Evidence and normalization

- First exploratory native capture was 1904 × 1015 because the Windows display constrained initial bounds. It was not used for a pixel-aligned comparison.
- Explicitly resizing the native content viewport produced [native v2](design/reading-first-native-v2.png), exactly 1488 × 1057 at scale 1.
- [Combined comparison](design/comparison-full.png) places source and native output in the same image, without stretching either. Additional navigation, article, and toolbar crops are saved beside it.
- Matching state: Chinese, beige, problem 994, lesson tab, initial reading position. Fixture plan is **All problems**, preserving real Hot 100 membership rather than falsely presenting problems 127/752/1091 as Hot 100 members. Full product navigation retains the 12 existing chapters.

## Findings and fixes awaiting recapture

| Priority | Finding | Fix |
| --- | --- | --- |
| P1 | Native button labels centered chapter and method titles. | Use left-aligned full-width child labels and explicit accessible names. |
| P1 | The article view overrode the state's scrolling flag; the outline did not move the article. | Enable scrolling on the rendered TextView itself. A native interaction test now asserts the actual list scroll offset changes without changing selection. |
| P2 | Primary CTA remained black despite the red theme. | Synchronize Component/Base theme tokens and set the separate button-primary palette. |
| P2 | Heading, tab, and CTA type was smaller than the reference. | Increase the problem heading, section headings, tab text, and CTA text; preserve the native Windows font. |
| P2 | Empty methods consumed sidebar space. | Show loaded methods and explicitly expanded methods; retain chapter overview access. |
| P2 | Table rows were too tall; header text used a different color. | Reduce cell vertical padding and use the normal ink color. |
| P2 | Content could paint across the status bar. | Clip the bounded article viewport. |

Required fidelity surfaces reviewed: typography, layout rhythm, theme tokens, supplied logo/library icon quality, and real problem copy. Functional content intentionally includes the full explanation and exact verified Go implementation, rather than replacing it with screenshot text.

Native interaction checks cover article anchors, persisted mode, code/cursor/notes/test draft preservation, keyboard mode shortcuts, and search focus. A final pass still requires the revised captures below.
