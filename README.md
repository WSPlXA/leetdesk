# LeetDesk

**用 Rust + GPUI 构建的原生 LeetCode 刷题工作区，目前专注 Go。** 在一个窗口里读题、按主题找题、写代码、运行用例、提交解答，并用 Markdown 记录做法与复盘。

支持 Windows x64、Intel Mac 和 Apple Silicon Mac。下载已发布的 [v0.1.0 压缩包](https://github.com/WSPlXA/leetdesk/releases/tag/v0.1.0)。

## 安装与首次使用

1. 下载与你的设备对应的压缩包并解压：

   | 系统 | 文件 | 启动方式 |
   | --- | --- | --- |
   | Windows x64 | `leetdesk-windows-x86_64.zip` | 运行 `leetdesk.exe` |
   | Intel Mac | `leetdesk-macos-x86_64.zip` | 打开 `LeetDesk.app` |
   | Apple Silicon Mac | `leetdesk-macos-aarch64.zip` | 打开 `LeetDesk.app` |

2. 安装 [Go](https://go.dev/dl/) 和 `gopls`，以便编译、格式化、诊断和补全 Go 代码：

   ```sh
   go install golang.org/x/tools/gopls@latest
   ```

   确保 `go`、`gofmt`、`gopls` 可以从终端运行。若 `gopls` 不在 `PATH` 中，可以用 `LEETDESK_GOPLS` 指向它。Windows 登录窗口还需要 WebView2 Runtime。

3. 启动应用，在 LeetCode 中国站或国际站的官方页面登录。登录后同步题库、分类和当前账号的解题状态；后续启动先读取本地缓存，需要更新时点击 **同步题库**。登录暂时不可用时，可选择使用离线缓存。

4. 从左侧选择题单或主题，再按状态、难度和关键词筛选。打开题目后，右侧直接编辑 Go 解法；**运行用例**检查当前代码，**提交**交给 LeetCode 判题。

macOS 包已在对应架构的 GitHub Actions runner 上构建。当前使用临时签名，尚未进行 Apple 公证；首次打开如被系统拦截，请在 macOS 的安全设置中允许。Windows 下载包包含应用图标；窗口顶部的题目名称或空白区域可以拖动窗口。

## 刷题工作流

- **找题：** 支持热题 100、链表、字符串、双指针、图、树、动态规划等题单与主题。搜索、主题、难度、解题状态可以组合筛选；收藏、待复习和自定义标签用于安排下一轮练习。
- **读题：** 在线加载题面与官方 Go 模板，下载后缓存。题面、做法讲解、个人笔记和代码可在同一工作区查看，面板宽度可调整。
- **写代码：** Tree-sitter 语法高亮；`gopls` 提供诊断、补全、类型提示与跳转定义。内置 `for`、`make`、`append` 等模板补全；支持自动缩进、括号配对、长行折行、撤销及搜索。`Ctrl+S`（Mac 为 `Cmd+S`）保存并用 `gofmt` 格式化 Go 代码。
- **验证：** 本地用例会实际编译并运行 Go，逐例显示预期值、实际值、错误和耗时。在线题目还可调用平台测试与提交接口；通过后更新当前题目的状态缓存。正在判题时停止等待不会撤销平台已接收的任务。
- **复盘：** 每道题有独立的 Markdown 笔记，保存后更新预览；也可导入已有的 MD 做法讲解。代码、笔记与进度会自动保存，重新打开继续上次练习。

LeetDesk 通过系统 WebView 展示官方登录页；密码和验证码由你在该页面输入。账号数据与题目缓存在本机保存。会员题目及 Go 模板是否可用取决于平台。

## 常用快捷键

| 操作 | Windows | macOS |
| --- | --- | --- |
| 保存代码并格式化 / 保存笔记 | `Ctrl+S` | `Cmd+S` |
| 运行用例 | `Ctrl+Enter` | `Cmd+Enter` |
| 提交解答 | `Ctrl+Shift+Enter` | `Cmd+Shift+Enter` |
| 格式化 Go 代码 | `Ctrl+Shift+F` | `Cmd+Shift+F` |
| 整理 Go 导入 | `Ctrl+Shift+O` | `Cmd+Shift+O` |
| 手动补全 | `Ctrl+Space` | `Cmd+J` |
| 导入 Markdown | `Ctrl+I` | `Cmd+I` |
| 接受补全 / 跳到模板下一处 | `Tab` | `Tab` |

`Alt+Z` 切换代码自动折行；`F12` 跳转定义。编辑器会根据当前补全状态处理 `Tab`：有候选时接受补全，否则用于缩进；`Shift+Tab` 可反缩进或返回上一个模板填写位置。

Go 解法保留 `package solution`。内置的 `ListNode`、`TreeNode` 类型由题目工作区提供，无需在解法中重复定义。复制解法时会移除 `package solution` 声明。

## Markdown 讲解与笔记

点击 **导入 MD** 可选择文件或文件夹；原文件不会被改写。关联题目的顺序是：文档元数据中的 `problem_id`、文件名前缀题号、文档中唯一的 LeetCode 题目链接。都没有时，关联到导入时选中的题目。

```markdown
---
problem_id: 1
tags: [哈希表, 面试复习]
---

# 两数之和

## 思路
用哈希表记录已经看过的数。

## 复杂度
时间 O(n)，空间 O(n)。
```

更多示例见 [examples/1-two-sum.md](examples/1-two-sum.md)。讲解支持标题、表格、代码块及相对路径图片；使用图片时请保留其原目录。**我的笔记** 按题保存为 `workspaces/<题目 ID>/notes.md`，点击保存或按 `Ctrl+S` / `Cmd+S` 后自动渲染，也可以点击 **打开 MD** 在外部编辑。

## 数据与离线模式

Windows 默认把数据保存在**启动时的工作目录**下的 `data/`；macOS 默认位置是 `~/Library/Application Support/LeetDesk/`。需要固定或迁移数据目录时，启动时传入 `--data-dir PATH`，或设置 `LEETDESK_DATA`。

```sh
leetdesk --data-dir /path/to/my-leetdesk-data
leetdesk --offline --data-dir /path/to/my-leetdesk-data
leetdesk --doctor
```

`--offline` 跳过登录并打开已缓存的题库；新数据目录会显示内置练习题。尚未下载的在线题面需要重新联网加载。`--doctor` 检查本机 Go、gopls 和 gofmt。缓存中的题目、已通过状态和题单通过 **同步题库** 手动更新，不会在后台定时刷新。

除在线题库外，还可点击 **导入题目 JSON** 添加本地练习题。格式参见 [assets/problems.json](assets/problems.json)，支持单题对象或数组；也可使用 `leetdesk --import-catalog PATH`。内置练习题和本地 JSON 题目的用例由本机 Go 子进程执行，**不提供安全沙箱**，只运行可信代码。

## 从源码运行

需要 Rust stable。Windows 还需要 MSVC 工具链和 Visual Studio C++ Build Tools；macOS 需要 Xcode Command Line Tools。Go 和 gopls 的安装方法见上文。

**Windows（PowerShell）：**

```powershell
rustup toolchain install stable-x86_64-pc-windows-msvc
./start.ps1
```

`start.ps1` 构建到 `target/desktop/leetdesk.exe` 并启动应用。关闭旧窗口后再启动新构建，避免两个实例同时编辑同一数据目录。

**macOS：**

```sh
xcode-select --install
sh ./start.sh
```

`start.sh` 使用本机 Rust 工具链构建并运行 `target/release/leetdesk`。

在 macOS 或已配置默认 MSVC 工具链的 Windows 上运行检查：

```sh
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
```

Windows 也可显式指定 `+stable-x86_64-pc-windows-msvc`。真实 Go/gopls 集成测试需额外运行 `cargo test --no-default-features --test go_integration --locked -- --ignored --test-threads=1`。平台接口检查及性能记录见 [VERIFICATION.md](VERIFICATION.md) 和 [PERFORMANCE.md](PERFORMANCE.md)。

## 发布

[GitHub Actions 工作流](.github/workflows/release.yml) 在推送到 `master`、发起 PR 或手动运行时构建 Windows x64、Intel Mac 和 Apple Silicon Mac 包；推送与 `Cargo.toml` 版本一致的 `v*` 标签后，三个构建都成功才会创建 Release 并上传压缩包。Windows 构建还会运行桌面端测试。

项目使用 [MIT License](LICENSE)。
