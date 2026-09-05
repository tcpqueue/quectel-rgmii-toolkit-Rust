# Quectel RGMII Toolkit · Rust

面向移远 RM520N-EU 的轻量设备管理后台。Rust 原生后端，保留 Go 版的页面、AT 操作和安装方式，支持中文、English、Русский、العربية。

[下载离线安装包](https://github.com/tcpqueue/quectel-rgmii-toolkit-Rust/releases/latest) · [功能核对与真机测试](docs/validation.md) · [构建说明](docs/build.md) · [Go 原项目](https://github.com/snjzb/quectel-rgmii-toolkit-Go)

![总览界面](docs/images/overview.png)

> 截图使用模拟数据。已在 RM520N-EU 真机安装测试，其他型号和固件尚未完成实机验证。

## 安装

1. 在 [Releases](https://github.com/tcpqueue/quectel-rgmii-toolkit-Rust/releases/latest) 下载 `offline.zip`，完整解压。
2. 将模块连接到 Windows 电脑，确认 ADB 能识别设备。安装包已包含 ADB，无需编译或联网下载依赖。
3. 双击 `toolkit.bat`，等待安装成功。只有脚本提示需要重启时才重启设备。
4. 浏览器访问模块地址，通常为 `http://192.168.225.1`。首次打开先选择语言，再使用 `admin / admin` 登录。

也可以通过 ADB 转发访问：

```powershell
.\adb.exe forward tcp:18081 tcp:80
```

随后打开 `http://127.0.0.1:18081`。

**升级已有 Go 版：** 直接运行安装工具。脚本停止旧进程、替换程序和页面，保留有效的 Web 登录凭据、TTL、监测目标和 root 密码初始化标记；无需先卸载。首次安装将系统 root 密码初始化为 `admin`，已有初始化标记时不重置密码。

**修改密码：** 登录后进入系统设置，可分别修改 Web 登录密码和系统 root 密码，需要验证当前密码。修改后现有会话失效，重新登录即可。两种密码独立保存。

**卸载：** 双击 `uninstall.bat`。卸载会删除应用及其配置，升级时请使用安装工具。

## 功能

| 页面 | 内容 |
| --- | --- |
| 总览 | CPU、内存、网络状态、流量、频段、小区、信号和天线读数 |
| 网络与小区 | APN、网络模式、频段选择、LTE/NR 小区扫描、手动及扫描结果锁小区 |
| 系统设置 | Web/root 密码、TTL、IP 透传、DNS 代理、USB 网络模式、DMZ、LAN IP、AT 终端 |
| 短信 | 收件箱、长短信合并、分段发送、号码国家代码补全、批量删除 |
| 设备信息 | 型号、版本、SIM 和设备标识等信息 |
| 控制台 | 使用系统 root 密码认证的原生 PTY 终端 |

UI 延续 Art Design Pro 的布局和配色，静态资源本地提供，支持明暗主题、移动端和阿拉伯语从右到左布局。总览把主要信息放在上方，趋势图放在下方；读不到 4G 数据时隐藏相关读数与图例，SINR 为 0 的有效数据正常显示。

## 五分钟趋势

![信号、温度与延迟走势](docs/images/monitoring.png)

| 数据 | 周期 | 保留量 | 存储 |
| --- | --- | --- | --- |
| Ping RTT / Jitter | 1 秒 | 最近 5 分钟，最多 300 点 | 内存 |
| RSRP / SINR / 温度 | 5 秒 | 最近 5 分钟，最多 60 点 | 内存 |
| 下载 / 上传速率与流量占比 | 5 秒 | 最近 5 分钟，最多 60 点 | 内存 |

- 默认 Ping `www.baidu.com`，可改为域名、IPv4 或 IPv6 地址。
- 采集由设备后台运行，关闭网页后继续；重启程序后历史清空。
- RSRP 与 SINR 共用图表，分别使用左右坐标轴。温度单独绘图。
- 下载与上传共用双折线图，按实际 AT 采样间隔计算速率，重复缓存不产生虚假的零速率。占比按近五分钟有效采样区间的上下行字节数计算；无流量时显示空占比。
- 单点 Jitter 为 `abs(RTT[n] - RTT[n-1])`，平均抖动为窗口内有效差值的算术平均。丢包、采样间断和窗口外的数据不参与跨点配对。
- DNS 解析失败与 ICMP 超时分别记录。Ping 结果表示目标的 ICMP 可达性。

## 资源与存储

后端使用单线程异步运行时、一个 AT 工作线程及一个串口读取线程，直接访问 `/dev/smd11`，不再启动 Go SMD 辅助进程或 `socat`。请求串行执行，短信的提示符、PDU 和结束响应属于同一事务。

历史数据采用固定长度环形数组和定点数，三类历史的原始数组合计不超过 **8,160 字节**。AT 队列最多 16 个请求，缓存最多 64 项、响应内容合计最多 1 MiB，会话与 WebSocket 也有数量限制。这些是内部缓冲区上限，不代表程序总内存占用。

持久化设置统一执行 **根目录 rw → 写临时文件 → 同步 → 原子替换 → 根目录 ro**。内容不变时跳过写入；曲线、会话、锁文件均留在内存或 tmpfs。安装后的服务不输出持续日志，控制台不写 shell 历史。应用目录及可执行文件权限为 `777`，认证文件为 `600`。

v0.1.0 在 RM520N-EU 同条件后台采样中，CPU 约 **1.46% → 0.45%**，PSS 内存约 **7.25 MiB → 1.40 MiB**；正常页面访问场景下 Rust 约 **3.1 MiB**。ARM 程序从 7.93 MB 缩小到约 4.15 MB。历史基线的采样条件与适用范围见 [验证记录](docs/validation.md)。

## 界面与语言

![首次登录与语言选择](docs/images/login.png)

| 中文移动端 | العربية |
| --- | --- |
| ![中文移动端](docs/images/mobile.png) | ![阿拉伯语移动端](docs/images/arabic-mobile.png) |

语言选择保存在当前浏览器中，不因切换语言反复写入模块存储。旧版语言 API 仍保留兼容。

## Windows 预览

双击 `windows-test/start.bat`，打开 `http://127.0.0.1:8080`，默认 `admin / admin`。该模式使用模拟 AT 和监测数据，便于预览页面；不能代替模块硬件测试。

## 开发

源码、锁定依赖、ARMv7 静态可执行文件及 Windows 预览程序均随仓库提供。编译与打包步骤见 [docs/build.md](docs/build.md)。WSL 中请在 Linux 原生目录构建，例如 `~/projects/`；`/mnt/...` 只用于传输文件。

## 来源与许可

基于 [snjzb/quectel-rgmii-toolkit-Go](https://github.com/snjzb/quectel-rgmii-toolkit-Go) 的功能和前端迁移，保留原项目 MIT 许可与署名。界面参考 [Art Design Pro](https://github.com/Daymychen/art-design-pro)，图表和图标使用 ECharts、Lucide。相关前端许可位于 `development/simpleadmin/www/licenses/`。Rust 依赖版本见 `Cargo.lock`。
