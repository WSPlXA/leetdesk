# LeetDesk

**用 Rust + GPUI 构建的原生 LeetCode 刷题工作区，支持 Go 和 Java。** 在一个窗口里读题、按主题找题、写代码、运行用例、提交解答，并用 Markdown 记录做法与复盘。可选的单人自托管服务能备份笔记、解法和每次提交的代码快照。

支持 Windows x64、Intel Mac 和 Apple Silicon Mac。下载[最新发布版](https://github.com/WSPlXA/leetdesk/releases/latest)。

## 安装与首次使用

1. 下载与你的设备对应的压缩包并解压：

   | 系统 | 文件 | 启动方式 |
   | --- | --- | --- |
   | Windows x64 | `leetdesk-windows-x86_64.zip` | 运行 `leetdesk.exe` |
   | Intel Mac | `leetdesk-macos-x86_64.zip` | 打开 `LeetDesk.app` |
   | Apple Silicon Mac | `leetdesk-macos-aarch64.zip` | 打开 `LeetDesk.app` |

2. 使用 Go 时安装 [Go](https://go.dev/dl/) 和 `gopls`，以便编译、格式化、诊断和补全 Go 代码：

   ```sh
   go install golang.org/x/tools/gopls@latest
   ```

   确保 `go`、`gofmt`、`gopls` 可以从终端运行。若它们不在 `PATH` 中，可用 `LEETDESK_GOPLS` 和 `LEETDESK_GOFMT` 指向对应程序。使用 Java 本地用例时安装 JDK 17 或更新版本，并让 `java`、`javac` 位于 `PATH`。Java 格式化需要安装 `google-java-format`，或设置 `LEETDESK_JAVA_FORMAT` 指向该命令。Windows 登录窗口还需要 WebView2 Runtime。

3. 启动应用，在 LeetCode 中国站或国际站的官方页面登录。选择国际站后自动使用英文界面、英文题名和英文题面，中国站使用中文；站点偏好在重启后恢复。两个站点分别缓存题库、题面和账号通过状态，普通题目按题号和 slug 对应。登录后同步题库、分类和当前账号的解题状态；后续启动先读取本地缓存，需要更新时点击 **同步题库**。登录暂时不可用时，可选择使用离线缓存。

4. 从左侧选择题单或主题，再按状态、难度和关键词筛选。打开题目后，在编辑器上方切换 Go / Java；两个语言的解答分别保存。**运行用例**检查当前代码，**提交**交给 LeetCode 判题。

macOS 包已在对应架构的 GitHub Actions runner 上构建。当前使用临时签名，尚未进行 Apple 公证；首次打开如被系统拦截，请在 macOS 的安全设置中允许。Windows 下载包包含应用图标；窗口顶部的题目名称或空白区域可以拖动窗口。

点击工作区右上角的 **小齿轮**，或按 `Ctrl+,`（Mac 为 `Cmd+,`），打开设置标签：

- **界面语言：** 跟随站点 / 简体中文 / English。手动选择优先于站点自动切换；题面和个人内容保留原文。
- **代码字体：** 在本机已安装的常用等宽字体中选择，例如 Consolas、Courier New、Cascadia Code、Menlo、JetBrains Mono；也可恢复平台默认字体。字体缺失时使用默认等宽字体。
- **编辑器颜色主题：** 高对比红色（默认）、琥珀色、单色。设置页即时预览字体与配色，关键字加粗、注释斜体。
- **外观和排版：** 黑色 / 白色 / 米色界面、12–24 px 代码字号、自动折行。编辑器配色随界面明暗调整。

以上偏好自动保存并在重启后恢复。登录页也可切换界面明暗主题，题面、笔记、表格和数学公式同步调整颜色。

布局参考 [Zed 的项目面板与编辑区](https://zed.dev/docs/project-panel)：紧凑顶部操作栏、左侧题库、带活动标记的文件标签、底部测试面板和状态栏。分类面板默认展开，点击 **分类** 可收起；设置在编辑区独立标签中打开，关闭后继续原来的代码和选区。

左侧 **按解法** 提供 12 个大类、69 个具体解法分组，首批编排 166 道典型题，覆盖热题 100。点击大类或具体做法打开统一的 **学习讲解** 页：大类原理、判断路线与复杂度之后，接着阅读当前题目的解题步骤、依据和易错点，以及练习顺序。切题保留讲解页并更新内容，普通打开时自动关联当前题目的分类；若正在查看别的分组，会明确标出当前题目不属于该组。

166 道编排题均有原创详细讲解，离线可读：从大类方法映射到本题的状态、决策或数据结构，解释解题步骤与正确性，给出包含标准库导入的 Go 参考代码，并用具体输入逐步推演关键变量，最后说明复杂度与易错点。说明随中英文界面切换，示例中的变量名与代码一致。代码使用 Go 1.21+，节点结构单独说明由平台提供；查看参考代码不改动编辑器草稿。接雨水、省份数量、股票和最长有效括号还按所选做法切换完整的另一套讲解、代码和推演，共 170 套。

导入的 Markdown 作为同页补充资料，保留原文和图片相对路径，个人笔记仍可单独编辑。尚未编写解析的其他题目显示明确提示并保留已有资料。图搜索说明参考 [labuladong 的图遍历教程](https://labuladong.online/zh/algo/data-structure-basic/graph-traverse-basic/)，包括授权登录后可读的路径枚举与 BFS 部分，说明页保留参考链接。可运行 `python scripts/check_lesson_go.py`，对页面中同一份 Go 代码逐题编译和运行样例/边界检查；仅使用标准库，不访问网络。

分组支持同一题的多种解法，例如接雨水可在双指针和单调栈中练习。计数表示当前题库的 **平台 AC / 已载入**，与全部题目或热题 100 的题单范围一致；难度、状态和搜索继续与解法筛选取交集。指南另显示编排题目中已经载入的数量，未下载的题目可先同步题库。未整理的题目保留在 **未归类题目**，不根据宽泛标签猜测做法；**按主题** 仍可使用原有官方主题和个人标签。

## 刷题工作流

- **找题：** 支持热题 100、链表、字符串、双指针、图、树、动态规划等题单与主题。搜索、主题、难度、解题状态可以组合筛选；收藏、待复习和自定义标签用于安排下一轮练习。
- **读题：** 在线加载题面与官方 Go / Java 模板，下载后缓存。题面、做法讲解、个人笔记和代码可在同一工作区查看，面板宽度可调整。
- **写代码：** Tree-sitter 语法高亮随主题使用高对比度红色系，关键字加粗、注释使用灰色斜体，并提高行号、边框和选区对比度。`gopls` 提供诊断、补全、类型提示与跳转定义。内置 `for`、`make`、`append` 等模板补全；输入 `q = app` 可补成 `append(q, value)`，直接选中待填写的元素。`var xxx func(int)` 提供 `xxx(…)` 调用候选和参数填写位置，同时保留函数变量候选以便传递回调；`Tab` 依次切换参数。支持自动缩进、括号配对、长行折行、撤销及搜索。`Ctrl+S`（Mac 为 `Cmd+S`）保存并用 `gofmt` 格式化 Go 代码。
- **代码工具：** 编辑器上方始终显示 **恢复初始模板**，按当前语言恢复模板，并先将原代码备份为 `solution-before-reset-*.go` 或 `.java`。Go 的 **整理导入** 使用 `gopls imports`；**格式化** 对 Go 使用 `gofmt`，对 Java 使用可选的 `google-java-format`。格式化失败时，原草稿仍会保存，底部状态栏显示具体错误。
- **验证：** 本地用例会实际编译并运行当前语言，逐例显示预期值、实际值和错误。运行结果中保留本次测试输入的只读快照；运行时可切回测试用例页查看或编辑下一次输入，结果返回不会切走当前页面。在线题目按当前语言调用平台测试与提交接口；通过后更新当前题目的状态缓存。正在判题时停止等待不会撤销平台已接收的任务。
- **复盘：** 每道题有独立的 Markdown 笔记，保存后更新预览；也可导入已有的 MD 做法讲解。代码、笔记与进度会自动保存，重新打开继续上次练习。

LeetDesk 通过系统 WebView 展示官方登录页；密码和验证码由你在该页面输入。账号数据与题目缓存在本机保存。会员题目及语言模板是否可用取决于平台。Java 当前提供语法高亮和内置模板补全；语义诊断、类型提示和跳转定义仍由 Go 的 `gopls` 提供，Java 暂无对应语言服务。

## 常用快捷键

| 操作 | Windows | macOS |
| --- | --- | --- |
| 打开 / 关闭设置 | `Ctrl+,` | `Cmd+,` |
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

在齿轮设置中点击 **导入 MD 文件** 选择一篇或多篇 `.md` / `.markdown`；点击 **导入文件夹** 递归导入其中的讲解。原文件不会被改写。关联题目的顺序是：文档元数据中的 `problem_id`、文件名前缀题号、文档中唯一的 LeetCode 题目链接。都没有时，关联到导入时选中的题目。

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

更多示例见 [examples/1-two-sum.md](examples/1-two-sum.md)。讲解支持标题、表格、保留缩进的代码块、相对路径图片，以及 `$O(n)$` 行内公式和独占一段的 `$$…$$` 公式；使用图片时请保留其原目录。表格有清晰的表头和分隔线，列较宽时可在预览内横向滚动。**我的笔记** 按题保存为 `workspaces/<题目 ID>/notes.md`，点击保存或按 `Ctrl+S` / `Cmd+S` 后自动渲染，也可以点击 **打开 MD** 在外部编辑。

需要在另一台电脑继续写笔记时，打开 **我的笔记**，点击 **导出全部笔记** 保存一个 ZIP 文件。它包含所有题目的个人笔记、导入过的做法讲解，以及文档引用的相对路径本地图片；找不到的图片会在导出后提示。把 ZIP 带到另一台电脑后，在同一位置点击 **导入笔记包**。重复导入相同内容会跳过；若同一题的个人笔记在两台电脑都已修改，导入内容会追加到现有笔记，并单独保留一份 `notes-imported-*.md`。笔记包只承载笔记，不包含 LeetCode 登录信息或代码。

## 单人自托管备份

LeetDesk 可以把个人 Markdown 笔记、导入的讲解与图片、Go / Java 解法，以及每次在线提交的源代码快照保存到自己的服务器。服务端每次上传都保留一个独立 ZIP；取回操作使用最新一份。桌面端仍以本地文件为工作副本，离线时可以继续编辑。备份不包括 LeetCode 登录凭据、题库缓存、用例运行产物或本地运行历史。

在服务器上生成至少 32 字符的随机令牌，例如 `openssl rand -hex 32`。将其作为 `LEETDESK_SERVER_TOKEN` 注入进程环境，备份目录设为 `LEETDESK_SERVER_DATA`。默认只监听 `127.0.0.1:8787`；从外网访问时请用 HTTPS 反向代理或 VPN，不要直接暴露 HTTP 端口。服务只实现单人令牌认证，没有注册、找回密码或多用户管理。

**直接拉镜像：** 到仓库的 **Actions → Publish server image → Run workflow**，输入已存在的版本标签（当前为 `v0.2.0`）。工作流从该标签构建并测试 `linux/amd64` 镜像，通过后推送到 `ghcr.io/wsplxa/leetdesk-server:v0.2.0`。推送代码或标签不会自动运行这个工作流。首次发布的 GHCR 包默认是私有的；如果要在服务器上免登录拉取，须在 GitHub 的包页面将可见性改为 Public。这个改动不可撤销。不想公开镜像时，可用有 `read:packages` 权限的 GitHub 令牌在服务器执行 `docker login ghcr.io`。镜像只包含服务程序，不含个人备份数据。

以下命令在 Linux 服务器上运行；将令牌文件中的值复制给桌面端的 `LEETDESK_SYNC_TOKEN`：

```sh
sudo install -d -m 700 /etc/leetdesk /srv/leetdesk
sudo chown 10001:10001 /srv/leetdesk
printf 'LEETDESK_SERVER_TOKEN=%s\n' "$(openssl rand -hex 32)" | sudo tee /etc/leetdesk/server.env >/dev/null
sudo chmod 600 /etc/leetdesk/server.env
sudo docker pull ghcr.io/wsplxa/leetdesk-server:v0.2.0
sudo docker run -d --name leetdesk-server --restart unless-stopped \
  --env-file /etc/leetdesk/server.env \
  -v /srv/leetdesk:/data \
  -p 127.0.0.1:8787:8787 \
  ghcr.io/wsplxa/leetdesk-server:v0.2.0
```

旧版本升级时，先取新标签的镜像，再停止并删除旧容器，用相同的令牌文件、数据目录和端口重新运行。不要删除 `/srv/leetdesk`；备份历史保存在那里。

**从源码构建镜像或二进制：** 也可以使用 [Dockerfile.server](Dockerfile.server) 自行构建，或直接构建 Rust 服务端：

```sh
cargo build --locked --release --no-default-features --features server --bin leetdesk-server
export LEETDESK_SERVER_TOKEN="$(openssl rand -hex 32)"
export LEETDESK_SERVER_DATA='/srv/leetdesk'
./target/release/leetdesk-server
```

容器内服务监听 `0.0.0.0:8787`，推荐只映射到宿主机回环地址 `127.0.0.1:8787:8787`，再由 HTTPS 反向代理转发。请定期备份服务端数据目录，它包含所有历史快照。

在桌面端设置同一个令牌和服务地址；Windows 可以在启动 LeetDesk 前设置用户环境变量，macOS 可以从终端带入环境变量启动。远程地址必须是 HTTPS；本机 `localhost` / `127.0.0.1` 可以使用 HTTP。然后进入 **我的笔记** 点击 **上传备份**，另一台电脑点击 **取回备份**。导入时相同文件会跳过；不同的解法或提交快照另存为带 `-imported-` 的副本，不覆盖本地文件。个人笔记发生冲突时会合并正文并保存原始导入副本。解法冲突文件位于对应题目的 `workspaces/<题号>/` 下，选择要使用的版本后可手动替换当前解法。

```sh
export LEETDESK_SYNC_URL='https://你的域名'
export LEETDESK_SYNC_TOKEN='同一个随机令牌'
```

Windows PowerShell 启动前可设置 `$env:LEETDESK_SYNC_URL='https://你的域名'` 和 `$env:LEETDESK_SYNC_TOKEN='服务器令牌'`，再运行 `leetdesk.exe`。令牌需在服务器和桌面端保持一致。

令牌只从环境变量读取，不写入备份包。遗失或泄露时，在服务端更换令牌并同步更新所有桌面端。单人备份不会自动同步；点击按钮后应等待状态栏显示成功，再在另一台设备取回。两台设备同时修改同一道题时请检查保留的冲突副本。

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

[GitHub Actions 工作流](.github/workflows/release.yml) 只在手动运行时构建 Windows x64、Intel Mac 和 Apple Silicon Mac 包，不会因推送提交或标签自动启动。到仓库的 **Actions → Build and release → Run workflow**，留空 `release_tag` 只生成 Actions 产物；填入已推送、与 `Cargo.toml` 版本一致的标签（如 `v0.2.0`）则在三个构建都成功后创建 Release 并上传压缩包。Windows 构建还会运行桌面端测试。[服务端镜像工作流](.github/workflows/server-image.yml) 也只接受手动触发，先测试再推送 GHCR。

项目使用 [MIT License](LICENSE)。
