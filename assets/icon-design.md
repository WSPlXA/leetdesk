# LeetDesk 图标

使用内置 `image_gen` 生成，未使用 API/CLI 回退。

- 原图：`assets/leetdesk-icon.png`，带透明背景。
- Windows 图标：`assets/leetdesk.ico`，含 16、20、24、32、40、48、64、128、256 像素规格，保留透明度。
- `assets/windows.rc` 将图标嵌入程序资源 ID 1；GPUI Windows 后端用此 ID 加载窗口与任务栏图标。ICO 由原图做格式导出，没有重绘图案。

生成提示词：

```text
Use case: logo-brand. Create one finished 1024x1024 square application icon for LeetDesk, a native desktop editor for practicing coding problems in Go. Match its existing UI palette: deep navy #141D2B and mint green #78DDBD, with off-white #DCE5F2. A bold simple central coding symbol: a pair of mint angle brackets framing a confident off-white check mark, subtly evoking a small practice card. Rounded-square deep navy tile occupies almost the whole canvas with an even small margin. Flat vector-like geometry, crisp smooth edges, balanced spacing, thick strokes, no thin details, excellent legibility at 16 and 32 pixels. Transparent pixels outside the rounded-square tile. Straight-on icon asset, no mockup, no perspective, no letters or text, no watermark. Original design; do not copy any existing company logo.
```
