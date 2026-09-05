# 构建与打包

## 环境

使用 Linux 或 WSL 的 Linux 原生目录。需要 Git、Rustup、ARM GNU 交叉编译器、MinGW-w64、Zip。仅在重新生成 Go 对照样本时需要 Go。

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

## 重新生成 Go 对照样本

对照版本为 Go 仓库提交 `a207171c4a62353610688e025fbcbbf92970a884`，包含已完成的 UI、密码、监测和导航修复。样本由 Go 原函数生成，而非手写 Rust 预期值。

```sh
bash scripts/export-go-fixtures.sh /path/to/quectel-rgmii-toolkit-Go
cargo test --locked
```

脚本在临时 Linux 目录复制 Go 源码，在短信解析入口加入测试采样，运行原短信测试及样本导出，不改动输入仓库。脚本与 Go 导出器均保存在本仓库。

## 打包

```sh
bash scripts/package.sh
unzip -t packages/quectel-rgmii-toolkit-Rust-0.1.0-offline.zip
```

安装包包含 ADB、安装/卸载入口、设备程序、离线前端、Windows 预览程序、README、验证记录与校验文件。`packages/`、开发缓存及运行状态不提交到 Git。发布的 ZIP 作为 GitHub Release 附件提供。

## 设备命令

```sh
systemctl status simpleadmin-httpd.service
systemctl restart simpleadmin-httpd.service
/usrdata/simpleadmin/simpleadmin-httpd --version
```

默认服务为 HTTP `:80`。手动启动可使用 `--no-tls=false` 启用 HTTPS，证书路径支持 `--cert`、`--key`、`--ca-cert`、`--ca-key`。支持 Go 版单横线参数写法。

服务运行期间通过 WebUI 的 AT 终端执行命令。独立 `at` 子命令需要先停止服务，防止两个持久读取线程争抢 SMD 响应。不要同时启动 Go 和 Rust 后端。
