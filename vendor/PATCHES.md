# GPUI Base 本地补丁

原始版本：gpui-base 0.7.3，保留 `gpui-base/LICENSE-APACHE`。
通过 Cargo 的 `[patch.crates-io]` 使用本目录，不修改用户的全局 Cargo 缓存。

修改文件：

- `src/text/inline_flow.rs`：行内片段先统一绘制背景，再绘制文字，避免字体回退、舍入或字形越界时相邻代码块背景遮字。
- `src/text/format/html.rs`：将 `<pre>` 解析为保留换行的代码块，包含内部 `<strong>/<code>` 的实体解码文字，用于官方题面示例。
- `src/text/node.rs`：HTML 段落使用自身状态的稳定标识，替代共用的 `p` 标识，避免布局和文本片段缓存串用；复制使用经过字节范围校验的选区。
- `src/text/inline.rs`：片段文本变化时清除旧选区；无效范围（含 UTF-8 字符中间位置）不参与复制，避免直接字符串切片导致应用退出。

- `src/input/base/state.rs`：增加局部编辑 API，保留所有选择区的身份和方向、视口及折叠状态；用一个原子事务记录格式化，保护输入法组合文本；暴露文档版本和同步 Tab 动作。选中替换后的连续字符输入按同一输入事务合并。
- `src/input/base/undo_manager.rs`：允许上述替换起始的输入事务继续合并相邻字符。
- `src/input/base/rope_ext.rs`：LSP Position 的列统一使用 UTF-16，修复 emoji 后的光标、补全、诊断坐标，并更新相应断言。
- `src/input/editor/indent.rs`：Tab 在当前编辑器内接受补全候选或执行缩进；多行选择区末尾位于下一行行首时，不缩进未选中的下一行。
- `src/input/base/kind.rs`、`src/input/editor/lsp/overlay.rs`、`src/input/editor/mod.rs`、`src/input/editor/indent.rs`：本地 Go 模板插入纯源码并选择第一个填写处；Tab / Shift+Tab 切换填写处，编辑字段时后续位置跟随字节偏移，移出模板时恢复普通缩进。
- `src/input/base/kind.rs`、`src/input/editor/mod.rs`：提供源码外的类型提示存储与更新接口，编辑时立即清除旧提示。
- `src/input/base/element.rs`、`src/input/editor/display_map/inline_line.rs`、`text_wrapper.rs`：用零源码长度的片段为提示保留宽度，软折行使用同样的测量，鼠标落在提示上时映射到源码锚点；键盘移动和剪贴板仍操作原始文本。
- 上述显示层同时按制表位展开 Tab，并为其一个源码字节保留实际宽度，修复 Go 文件缩进显示过窄；折行续行保留前导 Tab 的缩进，源码不改为空格。按文档版本、提示、字体与制表宽度缓存测量，避免每次重绘重新测量整个文件。

应用层使用 `src/editing.rs` 生成 Unicode 差异，在后台计算，然后检查题目与文档版本后应用。后台草稿保存不触发格式化，避免把用户刚撤销的操作重做。Ctrl+Shift+Z 补充 Windows 重做绑定。

设计对照：

- [Zed 编辑事务与缓冲区编辑](https://github.com/zed-industries/zed/blob/main/crates/editor/src/editor.rs)
- [Zed 选择区集合与锚点](https://github.com/zed-industries/zed/blob/main/crates/editor/src/selections_collection.rs)
- [Zed 输入处理](https://github.com/zed-industries/zed/blob/main/crates/editor/src/input.rs)
- [Zed 默认保存设置](https://github.com/zed-industries/zed/blob/main/assets/settings/default.json)

这里按编辑事务和锚点的思路实现独立补丁，没有复制 Zed 的编辑器源码。

Go 类型来自 [gopls 的 assignVariableTypes 与 rangeVariableTypes](https://go.dev/gopls/inlayHints)，没有用正则猜测类型。
