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
node tests/tls.cjs
```

添加 `CAPTURE_DOCS=1` 可用模拟数据重新生成 README 截图。添加 `DEVICE_URL=http://127.0.0.1:18081` 则测试 ADB 转发后的真机；设备测试使用默认凭据时请先确认设备状态。`tests/device-soak.cjs` 支持 `DEVICE_USER`、`DEVICE_PASSWORD`，只运行只读 AT 查询和五分钟采集检查。

Windows 上运行 `powershell -ExecutionPolicy Bypass -File scripts/test-windows.ps1` 可检查预览程序、登录和模拟环境的密码持久化。

## 兼容性回归样本

对照版本为 Go 仓库提交 `a207171c4a62353610688e025fbcbbf92970a884`，包含已完成的 UI、密码、监测和导航修复。样本由 Go 原函数生成，而非手写 Rust 预期值。

样本已固定保存在 `tests/fixtures/`，Rust 测试直接读取 JSON，不执行 Go。迁移阶段使用的 Go 导出器和双后端设备比较脚本已移除；需要追溯样本生成过程时可查看 Git 历史。保留文件中的来源命名，以便核对历史验证记录。

## 打包

### Windows 图形设备助手

中文设备助手使用 WPF/.NET Framework，面向 Windows 10/11，不依赖 WinUI Windows App Runtime。源码为 `installer/Program.cs` 与内嵌的 `installer/MainWindow.xaml`，发布的 `SimpleAdmin-Setup.exe` 已编译好。

在 Windows PowerShell 中重新编译和测试：

```powershell
powershell -ExecutionPolicy Bypass -File scripts/build-installer.ps1
powershell -ExecutionPolicy Bypass -File scripts/build-installer.ps1 -TestBuild
powershell -ExecutionPolicy Bypass -File tests/installer-windows.ps1 -GuiTest
```

使用系统自带的 .NET Framework C# 编译器，在 Windows 临时目录编译后复制回仓库。测试专用构建保存在 `work/`，不会打包；其自动操作入口不包含在发布程序中。UI 测试使用临时模拟 ADB，不访问真实设备。

GUI 将所选设备明确传给安装脚本，通过结构化进度事件更新界面；同时核对进程退出码、最终结果与验证后的访问地址，避免从日志中的单个成功字样推断安装完成。主界面仅中文，完整诊断输出保留工具原文。

### 离线包

正式 `SimpleAdmin-Setup.exe` 内嵌 `payload.zip`，包含 ADB 和 DLL、内部 PowerShell 安装器、`development/` 完整内容与 LICENSE。每次修改上述资源后必须重新执行 Windows `scripts/build-installer.ps1`，再更新 SHA256SUMS。发布 ZIP 仅装入该 EXE；源码中的脚本作为内部实现和维护工具保留。

可运行 `SimpleAdmin-Setup.exe --verify-payload` 校验内嵌资源能解压、必要文件存在且内置 ADB 能启动；此检查不会连接设备或执行安装。运行时使用随机临时目录，报告单独保存到 LocalAppData；不终止共享 ADB 进程。

修改前端或安装运行辅助文件后，先更新安装文件校验清单。构建脚本也会自动执行这一步。

```sh
bash scripts/checksums.sh
sha256sum development/simpleadmin/simpleadmin-httpd.armv7 windows-test/bin/simpleadmin-httpd.exe SimpleAdmin-Setup.exe > SHA256SUMS
node tests/installer.cjs
bash scripts/package.sh
unzip -t packages/quectel-rgmii-toolkit-Rust-0.2.3-offline.zip
```

安装包只包含 `SimpleAdmin-Setup.exe`，ADB、设备程序、离线前端及安装逻辑均内嵌其中。`packages/`、开发缓存及运行状态不提交到 Git。发布时可直接提供 EXE，或将 ZIP 作为 GitHub Release 附件。

Windows 安装入口可通过 `powershell -ExecutionPolicy Bypass -File tests/installer-windows.ps1` 验证。它使用临时生成的模拟 ADB 和本机 HTTP 服务，不连接真实模块。Linux 安装测试同样使用隔离目录和模拟系统操作，不执行真实挂载、服务管理或 AT 操作。

## 设备命令

```sh
systemctl status simpleadmin-httpd.service
systemctl restart simpleadmin-httpd.service
/usrdata/simpleadmin/simpleadmin-httpd --version
```

默认服务为 HTTP `:80`。手动启动可使用 `--no-tls=false` 启用 HTTPS，证书路径支持 `--cert`、`--key`、`--ca-cert`、`--ca-key`。支持 Go 版单横线参数写法。

服务运行期间通过 WebUI 的 AT 终端执行命令。独立 `at` 子命令需要先停止服务，防止两个持久读取线程争抢 SMD 响应。不要同时启动 Go 和 Rust 后端。
