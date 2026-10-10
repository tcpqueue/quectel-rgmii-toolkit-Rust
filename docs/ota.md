# 在线更新（OTA）

从 v0.4.0 起，模块可以在“系统设置 → 在线更新”中检查并安装新版本，不再需要电脑和 ADB。更早的版本需要先用 Windows 设备助手升级到 v0.4.0。

## 使用

| 设置 | 说明 |
| --- | --- |
| 自动更新 | **关闭**、**每天检查，手动安装**（默认）、**每天检查并自动安装**。开机约 10 分钟后第一次检查，之后每 24 小时一次；检查失败（例如尚未联网或连不上 GitHub）时约 1 小时后重试。 |
| 更新源 | 留空即官方仓库 `tcpqueue/quectel-rgmii-toolkit-Rust`；可以填其他 GitHub 仓库 `owner/repo`，或自建网址，例如 `https://example.com/simpleadmin`。 |
| GitHub 代理 | 只用于 GitHub 来源。填前缀时把完整地址接在后面，例如 `https://ghfast.top/` 会访问 `https://ghfast.top/https://github.com/...`；若代理格式不同，可以写成含 `{url}` 的模板。 |
| 验证公钥 | 只有自定义源需要，填写该源的 Ed25519 公钥（Base64，32 字节）。官方公钥已内置。 |

“检查更新”显示最新版本和更新说明链接，“立即更新”下载并安装。安装期间页面会断开约一分钟，服务重启后自动刷新；会话随服务重启失效，需要重新登录。账号密码、HTTP 端口、短信与转发等设置都会保留。

## 安全

- 每个版本发布三个文件：`simpleadmin-ota-<版本>.tar.gz`（与设备助手上传的 `development/` 目录相同）、`simpleadmin-ota.json`（版本、文件名、大小、SHA-256）和 `simpleadmin-ota.json.sig`（Ed25519 签名）。
- 模块先校验清单签名，只接受内置官方公钥或你填写的公钥；再核对安装包大小与 SHA-256。代理或镜像无法替换安装包。
- 只会安装比当前版本更高的版本，旧的签名清单不能用来降级。
- 安装包只能包含 `development/` 下的普通文件和目录，链接、设备文件、绝对路径或 `..` 都会被拒绝。
- 校验通过后解压到 `/tmp/development`，由按需启动的 `simpleadmin-ota.service` 运行原有安装脚本。安装脚本会停止并重启网页服务，所以安装不能在网页服务进程内进行。结果和日志写入 `/tmp/simpleadmin-ota-result.env` 与 `/tmp/simpleadmin-ota.log`，失败时会在页面上显示。安装脚本失败且网页服务已被停止时，会尝试重新启动网页服务，以便查看错误并重试。

## 发布

`scripts/package.sh` 会调用 `scripts/ota-package.sh` 生成上述三个文件，然后与 EXE 一起作为 Release 附件上传。签名私钥默认在 `~/.config/simpleadmin/ota-ed25519.pem`，也可以用 `SIMPLEADMIN_OTA_KEY` 指定；脚本会确认它与 `src/ota-public-key.txt` 对应。

私钥不进入仓库，请离线备份。丢失后无法再为已安装的模块签发更新，只能换新密钥，并让用户用设备助手重新安装一次。

## 自建源或分支

1. 生成密钥：`openssl genpkey -algorithm ed25519 -out ota.pem`。
2. 导出公钥：`openssl pkey -in ota.pem -pubout -outform DER | tail -c 32 | base64`，填到模块的“验证公钥”。
3. 用 `SIMPLEADMIN_OTA_KEY=ota.pem` 运行打包。脚本要求私钥与 `src/ota-public-key.txt` 一致，分支需要先把该文件换成自己的公钥。
4. 发布到 GitHub Release，或把三个文件放在同一个网址目录下。
