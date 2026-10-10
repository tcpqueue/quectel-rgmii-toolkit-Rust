(function (global) {
  'use strict';
  const root = global.SimpleAdmin = global.SimpleAdmin || {};
  root.Lang = root.Lang || (function () {
    const storageKey = 'simpleadmin.language';
    const defaultLanguage = 'zh-CN';
    const supportedLanguages = ['zh-CN', 'en'];
    let currentLanguage = normalizeLanguage(navigator.language) || defaultLanguage;
    try { currentLanguage = normalizeLanguage(localStorage.getItem(storageKey)) || currentLanguage; } catch (_) {}

    const translations = {
  "en": {
"本卡短信": "SIM SMS",
"对接说明": "Setup guide",
"平台网站": "Platform website",
"转发设置": "Forwarding settings",
"使用本卡发送，运营商可能按短信分段收费": "Sends using this SIM; your operator may charge per SMS segment",
"生效方式": "Persistence",
"临时锁频": "Temporary cell lock",
"持久化锁频": "Persistent cell lock",
"锁频后 3 分钟未拨号成功则取消，并停用开机恢复": "Unlock and disable boot restore if no data session is established within 3 minutes",
"临时锁频重启后失效；持久化锁频在应用启动后自动恢复，适用于下方手动或扫描小区锁定。": "Temporary locks end on reboot. Persistent locks restore when the app starts. Applies to manual and scanned cell locks below.",
"重新启用": "Re-enable",
"取消锁频": "Unlock",
"已拨号，锁频保持": "Data connected; lock retained",
"已超时回退，开机恢复已关闭": "Timed out and unlocked; boot restore disabled",
"回退失败，正在重试": "Unlock failed; retrying",
"等待开机恢复": "Waiting for restore",
"NR5G-SA 每次只能锁定一个小区": "NR5G-SA supports one locked cell at a time",
"invalid forwarding phone number": "Invalid forwarding phone number",
"Forwarding destination is the sender; loop prevented": "Forwarding destination is the sender; loop prevented",
"端口已被占用或不可用，原配置保持不变":"Port is occupied or unavailable. The previous configuration was kept.","新密码留空时保留原密码，修改后需要重新登录":"Leave the new password blank to keep it. Sign in again after saving.","请求超时，请重新检查当前端口":"Request timed out. Check the current port.","端口已保存并生效，无需重启模块":"Port saved and active. No module restart required.","请填写当前 Web 密码和 1–65535 的端口":"Enter the current Web password and a port from 1 to 65535","保存端口":"Save port","端口未改变":"Port unchanged","当前 Web 密码":"Current Web password","读取 WebUI 设置失败":"Unable to load WebUI settings","WebUI HTTP 端口":"WebUI HTTP port","WebUI 账号与密码":"WebUI account and password","请输入当前密码和 Web 账号":"Enter the current password and Web username","打开新地址":"Open new address","Web 账号":"Web username","端口保存后立即生效，不重启模块；请使用新端口重新访问":"Port changes take effect immediately without restarting the module. Reconnect using the new port.","通过 ADB 转发访问时，请在安装器重新打开管理页面":"For ADB forwarding, reopen the management page from the installer","HTTP 端口":"HTTP port",

"系统时间异常，自动删除已延后": "System clock is unreliable; automatic deletion has been postponed",
"重启或时间异常后，自动删除会延后至少 24 小时。": "After a restart or clock anomaly, automatic deletion is postponed for at least 24 hours.",
"短信转发": "SMS forwarding",
"启用短信功能": "Enable SMS service",
"全部渠道转发成功 24 小时后自动删除": "Delete 24 hours after all channels confirm delivery",
"待删除": "Scheduled for deletion",
"短信功能已关闭": "SMS service is disabled",
"下载与上传": "Download and upload",
"下载": "Download",
"上传": "Upload",
"近 5 分钟流量占比": "Traffic share over the last 5 minutes",
"等待流量采样": "Waiting for traffic samples",
"最近五分钟下载和上传速率走势图": "Download and upload speeds over the last five minutes",
"关闭菜单": "Close menu",
"展开或收起菜单": "Expand or collapse menu",
"刷新页面": "Refresh page",
"切换明暗主题": "Toggle light or dark theme",
"账户设置": "Account settings",
"4G 与 5G 实时信号": "Live 4G and 5G signal",
"信息详略": "Information detail level",
"刷新间隔（秒）": "Refresh interval (seconds)",
"无线制式": "Radio technology",
"关闭": "Close",
"无本机号码": "No phone number",
"连接中...": "Connecting...",
    "首页": "Home",
    "网络": "Network",
    "设置": "Settings",
    "短信": "SMS",
    "控制台": "Console",
    "设备信息": "Device Info",
    "总览": "Overview",
    "网络与小区": "Network and Cells",
    "系统设置": "System Settings",
    "短信服务": "SMS Service",
    "退出登录": "Log Out",
    "菜单": "Menu",
    "暗夜模式": "Night Mode",
    "浅色模式": "Light Mode",
    "温度": "Temperature",
    "SIM 卡": "SIM Card",
    "信号百分比": "Signal Percentage",
    "互联网连接": "Internet Connection",
    "CPU 使用率": "CPU Usage",
    "RAM 使用量": "RAM Usage",
    "实时负载": "Live Load",
    "网络信息": "Network Info",
    "精简显示": "Compact View",
    "完整显示": "Full View",
    "激活 SIM": "Active SIM",
    "运营商": "Carrier",
    "网络模式": "Network Mode",
    "频段": "Bands",
    "带宽": "Bandwidth",
    "在线时长": "Uptime",
    "刷新频率（最少 2 秒）": "Refresh rate (minimum 2 seconds)",
    "信号信息": "Signal Info",
    "信号评估": "Signal Assessment",
    "累计流量": "Total Traffic",
    "速率": "Speed",
    "更新时间": "Last Updated",
    "刷新": "Refresh",
    "制造商": "Manufacturer",
    "型号名称": "Model Name",
    "固件版本": "Firmware Version",
    "电话号码": "Phone Number",
    "局域网IP": "LAN IP",
    "广域网IPv": "WAN IPv",
    "版本": "Version",
    "更新": "Update",
    "访问": "Visit",
    "代码库": "repository",
    "或": "or",
    "文档": "documentation",
    "以获取更多信息。版权所有。2024": "for more information. Copyright. 2024",
    "这将重启调制解调器。": "This will reboot the modem.",
    "继续吗？": "Continue?",
    "继续？": "Continue?",
    "重启": "Reboot",
    "取消": "Cancel",
    "重启中...": "Rebooting...",
    "设备可能已经开始重启，正在显示重启倒计时。": "The device may have started rebooting, showing the reboot countdown.",
    "正在禁用 IP 透传，网口会重启，请等待倒计时结束。": "Disabling IP passthrough. The network port will restart. Please wait for the countdown to finish.",
    "请等待": "Please wait",
    "秒。": "seconds.",
    "加载中...": "Loading...",
    "AT 终端": "AT Terminal",
    "AT 命令": "AT Command",
    "AT 命令输出": "AT Command Output",
    "用分号（;）分隔多个命令，后端会逐条发送，示例：AT+CFUN?;+CCID": "Separate multiple commands with semicolons (;). The backend sends them one by one. Example: AT+CFUN?;+CCID",
    "提交": "Submit",
    "清除": "Clear",
    "一键实用工具": "Quick Tools",
    "重置 AT&F": "Reset AT&F",
    "重置": "Reset",
    "IP 透传": "IP Passthrough",
    "当前：已启用": "Current: Enabled",
    "当前：未启用": "Current: Disabled",
    "当前：": "Current:",
    "未指定": "Unspecified",
    "USB 协议": "USB Protocol",
    "ECM (推荐)": "ECM (Recommended)",
    "更改": "Change",
    "DMZ 设置": "DMZ Settings",
    "启用": "Enable",
    "禁用": "Disable",
    "LAN IP 设置": "LAN IP Settings",
    "保存": "Save",
    "其他设置": "Other Settings",
    "TTL 设置": "TTL Settings",
    "TTL 状态和值": "TTL Status and Value",
    "重启 / 重置 AT&F": "Reboot / Reset AT&F",
    "TTL 已激活": "TTL Enabled",
    "TTL 未激活": "TTL Disabled",
    "设置 TTL 值为 0 以禁用。": "Set TTL value to 0 to disable.",
    "语言设置": "Language Settings",
    "界面语言": "Interface Language",
    "中文": "Chinese",
    "已保存": "Saved",
    "保存失败": "Save failed",
    "语言已保存。": "Language saved.",
    "登录密码": "Login Password",
    "账户安全": "Account Security",
    "当前密码": "Current Password",
    "新密码": "New Password",
    "确认新密码": "Confirm New Password",
    "修改密码": "Change Password",
    "更改登录密码": "Change Login Password",
    "请输入当前密码和新密码": "Enter the current password and new password",
    "两次输入的新密码不一致": "The new passwords do not match",
    "密码已保存，请使用新密码重新登录。": "Password saved. Please log in again with the new password.",
    "密码保存失败": "Password save failed",
    "当前密码不正确": "Current password is incorrect",
    "新密码不能为空": "New password cannot be empty",
    "密码确认不一致": "Password confirmation mismatch",
    "请输入当前登录密码": "Enter current login password",
    "请输入新的登录密码": "Enter new login password",
    "请再次输入新的登录密码": "Enter new login password again",
    "网络工具": "Network Tools",
    "IP 协议类型": "IP Protocol Type",
    "保存更改": "Save Changes",
    "小区锁定": "Cell Lock",
    "选择扫描模式": "Select scan mode",
    "当前：选择扫描模式": "Current: Select scan mode",
    "全面扫描": "Full Scan",
    "仅LTE": "LTE Only",
    "仅NR5G": "NR5G Only",
    "开始扫描": "Start Scan",
    "扫描中... 请稍候.": "Scanning... Please wait.",
    "锁定": "Lock",
    "小区扫描需要几分钟完成，请勿离开次页面。": "Cell scanning may take a few minutes. Do not leave this page.",
    "小区数量": "Number of Cells",
    "锁定LTE小区": "Lock LTE Cell",
    "锁定NR5G-SA小区": "Lock NR5G-SA Cell",
    "解锁LTE小区": "Unlock LTE Cell",
    "解锁NR5G-SA小区": "Unlock NR5G-SA Cell",
    "初始化网络...": "Initializing network...",
    "锁定频段": "Band Lock",
    "频段锁定": "Band Lock",
    "复选框将会在这里生成": "Checkboxes will be generated here",
    "获取支持的频段中...": "Getting supported bands...",
    "锁定当前选中": "Lock Selected",
    "启用所有": "Enable All",
    "小区扫描": "Cell Scan",
    "小区扫描将扫描您所在区域的所有LTE和NR5G-SA小区。扫描可能会断开您的网络连接，并需要几分钟完成。": "Cell scan will scan all LTE and NR5G-SA cells in your area. The scan may interrupt your network connection and may take a few minutes.",
    "选择": "Select",
    "频点": "ARFCN",
    "信号": "Signal",
    "收件箱": "Inbox",
    "读取短信...": "Reading SMS...",
    "无短信": "No SMS",
    "全选": "Select All",
    "取消选中": "Deselect",
    "发件人:": "Sender:",
    "日期和时间:": "Date and Time:",
    "删除": "Delete",
    "发信息": "Send Message",
    "收件人号码": "Recipient Number",
    "短信内容": "Message Content",
    "发送短信": "Send SMS",
    "正在注销...": "Logging out...",
    "主导航": "Main navigation",
    "切换导航": "Toggle navigation",
    "新的 IMEI": "New IMEI",
    "DMZ IP 地址": "DMZ IP Address",
    "TTL 值": "TTL Value",
    "网关 IP 地址": "Gateway IP Address",
    "起始地址": "Start Address",
    "结束地址": "End Address",
    "输入收件人号码": "Enter recipient number",
    "输入短信内容": "Enter message content",
    "优秀": "Excellent",
    "良好": "Good",
    "一般": "Fair",
    "差": "Poor",
    "无信号": "No Signal",
    "已激活": "Active",
    "未激活": "Inactive",
    "已连接": "Connected",
    "未连接": "Disconnected",
    "测试模式": "Test Mode",
    "未知": "Unknown",
    "未知时间": "Unknown Time",
    "未插卡": "No SIM",
    "没有提供新的 IMEI。": "No new IMEI was provided.",
    "IMEI 无效": "Invalid IMEI",
    "新的 IMEI 与当前 IMEI 相同。": "The new IMEI is the same as the current IMEI.",
    "IMEI 与当前 IMEI 相同": "The IMEI is the same as the current IMEI",
    "请输入新的 IMEI": "Enter new IMEI",
    "获取IMEI中...": "Getting IMEI...",
    "请输入所有必填字段": "Please fill in all required fields",
    "请输入至少一个有效的earfcn和pci对": "Enter at least one valid EARFCN and PCI pair",
    "请输入要锁定的小区数量": "Enter the number of cells to lock",
    "请至少选择一个小区进行锁定.": "Please select at least one cell to lock.",
    "最多只能选择 10 条小区进行锁定": "You can select up to 10 cells to lock",
    "选择的网络模式无效": "Invalid network mode selection",
    "没有选中任何频段，请选择至少一个频段！": "No bands selected. Select at least one band.",
    "没有做出更改": "No changes were made",
    "某些必填字段缺失，请检查小区数据": "Some required fields are missing. Check the cell data.",
    "网关 IP 地址格式无效！": "Invalid gateway IP address format.",
    "请输入有效的网关 IP 地址和起始、结束 IP 地址！": "Enter a valid gateway IP address, start address, and end address.",
    "未指定 IP 透传模式": "IP passthrough mode is not specified",
    "无效的 IP 透传模式": "Invalid IP passthrough mode",
    "未指定 USB 网络模式": "USB network mode is not specified",
    "USB 网络模式无效": "Invalid USB network mode",
    "未检测到 SIM 卡": "No SIM card detected",
    "短信发送成功！": "SMS sent successfully.",
    "未知错误": "Unknown error",
    "没有有效的短信索引": "No valid SMS index",
    "短信索引未正确初始化或为空": "SMS indexes were not initialized correctly or are empty",
    "自动": "Auto",
    "选择首选网络": "Select preferred network",
    "NR5G模式控制": "NR5G Mode Control",
    "获取中...": "Loading...",
    "已启用": "Enabled",
    "未启用": "Disabled",
    "禁用NR5G-NSA": "Disable NR5G-NSA",
    "禁用NR5G-SA": "Disable NR5G-SA",
    "解锁LTE": "Unlock LTE",
    "解锁NR5G-SA": "Unlock NR5G-SA",
    "活动频段:": "Active Bands:",
    "开始小区扫描": "Start Cell Scan",
    "空": "Empty",
    "数据": "Data",
    "网络已断开": "Network disconnected",
    "正在解析LTE数据": "Parsing LTE data",
    "正在解析NR5G-SA数据": "Parsing NR5G-SA data",
    "PCC PCI值:": "PCC PCI:",
    "SCC PCI值:": "SCC PCI:",
    "天": "days",
    "小时": "hours",
    "分钟": "minutes",
    "未锁定": "Unlocked",
    "已锁定4G": "4G Locked",
    "已锁定5G": "5G Locked",
    "已锁定4G和5G": "4G and 5G Locked",
    "未禁用": "Not Disabled",
    "禁用NSA": "NSA Disabled",
    "禁用SA": "SA Disabled",
    "获取频段中...": "Getting bands...",
    "全部选中": "Select All",
    "中国移动": "China Mobile",
    "中国联通": "China Unicom",
    "中国电信": "China Telecom",
    "中国广电": "China Broadnet",
    "中国铁通": "China Tietong",
    "中国卫通": "China Satcom",
    "国家电网": "State Grid",
    "号码或内容不能为空": "Phone number or message cannot be empty",
    "短信发送失败：": "SMS sending failed:",
    "AT命令执行结果:": "AT command result:",
    "发送AT命令失败:": "Failed to send AT command:",
    "错误:": "Error:",
    "登录": "Log in",
    "登录中...": "Logging in...",
    "用户名": "Username",
    "密码": "Password",
    "请输入账号和密码": "Enter your username and password",
    "用户名或密码错误": "Incorrect username or password",
    "语言": "Language",
    "设备管理": "Device management",
    "管理员": "Administrator",
    "全屏": "Fullscreen",
    "设备状态": "Device status",
    "内存使用率": "Memory usage",
    "信号质量": "Signal quality",
    "互联网": "Internet",
    "实时信号": "Live signal",
    "制式": "Radio",
    "概览": "Overview",
    "详情": "Details",
    "接入网络": "Network access",
    "地址与流量": "Addresses and traffic",
    "小区参数": "Cell parameters",
    "下载速率": "Download speed",
    "上传速率": "Upload speed",
    "累计下载": "Downloaded",
    "累计上传": "Uploaded",
    "RSRP 天线": "RSRP antennas",
    "刷新间隔": "Refresh interval",
    "秒": "seconds",
    "保存刷新间隔": "Save refresh interval",
    "显示敏感信息": "Show sensitive information",
    "隐藏敏感信息": "Hide sensitive information",
    "当前延迟": "Current latency",
    "平均抖动": "Average jitter",
    "丢包率": "Packet loss",
    "模块温度": "Module temperature",
    "相邻 RTT 绝对差": "Absolute change between adjacent RTTs",
    "应答": "Replies",
    "最近 5 分钟": "Last 5 minutes",
    "模拟数据": "Simulated data",
    "无线信号": "Radio signal",
    "温度趋势": "Temperature history",
    "延迟与抖动": "Latency and jitter",
    "5 秒采样": "Sampled every 5 s",
    "ICMP · 1 秒采样": "ICMP · every 1 s",
    "监测设置": "Monitoring settings",
    "延迟监测": "Latency probe",
    "间隔（秒）": "Interval (s)",
    "信号与流量采样": "Signal & traffic sampling",
    "保存设置": "Save settings",
    "后台采样已关闭": "Background sampling off",
    "延迟监测已关闭": "Latency probe off",
    "监测设置已保存": "Monitoring settings saved",
    "监测设置保存失败，请重试": "Could not save monitoring settings; try again",
    "延迟间隔为 1–300 秒，采样间隔为 2–300 秒": "Latency interval must be 1–300 s and sampling interval 2–300 s",
    "请求超时，请确认设置是否已更新": "Request timed out; check whether the settings were applied",
    "向监测目标发送 ICMP，用于延迟、抖动、丢包和总览“网络连接”。关闭后网络连接改按模块拨号状态判断。": "Sends ICMP to the monitor target for latency, jitter, loss and the Overview connection status. When off, connection status follows the module data session.",
    "后台定时查询信号、温度和流量计数，会占用 AT 口。关闭后只在总览页打开时随页面刷新记录。": "Queries signal, temperature and traffic counters in the background, which uses the AT port. When off, samples are recorded only while the Overview page refreshes.",
    "蜂窝流量估算：": "Estimated cellular data:",
    "MB/天": "MB/day",
    "°C · 5 秒采样": "°C · every 5 s",
    "Ping 目标": "Ping target",
    "应用": "Apply",
    "保存中…": "Saving…",
    "目标已保存": "Target saved",
    "重试": "Retry",
    "等待首次采样": "Waiting for first sample",
    "等待温度数据": "Waiting for temperature data",
    "平均": "Average",
    "最低": "Minimum",
    "最高": "Maximum",
    "失败": "Failed",
    "次": "times",
    "等待采样": "Waiting for samples",
    "读取失败": "Read failed",
    "链路正常": "Connected",
    "应答超时": "Reply timed out",
    "DNS 解析失败": "DNS resolution failed",
    "ICMP 权限不足": "ICMP permission denied",
    "网络不可达": "Network unreachable",
    "无法读取监控数据": "Unable to read monitoring data",
    "请输入有效域名或 IP，不包含协议、端口或路径": "Enter a hostname or IP without protocol, port or path",
    "目标保存失败，请重试": "Failed to save target. Try again",
    "请求超时，请确认目标是否已更新": "Request timed out. Check whether the target was updated",
    "延迟": "Latency",
    "抖动": "Jitter",
    "检测失败": "Check failed",
    "账户安全 · Web 登录密码": "Account security · Web password",
    "系统 root 密码": "System root password",
    "当前 root 密码": "Current root password",
    "新 root 密码": "New root password",
    "确认新 root 密码": "Confirm new root password",
    "修改 root 密码": "Change root password",
    "IMEI 设置": "IMEI settings",
    "启用 DNS": "Enable DNS",
    "恢复全部": "Restore all",
    "当前 root 密码不正确": "Incorrect current root password",
    "请填写当前 root 密码，并确认两次新密码一致": "Enter the current root password and matching new passwords",
    "请求超时，请确认设备密码与挂载状态": "Request timed out. Check the device password and mount status",
    "密码不能为空、不能包含换行，且两次新密码必须一致（最多128字节）": "Passwords must match, contain no line breaks and be 1–128 bytes",
    "root 密码保存或恢复只读失败，请检查设备状态": "Failed to save root password or restore read-only mode. Check the device",
    "模块温度最近五分钟走势图": "Module temperature over the last five minutes",
    "最近五分钟延迟、抖动和失败采样走势图": "Latency, jitter and failed samples over the last five minutes",
"设备": "Device",
"系统": "System",
"外观": "Appearance",
"浅色": "Light",
"深色": "Dark",
"正在查询当前已锁定频段...": "Checking locked bands...",
"活动频段": "Active bands",
"勾选要保留的频段后点击“锁定”；“恢复全部”会解除所有制式的频段限制。": "Select the bands to keep, then choose Lock. Restore All removes the band limits for every mode.",
"网络参数": "Network parameters",
"只提交修改过的项目，保存后模块会重新注册网络。": "Only changed items are sent. The module re-registers to the network after saving.",
"扫描小区": "Cell scan",
"扫描模式": "Scan mode",
"暂无扫描结果": "No scan results",
"点选结果行后锁定；扫描需要几分钟，请勿离开此页面。": "Select result rows, then lock them. A scan takes a few minutes; stay on this page.",
"锁定所选": "Lock selected",
"手动锁定": "Manual lock",
"操作": "Action",
"保存账号": "Save account",
"网络功能": "Network features",
"DNS 代理 IPv4": "IPv4 DNS proxy",
"DNS 代理 IPv6": "IPv6 DNS proxy",
"偏好设置": "Preferences",
"维护": "Maintenance",
"重启模块": "Restart module",
"约 1 分钟后恢复连接": "The connection returns in about a minute",
"恢复模块 AT 出厂配置，随后确认重启": "Restores the module's AT factory settings, then asks to restart",
"这将修改 IMEI 并重启调制解调器。": "This changes the IMEI and restarts the modem.",
"短信详情": "Message details",
"模块": "Module",
"网络地址": "Network addresses",
"关于": "About",
"问题反馈": "Feedback",
"更多": "More",
"频段制式": "Band mode",
"留空则保留原密码": "Leave blank to keep the current password",
"命令输出会显示在这里": "Command output appears here",
"短信功能": "SMS features",
"必填": "Required",
"成功": "Saved"
  }
};

    function normalizeLanguage(language) {
      const value = String(language || '').trim().toLowerCase();
      if (value === 'english' || /^en(?:-|$)/.test(value)) return 'en';
      if (value === 'chinese' || value === 'cn' || /^zh(?:-|$)/.test(value)) return 'zh-CN';
      return '';
    }

    function normalizeText(value) {
      return String(value || '').replace(/\s+/g, ' ').trim();
    }

    function translateForLanguage(key, language) {
      const source = String(key || '');
      const lang = normalizeLanguage(language) || defaultLanguage;
      if (lang === 'zh-CN') return source;

      const table = translations[lang] || {};
      if (Object.prototype.hasOwnProperty.call(table, source)) return table[source];
      let match = source.match(/^(\d+\s*\/\s*\d+) 应答$/);
      if (match) return match[1] + ' ' + table['应答'];
      match = source.match(/^失败 (\d+) 次$/);
      if (match) return table['失败'] + ': ' + match[1];
      match = source.match(/^已激活卡(\d+)$/);
      if (match) return table['已激活'] + ' SIM ' + match[1];
      match = source.match(/^(\d+) (天|小时|分钟)$/);
      if (match) return match[1] + ' ' + table[match[2]];
      match = source.match(/^(ICMP · )?(\d+) 秒采样$/);
      if (match) {
        const label = lang === 'en' ? 'every {n} s' : '{n}';
        return (match[1] || '') + label.replace('{n}', match[2]);
      }
      match = source.match(/^等待 (NR|LTE) 信号数据$/);
      if (match) return table['等待采样'] + ' · ' + match[1];
      match = source.match(/^(NR|LTE) RSRP 和 SINR 最近五分钟走势图$/);
      if (match) return match[1] + ' RSRP / SINR · ' + table['最近 5 分钟'];
      for (const prefix of ['活动频段:', 'PCC PCI值:', 'SCC PCI值:', 'AT命令执行结果:', '发送AT命令失败:', '错误:']) {
        if (source.startsWith(prefix)) return table[prefix] + ' ' + source.slice(prefix.length).trim();
      }

      if (source.startsWith('当前：') || source.startsWith('当前:')) {
        const marker = source.startsWith('当前：') ? '当前：' : '当前:';
        const value = source.slice(marker.length).trim();
        return 'Current: ' + (value ? translateForLanguage(value, lang) : '');
      }

      if (source.startsWith('短信发送失败：')) {
        const value = source.slice('短信发送失败：'.length).trim();
        return translateForLanguage('短信发送失败：', lang) + ' ' + (value ? translateForLanguage(value, lang) : '');
      }

      const gettingMatch = source.match(/^获取(.+)中\.\.\.$/);
      if (gettingMatch) return translateForLanguage('获取中...', lang) + ' ' + translateForLanguage(gettingMatch[1], lang);

      return source;
    }

    function translate(key) {
      return translateForLanguage(key, currentLanguage);
    }

    function isRenderedFromKey(key, renderedText) {
      const rendered = normalizeText(renderedText);
      if (!key || !rendered) return false;
      if (rendered === normalizeText(key)) return true;
      return supportedLanguages.some((language) => rendered === normalizeText(translateForLanguage(key, language)));
    }

    function resolveI18nKey(currentValue, storedKey) {
      const current = normalizeText(currentValue);
      if (!current) return '';
      if (storedKey && isRenderedFromKey(storedKey, current)) return storedKey;
      return current;
    }

    function withOriginalWhitespace(value, replacement) {
      const text = String(value || '');
      const prefix = (text.match(/^\s*/) || [''])[0];
      const suffix = (text.match(/\s*$/) || [''])[0];
      return prefix + replacement + suffix;
    }

    function skipTextElement(element) {
      if (!element || element.nodeType !== 1) return false;
      const tag = element.tagName.toLowerCase();
      return ['script', 'style', 'svg', 'path', 'code', 'pre', 'textarea'].includes(tag)
        || Boolean(element.closest('[data-no-i18n]'));
    }

    function skipAttributeElement(element) {
      if (!element || element.nodeType !== 1) return false;
      const tag = element.tagName.toLowerCase();
      return ['script', 'style', 'svg', 'path', 'code', 'pre'].includes(tag)
        || Boolean(element.closest('[data-no-i18n]'));
    }

    let applying = false;
    let observer = null;
    let applyTimer = null;
    let autoLoadStarted = false;

    function translateTextNodes(rootNode) {
      if (!rootNode || typeof document === 'undefined') return;
      const start = rootNode.nodeType === 9 ? rootNode.body : rootNode;
      if (!start) return;
      const walker = document.createTreeWalker(start, NodeFilter.SHOW_TEXT, {
        acceptNode(node) {
          const parent = node.parentElement;
          if (!parent || skipTextElement(parent) || !normalizeText(node.nodeValue)) {
            return NodeFilter.FILTER_REJECT;
          }
          return NodeFilter.FILTER_ACCEPT;
        }
      });
      const nodes = [];
      while (walker.nextNode()) nodes.push(walker.currentNode);
      nodes.forEach((node) => {
        const parentKey = node.parentElement && node.parentElement.dataset
          ? node.parentElement.dataset.simpleadminI18nKey
          : '';
        const key = parentKey || resolveI18nKey(node.nodeValue, node.__simpleadminI18nKey);
        if (!key) return;
        node.__simpleadminI18nKey = key;
        const translated = translate(key);
        const nextValue = withOriginalWhitespace(node.nodeValue, translated);
        if (node.nodeValue !== nextValue) node.nodeValue = nextValue;
      });
    }

    function translateAttributes(rootNode) {
      const start = rootNode && rootNode.nodeType === 1 ? rootNode : document;
      if (!start || typeof start.querySelectorAll !== 'function') return;
      const attributes = ['aria-label', 'placeholder', 'title'];
      const elements = [];
      if (start.nodeType === 1) elements.push(start);
      start.querySelectorAll('*').forEach((element) => elements.push(element));
      elements.forEach((element) => {
        if (skipAttributeElement(element)) return;
        attributes.forEach((attr) => {
          if (!element.hasAttribute(attr)) return;
          const storeName = 'simpleadminI18n' + attr.replace(/[^a-z0-9]/gi, '');
          const key = resolveI18nKey(element.getAttribute(attr), element.dataset[storeName]);
          if (!key) return;
          element.dataset[storeName] = key;
          const translated = translate(key);
          if (element.getAttribute(attr) !== translated) element.setAttribute(attr, translated);
        });
      });
    }

    function apply(rootNode) {
      if (typeof document === 'undefined') return currentLanguage;
      applying = true;
      try {
        document.documentElement.setAttribute('lang', currentLanguage);
        document.documentElement.setAttribute('dir', 'ltr');
        translateTextNodes(rootNode || document);
        translateAttributes(rootNode || document);
      } finally {
        applying = false;
      }
      return currentLanguage;
    }

    function scheduleApply(rootNode) {
      if (typeof document === 'undefined') return;
      if (applyTimer) clearTimeout(applyTimer);
      applyTimer = setTimeout(() => {
        applyTimer = null;
        apply(rootNode || document);
      }, 0);
    }

    function startObserver() {
      if (typeof document === 'undefined' || typeof MutationObserver === 'undefined' || !document.body) return;
      if (observer) observer.disconnect();
      observer = new MutationObserver(() => {
        if (!applying) scheduleApply(document);
      });
      observer.observe(document.body, {
        childList: true,
        subtree: true,
        characterData: true,
        attributes: true,
        attributeFilter: ['aria-label', 'placeholder', 'title']
      });
    }

    function setCurrentLanguage(language) {
      const normalized = normalizeLanguage(language) || defaultLanguage;
      currentLanguage = supportedLanguages.includes(normalized) ? normalized : defaultLanguage;
      try { localStorage.setItem(storageKey, currentLanguage); } catch (_) { /* ignore storage errors */ }
      if (typeof document !== 'undefined') apply(document);
      if (typeof window !== 'undefined' && typeof window.dispatchEvent === 'function') {
        window.dispatchEvent(new CustomEvent('simpleadmin:language-changed', { detail: { language: currentLanguage } }));
      }
      return currentLanguage;
    }

    function load() {
      try { currentLanguage = normalizeLanguage(localStorage.getItem(storageKey)) || currentLanguage; } catch (_) {}
      apply(document);
      return Promise.resolve(currentLanguage);
    }

    function translateMessage(message) {
      return typeof message === 'string' ? translate(message) : message;
    }

    function wrapNativeDialogs() {
      if (typeof window === 'undefined' || window.__simpleadminI18nDialogsBound) return;
      window.__simpleadminI18nDialogsBound = true;

      const nativeAlert = window.alert;
      if (typeof nativeAlert === 'function') {
        window.alert = function (message) {
          return nativeAlert.call(window, translateMessage(message));
        };
      }

      const nativeConfirm = window.confirm;
      if (typeof nativeConfirm === 'function') {
        window.confirm = function (message) {
          return nativeConfirm.call(window, translateMessage(message));
        };
      }
    }

    function startAutoLoad() {
      if (autoLoadStarted || typeof window === 'undefined' || typeof document === 'undefined') return;
      autoLoadStarted = true;
      wrapNativeDialogs();

      if (typeof window.addEventListener === 'function') {
        window.addEventListener('simpleadmin:vue-mounted', () => scheduleApply(document));
        window.addEventListener('storage', (event) => {
          if (event.key === storageKey && normalizeLanguage(event.newValue) && event.newValue !== currentLanguage) setCurrentLanguage(event.newValue);
        });
      }

      const run = () => {
        load()
          .catch(() => currentLanguage)
          .then(() => {
            startObserver();
            scheduleApply(document);
          });
      };

      if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', run, { once: true });
      } else {
        run();
      }
    }

    startAutoLoad();

    return {
      supportedLanguages,
      normalizeLanguage,
      getCurrentLanguage() { return currentLanguage; },
      t: translate,
      apply,
      load,
      setLanguage(language) {
        return Promise.resolve(setCurrentLanguage(language));
      }
    };
  })();

})(window);
