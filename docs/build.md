# 构建与打包

## 环境

使用 Linux 或 WSL 的 Linux 原生目录。需要 Git、Rustup、ARM GNU 交叉编译器、MinGW-w64、Zip。不需要 Go 工具链。

```sh
sudo apt-get update
sudo apt-get install gcc-arm-linux-gnueabihf gcc-mingw-w64-x86-64-posix zip
git clone https://github.com/tcpqueue/quectel-rgmii-toolkit-Rust.git
cd quectel-rgmii-toolkit-Rust
rustup toolchain install 1.98.1 --profile minimal --component rustfmt,clippy
rustup target add --toolchain 1.98.1 armv7-unknown-linux-musleabihf x86_64-pc-windows-gnu
```

`.cargo/config.toml` 已配置交叉链接器。ARM 产物为 ARMv7、EABI5、musl 静态链接，运行时不依赖设备上的 Rust、Go 或动态 libc。

## 编译

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
bash scripts/build.sh
```

构建脚本运行 Rust 测试，生成两种可执行文件并更新 `SHA256SUMS`：

| 文件 | 用途 |
| --- | --- |
| `development/simpleadmin/simpleadmin-httpd.armv7` | 设备安装程序 |
| `windows-test/bin/simpleadmin-httpd.exe` | Windows 模拟预览 |

二进制文件纳入版本控制，下载源码归档也能直接安装。发布安装包不需要用户编译。

## 本地运行与验证

```sh
mkdir -p work/preview
cargo run --locked -- --mock --http 127.0.0.1:8080 \
  --static development/simpleadmin/www \
  --auth-file work/preview/simpleadmin.auth --ttl-file work/preview/ttlvalue
```

浏览器测试需要 Node.js 与 Playwright。在 Linux 原生目录安装测试依赖：

```sh
npm install --prefix work/browser playwright
work/browser/node_modules/.bin/playwright install chromium
cargo build --locked
PLAYWRIGHT_MODULE="$PWD/work/browser/node_modules/playwright" node tests/browser.cjs
PLAYWRIGHT_MODULE="$PWD/work/browser/node_modules/playwright" node tests/cell-lock-ui.cjs
node tests/traffic-ui.cjs
PLAYWRIGHT_MODULE="$PWD/work/browser/node_modules/playwright" node tests/traffic-direction-browser.cjs
node tests/tls.cjs
```

添加 `CAPTURE_DOCS=1` 可用模拟数据重新生成 README 截图。添加 `DEVICE_URL=http://127.0.0.1:18081` 则测试 ADB 转发后的真机；设备测试使用默认凭据时请先确认设备状态。`tests/device-soak.cjs` 支持 `DEVICE_USER`、`DEVICE_PASSWORD`，只运行只读 AT 查询和五分钟采集检查。

Windows 上运行 `powershell -ExecutionPolicy Bypass -File scripts/test-windows.ps1` 可检查预览程序、登录和模拟环境的密码持久化。

## 兼容性回归样本

对照版本为 Go 仓库提交 `a207171c4a62353610688e025fbcbbf92970a884`，包含已完成的 UI、密码、监测和导航修复。样本由 Go 原函数生成，而非手写 Rust 预期值。

样本已固定保存在 `tests/fixtures/`，Rust 测试直接读取 JSON，不执行 Go。迁移阶段使用的 Go 导出器和双后端设备比较脚本已移除；需要追溯样本生成过程时可查看 Git 历史。保留文件中的来源命名，以便核对历史验证记录。

## 打包

### Windows 设备助手（WebUI）

`SimpleAdmin-Setup.exe` 由 `installer/` crate 生成，与后端同属一个 Cargo workspace，用同一套 Rust 工具链在 Linux/WSL 交叉编译，不需要 .NET、Windows App SDK 或 PowerShell。双击后它在 `127.0.0.1` 的随机端口启动本地网页服务，用默认浏览器打开界面，同时显示一个小状态窗口；关闭状态窗口即退出。

```sh
cargo test --locked -p simpleadmin-installer
cargo build --locked --release --target x86_64-pc-windows-gnu -p simpleadmin-installer
```

`scripts/build.sh` 会在设备程序和 `development/SHA256SUMS` 更新后再编译安装器，并复制到仓库根目录的 `SimpleAdmin-Setup.exe`（不纳入 Git，作为 Release 附件发布）。

- **内嵌资源**：`installer/build.rs` 将 `adb.exe`、两个 ADB DLL、`LICENSE` 与整个 `development/` 打包、逐文件计算 SHA-256 后用 deflate 压缩进 EXE。首次运行解压到 `%LOCALAPPDATA%\SimpleAdmin\runtime\<资源哈希>`，此后每次启动校验后复用；新版本解压到新目录并尝试清理旧目录，正被 ADB 服务占用的旧文件会跳过。修改 `development/` 后必须重新编译安装器。
- **本地访问控制**：启动时生成 32 字节随机令牌，通过一次性链接换成 `HttpOnly; SameSite=Strict` Cookie 并从地址栏移除。所有请求要求唯一且正确的 `Host: 127.0.0.1:<端口>`（防 DNS 重绑定）；写操作还要求同源 `Origin`；浏览器标记为跨站的请求一律拒绝。页面使用严格 CSP，不加载任何外部资源。
- **状态窗口**：`installer/src/window.rs` 用 Win32 API 绘制，显示运行状态、当前操作、网页是否打开和本机端口，提供“打开网页”和“退出”。操作进行中关闭窗口会先确认；页面上的“退出设备助手”在操作中被拒绝，成功后窗口随之关闭。图标、版本信息和清单（通用控件 6、每显示器 DPI、`asInvoker`）由 `build.rs` 调用 `x86_64-w64-mingw32-windres` 嵌入，图标由 `scripts/make-icon.cjs` 生成。
- **单实例与退出**：再次双击时把已有窗口带到前台并打开页面。关闭网页不会退出程序；使用 `--no-window` 或在非 Windows 系统上运行时没有状态窗口，此时页面关闭 15 秒后、或 3 分钟没有页面访问时退出，操作进行中不会退出。
- **安装流程**：`installer/src/engine.rs` 按原 `toolkit.ps1` 的步骤直接调用 adb：只读预检（Linux、armv7l、模块特征、root、Bash、可写 /tmp）→ 上传 → 账号密码经 `adb exec-in` 标准输入写入模块 `/tmp` 并校验 SHA-256 → 安装 → 读取结果 → 通过 ADB 通道检查三个页面 → 失败时自动诊断 → 清理临时文件。报告写入 `%LOCALAPPDATA%\SimpleAdmin\Reports`，不包含密码。
- **串口准备**：`installer/src/qualcomm.rs` 使用 `serialport` crate（115200 8N1、DTR/RTS），按 VID 2C7C 标注移远端口。所有写操作先在服务端生成计划并给出一次性确认编号，执行前重新核对模块身份；联网方案还会重新读取 MPDN 规则，与预览不一致时不执行。

维护和测试参数（正式使用无需关心）：

| 参数 | 作用 |
| --- | --- |
| `--self-test` | 解压并校验资源、运行内置 `adb version` 后退出，不连接设备 |
| `--no-browser` | 不打开浏览器，只在 `installer.url` 写入访问地址 |
| `--no-window` | 不显示状态窗口，页面关闭后自动退出（非 Windows 系统始终如此） |
| `--port N` / `--data-dir DIR` | 固定端口、改用其他数据目录 |
| `--payload-dir DIR` | 使用未打包的目录（含 adb 与 `development/`），用于开发 |

`toolkit.ps1`、`toolkit.bat`、`diagnose.bat` 保留为源码包的无界面安装方式，新设备助手不再调用它们；`tests/installer-windows.ps1` 继续用模拟 ADB 验证该脚本。

#### 测试

```sh
cargo test --locked -p simpleadmin-installer
cargo build -p simpleadmin-installer
node tests/web-installer.cjs
```

Rust 测试覆盖串口逻辑（USB 字段保留、ADB 1/2 跳过、确认期间配置变化、写入拒绝、读回不一致、取消、身份变化、批量中止、联网方案、自定义指令限制）、安装流程（未连接/未授权、手机/模拟器/无 root/无 Bash/只读 /tmp 预检不写设备、上传/安装/结果/网页失败与自动诊断、端口保留与自定义、诊断与打开网页不改设备、凭据只经标准输入）、本地访问控制与资源包校验。

`tests/web-installer.cjs` 在 Linux 上启动真实安装器，用 `tests/fixtures/fake-quectel-modem.py` 创建的伪终端模拟模块 AT 口、脚本模拟 adb，以 Playwright 走完识别、解锁确认、自定义 AT、恢复出厂二次确认、联网方案预览、设备信息、安装成功与失败、输入校验、报告、打开管理页面、首次打开时的页面选择、侧边栏导航、深浅色与窄窗口布局，以及无状态窗口时页面关闭后的自动退出。设置 `SIMPLEADMIN_NO_OPEN=<文件>` 时，打开浏览器、记事本和文件夹的请求只记录到该文件。

### 离线包

```sh
bash scripts/build.sh
bash scripts/package.sh
```

`package.sh` 校验仓库中的二进制与安装文件清单后，生成 `packages/quectel-rgmii-toolkit-Rust-<版本>-offline.zip`（仅含 EXE）、单独的 `packages/SimpleAdmin-Setup.exe` 及其 SHA-256。两者作为 GitHub Release 附件发布；`packages/` 不提交到 Git。

修改前端或安装运行辅助文件后，先更新安装文件校验清单（构建脚本会自动执行）：

```sh
bash scripts/checksums.sh
node tests/installer.cjs
```

Linux 安装测试使用隔离目录和模拟系统操作，不执行真实挂载、服务管理或 AT 操作。

## 设备命令

```sh
systemctl status simpleadmin-httpd.service
systemctl restart simpleadmin-httpd.service
/usrdata/simpleadmin/simpleadmin-httpd --version
```

默认服务为 HTTP `:80`。手动启动可使用 `--no-tls=false` 启用 HTTPS，证书路径支持 `--cert`、`--key`、`--ca-cert`、`--ca-key`。支持 Go 版单横线参数写法。

服务运行期间通过 WebUI 的 AT 终端执行命令。独立 `at` 子命令需要先停止服务，防止两个持久读取线程争抢 SMD 响应。不要同时启动 Go 和 Rust 后端。
