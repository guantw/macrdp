# macrdp 项目指南

## 项目结构

- 产品是 macOS 14+ 原生 RDP 服务端。Rust 负责采集、编码、协议和输入；GUI 使用 Tauri 2、React、TypeScript 和 Vite。
- 根 `Cargo.toml` 管理 CLI 和共享 crates：`macrdp-server` 是 CLI 入口，`macrdp-core` 负责服务生命周期及各模块集成，`macrdp-capture`、`macrdp-encode`、`macrdp-input` 分别负责采集、编码和输入。
- `macrdp-ui/src-tauri/Cargo.toml` 是独立 Rust workspace。根目录的 workspace 构建或测试不包含 GUI；GUI Rust 变更需要使用它自己的 manifest。
- GUI 在进程内调用 `macrdp-core`，不是启动 CLI sidecar。当前 `tauri.conf.json` 和 `build.rs` 没有调用 `macrdp-ui/build-cli.sh`。
- 两个 workspace 都通过 `[patch.crates-io]` 引用本仓库的 `crates/ironrdp-server-gfx` 和 `crates/ironrdp-acceptor-patched`。协议修改应检查这些实际生效的目录；修改 patch 路径或版本时核对两个 manifest。
- `crates/ironrdp-server-gfx` 的 Cargo package 名为 `ironrdp-server`，不是目录名。

## 工具链与构建入口

- 使用 Rust stable、Apple Command Line Tools 或完整 Xcode，以及 Node.js/npm。采集依赖会编译 Swift 代码；具体环境要求见 `README.md` 和 `README_EN.md`。
- `.cargo/config.toml` 设置 `MACOSX_DEPLOYMENT_TARGET=14.0`。不要用指向旧 Swift 运行库的 `DYLD_LIBRARY_PATH` 处理链接问题，它会影响编译器及构建子进程；Swift 链接库搜索问题应核对当前工具链的库路径，必要时通过单次命令的 `LIBRARY_PATH` 指定。
- 前端依赖位于 `macrdp-ui/`，使用 npm 和现有 `package-lock.json`；CI 安装入口是 `npm ci`。
- Cargo/Tauri 的产物位置受 `CARGO_TARGET_DIR` 控制。指定该变量后，按实际目标目录查找产物，不沿用包装脚本中写死的 `target/` 路径。

以下命令从仓库根目录执行，表中注明切换目录的除外：

| 范围 | 入口 |
| --- | --- |
| CLI 构建 | `cargo build -p macrdp-server` |
| 前端类型检查及打包 | `npm --prefix macrdp-ui run build` |
| GUI Rust 构建 | `cargo build --manifest-path macrdp-ui/src-tauri/Cargo.toml` |
| GUI 应用打包 | 在 `macrdp-ui/` 执行 `npm run tauri -- build --bundles app` |
| GUI DMG 打包 | 在 `macrdp-ui/` 执行 `npm run tauri -- build --bundles dmg` |

Tauri 打包会通过 `beforeBuildCommand` 自动执行前端构建。

### 现有脚本的限制

- GUI manifest 的 `[profile.release]` 当前是 `opt-level = 0`。性能验证不能仅凭 `release` 名称认定已优化；优化构建需显式使用 `CARGO_PROFILE_RELEASE_OPT_LEVEL=3`。Makefile 的 `MACRDP_CLI_PROFILE` 不会改变 GUI 的优化等级。
- `make check` 把输出传给 `tail -1`，会掩盖前序构建命令的失败状态，最后的“全部通过”不能作为验证依据。需要使用上表中的实际命令及其退出状态。
- `macrdp-ui/build.sh` 会隐藏部分错误并尝试回退，不能以脚本结束或已有 `.app` 存在判断本次打包成功。
- Makefile、`build-cli.sh` 和 npm 的 `tauri:build`/`tauri:install` 脚本仍包含固定的产物路径；外置 Cargo 目标目录时不能直接套用这些路径。`tauri:install` 还会复制应用到 `/Applications`，不是单纯的构建检查。
- `.gitignore` 当前忽略 Cargo lockfiles。`--locked`/`--offline` 仅适用于已经有对应 lockfile 和依赖缓存的环境，不是全新 checkout 的通用构建参数。

## 测试与验证位置

按涉及的模块选择现有测试入口：

| 范围 | 入口 |
| --- | --- |
| 共享服务、配置、坐标映射和自适应控制 | `cargo test -p macrdp-core --lib` |
| RDP 协议、鉴权、封包和发送流水线 | `cargo test -p ironrdp-server --lib` |
| 采集或输入 | `cargo test -p macrdp-capture --lib` / `cargo test -p macrdp-input --lib` |
| 编码和像素转换 | `cargo test -p macrdp-encode --lib` |
| GUI Rust、托盘布局和配置持久化 | `cargo test --manifest-path macrdp-ui/src-tauri/Cargo.toml --lib` |

- 协议连接回归位于 `crates/ironrdp-server-gfx/src/connection_tests.rs`，通过本地回环连接覆盖鉴权、画面、输入和重连。
- `macrdp-encode` 的 `videotoolbox` 测试会建立真实硬件编码会话；它们不是纯算法测试。采集颜色配置测试会调用原生 API，但不开始屏幕采集；键盘事件单元测试只构造和检查事件，不注入输入。
- 前端目前没有 `npm test` 脚本；`npm run build` 执行 TypeScript 检查和 Vite 打包，不等同于界面交互测试。
- 原生屏幕采集、远程输入、Dock/菜单栏及窗口恢复行为，需要运行应用验证；真实采集和输入分别依赖屏幕录制与辅助功能权限。

## 运行行为与跨层接口

- 应用常驻菜单栏。`macrdp-ui/src-tauri/src/lib.rs` 在 `build()` 后、`run()` 前设置 `ActivationPolicy::Accessory`；仅有 `Info.plist` 的 `LSUIElement` 不足以覆盖窗口框架的默认策略。
- 主窗口关闭时只隐藏，服务继续运行；显式停止或退出才停止服务。菜单栏与重复启动共用 `show_main_window`，恢复顺序为显示、取消最小化、聚焦。
- 主窗口和托盘弹窗共享服务状态。相关改动同时检查 `macrdp-ui/src-tauri/src/` 下的 `commands.rs`、`event_bridge.rs`、`state.rs`，以及 `macrdp-ui/src/hooks/useServerStatus.ts`；启动/停止在后端串行执行，前端以事件为主、轮询为补充，旧轮询结果不能覆盖新状态。
- Tauri 命令与事件的数据结构对应 `crates/macrdp-core/src/callbacks.rs`、GUI Rust 实现及 `macrdp-ui/src/lib/` 下的 `types.ts`、`ipc.ts`。新增前端 Tauri 能力时检查 `macrdp-ui/src-tauri/capabilities/`，不要只修改生成的 schema 文件。
- 显示和输入涉及 RDP 像素、采集内容区域与 macOS 逻辑坐标。`crates/macrdp-core/src/display.rs` 将同一采集帧的内容区域和逻辑尺寸传给同目录 `handler.rs` 的坐标映射，Retina 缩放及黑边不能仅按桌面宽高推算。
- 编码准备与发送通过容量为 1 的队列重叠执行，实现在 `crates/ironrdp-server-gfx/src/server.rs`。调整流水线时保留帧顺序、背压和 resize/断开时的结束处理。
- 实际 FPS 按完成的画面写入及真实采样时长计算；流量按当前封包实际携带的载荷计算。入口在 `crates/ironrdp-server-gfx/src/gfx.rs` 和 `crates/macrdp-core/src/server.rs`，不能用配置帧率或被下一帧预取覆盖的共享大小替代。

## 配置与打包

- CLI 配置依次查找 `./config.toml`、`~/.config/macrdp/config.toml` 和 `~/Library/Application Support/macrdp/config.toml`；没有现有配置时，生成文件默认落在用户配置目录。TLS 文件默认位于所选配置目录的 `tls/`。
- GUI 配置独立存放在 `~/.macrdp/config-ui.toml`，由 `ui_config.rs` 转成服务配置；不要假定 GUI 会读取 CLI 的 `config.toml`。运行配置和 TLS 文件可能包含凭据，不属于测试夹具。
- 应用标识为 `com.macrdp.app`。`tauri.conf.json` 默认采用 ad-hoc 签名，而部分包装脚本引用本地签名身份；签名方式以实际构建入口为准，不能假定每个环境都有该身份。
- DMG 验证包括应用签名校验（`codesign --verify --deep --strict`）、镜像校验（`hdiutil verify`）、只读挂载后对比包内应用与本次签名产物，并核对版本、架构和 `LSUIElement`。
