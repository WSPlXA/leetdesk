# LeetDesk

使用 **Rust + GPUI** 构建的原生 LeetCode 练习工作区，第一版专注 **Go**。

应用把题目描述、Markdown 做法讲解、代码编辑、测试用例和复盘笔记放在同一个可调整的工作区中。

## 已实现

- 启动先连接 LeetCode 中国站或国际站，在系统 WebView 官方页面登录；支持微信授权弹窗，浏览器在本机保留会话。Windows 使用 WebView2，macOS 使用 WKWebView。
- 首次登录分页同步在线题库、官方分类和当前账号的“已通过 / 尝试过 / 未解答”状态；切题加载题面和官方 Go 模板。
- 后续启动优先使用当前账号的本地缓存，“同步题库”按钮更新题目列表、分类和解题状态，不进行定时同步。下载过的题面与模板也会缓存。
- 分类导航可收起，题目列表使用虚拟滚动；可按难度和解题状态筛选，支持全屏专注代码布局。
- 默认显示“题单 / 主题 / 我的标签”导航，支持官方热题 100，以及链表、字符串、双指针、图、树、动态规划等中文主题；题单、主题、完成状态、难度和搜索可以组合筛选。
- Windows 程序、窗口及任务栏使用 LeetDesk 图标，提供多尺寸 ICO。源图与生成说明见 [assets/icon-design.md](assets/icon-design.md)。
- 窗口不显示系统标题栏及最小化、最大化按钮；拖动应用顶部的题目名称或空白区域移动窗口，使用顶部“关闭”安全保存并退出。界面为纯黑底，Go 关键字、字符串、数字、类型、函数分别使用高对比度颜色。
- 题库搜索、分类与进度使用内存缓存，编辑重绘复用筛选结果和题面；未变化的自动保存跳过文件替换及数据库更新。性能基准见 [PERFORMANCE.md](PERFORMANCE.md)。
- 题面使用明确的暗色文字/代码配色；行内代码背景先于文字绘制，示例保留原有换行。
- HTML 段落分别缓存布局，跨段拖选、复制及调整面板宽度时保持正确文本；复制校验中文和 emoji 的字节边界。
- Go 代码编辑：Tree-sitter 高亮、行号、折叠、自动缩进、括号配对、撤销和搜索。
- 真实 `gopls` 接入：实时诊断、输入自动触发上下文补全、Tab / Enter 接受候选、悬浮类型说明、F12/Ctrl+点击跳转定义。
- Go 本地模板补全包括 `for`、`forr`、`forc`、`make`、`makemap`、`makechan`、`append`、`if`、`switch`；选择后用 Tab / Shift+Tab 切换填写位置。gopls 启动期间仍可使用本地模板。
- 变量名后显示暗灰色类型提示（如 `count: int`、`node: *ListNode`），支持短变量声明和 range 变量。提示由 gopls 推导，只参与显示和折行，不进入源码、剪贴板、格式化或提交内容；编辑后重新获取，丢弃过期响应。
- Tab 由当前编辑器处理：没有候选时插入缩进，选中多行时块缩进；Shift+Tab 反缩进，有补全候选时 Tab 接受候选。
- Go 硬 Tab 按四列制表位显示，嵌套缩进、类型提示和折行共用源码坐标；文件中仍保留 gofmt 使用的 Tab。
- 独立的 Go module 工作区；后台 `types.go` 提供 `ListNode` 和 `TreeNode`。
- `gofmt` 格式化，以及通过 gopls 整理/补充导入；格式化可撤销。
- 默认长行自动折行，`Alt+Z` 或编辑区“折行”按钮切换；在 `{}` 内回车展开并使用 Go tab 缩进。
- Go 编辑区 Ctrl+S 时运行 `gofmt`，以局部编辑保留选择区、光标和视口，一次撤销整个格式化。后台自动保存只写草稿，不重排正在输入的代码；语法不完整也会保存。笔记区 Ctrl+S 保存 Markdown 并更新预览。
- 按题号、标题和标签搜索；多标签分类、收藏、待复习、自定义标签。
- 导入 Markdown 文件或文件夹，保留原文，一道题可以关联多篇讲解。
- Markdown 标题、表格、代码块高亮和相对路径图片；链接可打开。
- “我的笔记”支持 Markdown 源码编辑、预览、可调整分栏，每题保存为独立的 `data/workspaces/<题目 ID>/notes.md`。点击“保存笔记”或 Ctrl+S 后自动渲染；后台自动保存也更新预览。切题与关闭重启会恢复笔记；“打开 MD”打开原文件。保存失败时保留草稿及上次成功保存的预览。
- 离线练习模式内置 6 道题，覆盖数组、字符串、栈、链表、树、二分和动态规划。
- 编辑 JSON 测试用例，实际编译和执行 Go，展示每个用例的预期/实际结果。
- 支持链表数组、树的层序数组、无序数组结果比较、panic 展示、超时及取消。
- 自动保存、切题恢复、恢复最后打开的题目和光标；SQLite 保存练习状态与运行快照。
- 题库 JSON 导入、解法复制和 LeetCode 页面跳转。

在线模式的“测试”执行样例，“提交”上传当前代码快照进行平台判题，两者都显示错误、耗时和输出；提交 Accepted 后更新当前题目的“已通过”及缓存。请求发送前重新验证账号，判题中避免重复提交；“停止等待”不会撤销平台已经接收的任务。首次启动需要你自行完成官方登录和验证码，程序不读取密码或会话 Cookie。

测试任务 ID 支持平台返回的带小数点格式。平台拒绝请求时保留具体错误，并区分 HTTP 401 登录失效、403 权限或验证、429 限流及服务异常。

## Windows 启动

需要 Rust **MSVC** 工具链、Visual Studio C++ Build Tools、Microsoft Edge WebView2 Runtime、Go（建议 1.22 或更新版本）和 gopls。仓库的 `rust-toolchain.toml` 使用跨平台 `stable`；Windows 命令显式选择 MSVC，避免系统默认 GNU 工具链缺少链接器。

```powershell
go install golang.org/x/tools/gopls@latest
rustup toolchain install stable-x86_64-pc-windows-msvc
cargo +stable-x86_64-pc-windows-msvc run --locked -- --doctor
cargo +stable-x86_64-pc-windows-msvc run --locked
```

也可以执行 `./start.ps1`，构建后启动桌面窗口。此脚本使用 `desktop` 构建配置，程序输出到 `target/desktop/leetdesk.exe`。更新后请先关闭旧窗口，再启动新版，避免两个窗口同时编辑同一数据目录。

Go 与 gopls 必须可运行。gopls 默认从 PATH 查找，也会检测 `%USERPROFILE%/go/bin/gopls.exe`。可以设置 `LEETDESK_GOPLS` 指定可执行文件。

默认数据目录是启动目录下的 `data/`。可用 `--data-dir PATH` 或 `LEETDESK_DATA` 指定其他位置。

点击官方登录页的微信按钮会打开授权弹窗；扫码后返回主窗口，程序自动检查登录，也可以点击“我已登录，开始同步”。关闭旧版窗口，再用 `./start.ps1` 启动新版；旧的 `target/release/leetdesk.exe` 不会被此脚本更新。

## macOS 启动

源码和依赖具备 macOS 路径：GPUI 窗口与系统 WKWebView、Go/gopls、路径和字体均按平台处理。需在 Mac 上安装 Xcode Command Line Tools、Rust stable、Go 和 gopls，然后在仓库目录运行：

```sh
xcode-select --install
go install golang.org/x/tools/gopls@latest
sh ./start.sh
```

`start.sh` 使用 Mac 本机 Rust 工具链构建并运行 `target/release/leetdesk`。Apple Silicon 与 Intel Mac 需分别在各自机器上本机构建；当前 Windows 环境已核对 macOS 依赖解析，但尚无 Mac 机器可验证编译、登录弹窗和窗口交互。窗口顶部题目名称及空白区域可拖动，关闭程序使用系统“退出”菜单或 Cmd+Q。

网络不可用时，登录页的“使用离线缓存”可以打开已同步题库。`--offline` 跳过登录并打开本地数据；新数据目录使用内置题库。缓存中尚未下载的题面需要联网登录后加载。会员题目及部分题目是否有 Go 模板由平台决定。

## 按主题练习

按主题练习：左侧选择“热题 100”或“全部题目”，再选择“链表 / 字符串 / 双指针 / 图”等主题；题目列表上方继续选择“未解答 / 已通过”等状态和难度。例如“热题 100 + 链表 + 未解答 + 简单”。“重置”清除全部筛选，顶部“☰ 分类”可以收起或展开导航。

热题 100 使用[力扣官方学习计划](https://leetcode.cn/studyplan/top-100-liked/)，按官方顺序列出当前题库内的题目，并显示通过进度。内置完整题单快照；点击“同步题库”更新在线题库和题单缓存，后续启动复用缓存。更新返回空数据、不足 100 题或重复题目时保留已有题单。离线题库只显示其已有题目，完整练习需同步在线题库。同一题允许拥有多个主题；个人标签单独保存和筛选。

## 快捷键

| 操作 | 快捷键 |
| --- | --- |
| 保存 | Ctrl+S；Mac 也支持 Cmd+S |
| 切换自动折行 | Alt+Z |
| 运行用例 | Ctrl+Enter；Mac 也支持 Cmd+Enter |
| 提交解答 | Ctrl+Shift+Enter；Mac 也支持 Cmd+Shift+Enter |
| 格式化 | Ctrl+Shift+F |
| 整理/补充 import | Ctrl+Shift+O |
| 手动补全 | Ctrl+Space；Mac 也支持 Cmd+J |
| 接受补全 / 模板下一处 | Tab / Enter；模板填写处用 Tab / Shift+Tab |
| 撤销 / 重做 | Ctrl+Z / Ctrl+Shift+Z（也支持 Ctrl+Y） |
| 跳转定义 | F12 / Ctrl+点击 |
| 导入讲解 | Ctrl+I |
| 编辑器搜索 | Ctrl+F |

保留 `solution.go` 的 `package solution` 声明。复制解法时会自动移除此声明。辅助节点类型已有定义，无需重复声明。外部文件的定义跳转在只读视图打开，点击“返回解法”继续练习。

语法高亮由 Tree-sitter 负责，类型分析、符号补全和错误诊断由 gopls 负责；循环与容器模板由编辑器本地提供。编辑器不会把“语言服务连接失败”伪装成没有错误；状态栏可以重连。第一次加载 Go 标准库和第一次编译可能需要建立缓存。

当前编辑组件尚不自动应用补全项附带的 import 修改，因此不展示尚未导入包的补全；可以先输入调用，再点击“整理导入”补充 import，随后获得完整成员补全。

## 导入你的 MD

不要求修改已有文件。关联顺序是：可选元数据题号、文件名前缀题号、文档中的唯一 LeetCode 题目链接。无法关联的文档会关联到点击导入时选中的题目。

可选格式如下（目前支持单行 tags 列表）：

```markdown
---
problem_id: 1
tags: [哈希表, 面试复习]
---

# 做法讲解

## 思路
……

## 复杂度
……
```

示例见 [examples/1-two-sum.md](examples/1-two-sum.md)。原文件不会被改写。同名文档导入时生成新的副本。相对图片/链接从原文档目录解析，因此使用相关图片时需要保留原目录。

当前 MD 作为已有题目的讲解导入。首次登录下载题库，后续启动读取当前账号缓存，通过“同步题库”更新；独立本地题库可导入 JSON。题号元数据引用尚未存在的题目时会明确报错。

## 扩展本地题库

点击“导入题目 JSON”，或：

```powershell
cargo run --locked -- --import-catalog ./my-problems.json
```

格式参照 `assets/problems.json`，支持单题对象或数组。

- `id` / `slug` / `title` / `difficulty` / `tags`：题目元数据。
- `description` / `explanation`：Markdown 文本。
- `starter`：包含 `package solution` 的 Go 模板。
- `function`：被测试的 Go 函数名。
- `parameters`：按函数参数顺序提供 `name`、`kind`。
- `returns`：返回值类型。
- `cases`：`input` 是参数数组，`expected` 是预期返回值。
- `unordered`：忽略返回数组的元素顺序，重复元素仍会保留。

目前支持的类型是 `int`、`ints`、`string`、`bool`、`list`、`tree`。设计类、多返回值和特殊判题规则尚未支持。

例如两数之和：

```json
[{ "input": [[2, 7, 11, 15], 9], "expected": [0, 1] }]
```

树使用 `[3,9,20,null,null,15,7]` 这样的紧凑层序表示；链表使用 `[1,2,3]`。

本地测试运行当前代码的副本；修改编辑区不会改变正在执行的快照。编译最多等待 90 秒，测试执行限时 5 秒。输出保留最多约 1 MiB/流。运行器是本机子进程，不是安全沙箱。

## 验证

```powershell
cargo +stable-x86_64-pc-windows-msvc check --locked
cargo +stable-x86_64-pc-windows-msvc test --lib --locked
cargo +stable-x86_64-pc-windows-msvc test --bin leetdesk --locked
cargo +stable-x86_64-pc-windows-msvc test --no-default-features --test go_integration --locked -- --ignored --test-threads=1
cargo +stable-x86_64-pc-windows-msvc clippy --all-targets --locked -- -D warnings
cargo +stable-x86_64-pc-windows-msvc build --profile desktop --locked
./target/desktop/leetdesk.exe --smoke-test --data-dir ./target/native-check
./target/desktop/leetdesk.exe --online-check --data-dir ./target/online-check
./target/desktop/leetdesk.exe --study-plan-check --data-dir ./target/topic-probe
```

Go 集成测试会真正启动 Go 编译器和 gopls，检查标准库补全、节点类型、诊断更新、自动导入，以及正确答案、错误答案、编译错误、panic 和超时。

`--smoke-test` 使用隐藏原生窗口检查启动、Go 语法解析器和 gopls 连接，完成后自动退出。验证记录见 `VERIFICATION.md`。

`--online-check` 使用独立浏览器目录检查公共接口、完整分页、官方 Go 模板和原生授权弹窗，不要求私人账号登录，也不提交解答。扫码授权及私人账号的同步结果需要在实际登录后检查。

`--study-plan-check` 检查官方热题 100 的分组与题目标识，缓存到指定的独立数据目录。中国站与国际站均已实际下载验证，各返回 100 道题。

## 代码结构

```text
src/desktop.rs   GPUI 工作区与编辑器语言服务适配
src/auth.rs      原生 WebView2 登录、账号检查和同步调度
src/online.rs    LeetCode 请求、官方题面/模板与状态解析
src/language.rs  stdio JSON-RPC、文档版本同步、诊断和 UTF-16 坐标
src/catalog.rs   题目模型、用例校验、MD 元数据及题目关联
src/store.rs     SQLite、代码文件、讲解导入和运行记录
src/runner.rs    Go 测试入口生成、编译执行、超时与取消
src/editing.rs   Unicode 差异与局部编辑
vendor/gpui-base 编辑事务、UTF-16 坐标补丁，见 vendor/PATCHES.md
assets/         内置练习题与 Go 测试入口模板
```
