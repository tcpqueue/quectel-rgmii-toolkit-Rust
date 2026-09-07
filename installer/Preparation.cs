using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace SimpleAdminSetup
{
    sealed class Preparation
    {
        readonly Window window;
        readonly Controller installer;
        readonly bool preview;
        ModuleIdentity identity;
        string identifiedPort;
        bool busy;
#if GUI_TEST_HARNESS
        bool readOnlyTest;
        sealed class ReadOnlyLink : IAtLink {
            readonly IAtLink inner;
            public ReadOnlyLink(string port) { inner = new SerialAtLink(port); }
            public AtReply Send(string command, CancellationToken token) {
                if (!new[] { "AT", "AT+CGMI", "AT+GMM", "AT+GMR", "AT+CGSN", "AT+QCFG=\"usbcfg\"" }.Contains(command))
                    throw new InvalidOperationException("Read-only test refused a non-whitelisted command.");
                return inner.Send(command, token);
            }
            public void Dispose() => inner.Dispose();
        }
        public async Task TestReadOnly(string port, string output) {
            readOnlyTest = true;
            Find<ComboBox>("AtPorts").ItemsSource = new[] { new AtPort { Name = port } };
            Find<ComboBox>("AtPorts").SelectedIndex = 0;
            await Identify();
            if (identity != null) await Unlock();
            File.WriteAllText(output, Find<TextBox>("AtLog").Text + "\n" + Find<InfoBar>("PrepareStatus").Title);
        }
#endif
        IAtLink OpenLink(string port) {
#if GUI_TEST_HARNESS
            if (readOnlyTest) return new ReadOnlyLink(port);
#endif
            return new SerialAtLink(port);
        }
        CancellationTokenSource cancellation;
        T Find<T>(string name) where T : FrameworkElement => (T)((FrameworkElement)window.Content).FindName(name);
        string Port => (Find<ComboBox>("AtPorts").SelectedItem as AtPort)?.Name;

        public Preparation(Window window, Controller installer, bool preview)
        {
            this.window = window; this.installer = installer; this.preview = preview;
            installer.StateChanged += Update;
            Find<Button>("GoPrepare").Click += async (s, e) => { Navigate(true); if (!preview && !busy) await Scan(); };
            Find<Button>("GoAdb").Click += (s, e) => { SelectStep("GoAdb"); Find<FrameworkElement>("AdbSection").StartBringIntoView(new BringIntoViewOptions { AnimationDesired = false, VerticalAlignmentRatio = 0 }); };
            Find<Button>("GoNetwork").Click += (s, e) => { SelectStep("GoNetwork"); Find<FrameworkElement>("NetworkSection").StartBringIntoView(new BringIntoViewOptions { AnimationDesired = false, VerticalAlignmentRatio = 0 }); };
            Find<Button>("GoVerify").Click += (s, e) => { SelectStep("GoVerify"); Find<FrameworkElement>("VerifySection").StartBringIntoView(new BringIntoViewOptions { AnimationDesired = false, VerticalAlignmentRatio = 0 }); };
            Find<ComboBox>("EthernetProfile").SelectionChanged += (s, e) => Update();
            Find<Button>("GoInstall").Click += (s, e) => Navigate(false);
            Find<Button>("ContinueInstall").Click += async (s, e) => { Navigate(false); await installer.RefreshAfterPreparation(); };
            Find<Button>("ScanPorts").Click += async (s, e) => await Scan();
            Find<ComboBox>("AtPorts").SelectionChanged += (s, e) => {
                if (Port != identifiedPort) { identity = null; Find<TextBlock>("ModuleSummary").Text = "已切换串口，请重新识别模块。"; }
                Update();
            };
            Find<Button>("IdentifyModule").Click += async (s, e) => await Identify();
            Find<Button>("UnlockAdb").Click += async (s, e) => await Unlock();
            Find<Button>("RestartModule").Click += async (s, e) => await Change("重启模块", new[] { "AT+CFUN=1,1" }, "当前网络会中断，USB 和 ADB 可能暂时消失。模块恢复后前往安装页刷新连接。", true);
            Find<Button>("EnableEthernet").Click += async (s, e) => await Ethernet(true);
            Find<Button>("DisableEthernet").Click += async (s, e) => await Ethernet(false);
            Find<Button>("FactoryReset").Click += async (s, e) => await Change("恢复模块出厂设置", new[] { "AT+QCFG=\"ResetFactory\"" }, "将恢复模块配置，可能清除 APN、频段、USB 等自定义设置并重启。此操作不能撤销，也不等同于卸载 SimpleAdmin。", true, true);
            Find<Button>("ReadModuleInfo").Click += async (s, e) => await ReadInfo();
            Find<Button>("RunCustomAt").Click += async (s, e) => {
                try {
                    var commands = ParseCustom(Find<TextBox>("CustomAt").Text);
                    await Change("执行自定义 AT", commands, "请核对每一条指令。配置类命令可能中断网络或重启模块；中途停止不会撤销已经执行的指令。");
                } catch (Exception error) { Status("请检查指令", error.Message, InfoBarSeverity.Warning); }
            };
            Find<ComboBox>("AtPreset").SelectionChanged += (s, e) => {
                string commands = (Find<ComboBox>("AtPreset").SelectedItem as ComboBoxItem)?.Tag as string;
                if (!string.IsNullOrEmpty(commands)) Find<TextBox>("CustomAt").Text = commands;
            };
            Find<Button>("StopAt").Click += (s, e) => cancellation?.Cancel();
            Find<Button>("SaveAtPreset").Click += (s, e) => SavePreset();
            Find<Button>("LoadAtPreset").Click += (s, e) => LoadPreset();
            Find<Button>("SaveAtLog").Click += (s, e) => SaveLog();
            Update();
        }
        public void SelectStep(string name) {
            foreach (string step in new[] { "GoPrepare", "GoAdb", "GoNetwork", "GoInstall", "GoVerify" }) Find<Button>(step).Style = step == name ? Find<Button>("UnlockAdb").Style : null;
        }
        void Navigate(bool preparation)
        {
            SelectStep(preparation ? "GoPrepare" : "GoInstall");
            Find<FrameworkElement>(preparation ? "PreparePage" : "InstallPage").StartBringIntoView(new BringIntoViewOptions { AnimationDesired = false, VerticalAlignmentRatio = 0 });
            Update();
        }
        void Status(string title, string message, InfoBarSeverity severity = InfoBarSeverity.Informational)
        {
            var status = Find<InfoBar>("PrepareStatus"); status.Title = title; status.Message = message; status.Severity = severity;
        }
        void Log(string line)
        {
            if (!window.DispatcherQueue.HasThreadAccess) { window.DispatcherQueue.TryEnqueue(() => Log(line)); return; }
            var log = Find<TextBox>("AtLog");
            if (log.Text.Length > 60000) log.Text = log.Text.Substring(log.Text.Length - 40000);
            log.Text += "[" + DateTime.Now.ToString("HH:mm:ss") + "] " + line + "\n";
            log.SelectionStart = log.Text.Length;
        }
        void Update()
        {
            bool available = !busy && !installer.IsBusy;
            bool supported = available && identity?.Supported == true && identity.Imei.Length == 15 && Port == identifiedPort;
            foreach (string name in new[] { "UnlockAdb", "RestartModule", "EnableEthernet", "DisableEthernet", "FactoryReset", "ReadModuleInfo", "RunCustomAt" }) Find<Button>(name).IsEnabled = supported;
            Find<Button>("IdentifyModule").IsEnabled = available && Port != null;
            Find<Button>("ScanPorts").IsEnabled = available;
            Find<ComboBox>("AtPorts").IsEnabled = available;
            foreach (string name in new[] { "EthernetDriver", "EthernetProfile", "AtPreset" }) Find<ComboBox>(name).IsEnabled = available;
            Find<ComboBox>("EthernetDriver").IsEnabled = available && Find<ComboBox>("EthernetProfile").SelectedIndex == 0;
            Find<TextBox>("CustomAt").IsEnabled = available;
            Find<Button>("StopAt").IsEnabled = busy && cancellation != null;
            Find<Button>("SaveAtPreset").IsEnabled = available;
            Find<Button>("LoadAtPreset").IsEnabled = available;
            Find<ProgressBar>("AtProgress").Visibility = busy ? Visibility.Visible : Visibility.Collapsed;
            Find<ProgressBar>("AtProgress").IsIndeterminate = busy;
        }
        async Task Scan()
        {
            if (busy || installer.IsBusy) { Status("请等待当前操作完成", "设备正在使用中。", InfoBarSeverity.Warning); return; }
            busy = true; installer.SetPreparationBusy(true); Update();
            try {
                string previous = Port;
                var ports = await Task.Run(AtPort.List);
                Find<ComboBox>("AtPorts").ItemsSource = ports;
                Find<ComboBox>("AtPorts").SelectedItem = ports.FirstOrDefault(p => p.Name == previous);
                if (ports.Count == 1) Find<ComboBox>("AtPorts").SelectedIndex = 0;
                if (ports.Count == 0) Status("未发现串口", "检查 USB 数据线与模块串口驱动。若已有 ADB，可直接前往安装页。", InfoBarSeverity.Warning);
            } catch (Exception error) { Status("串口列表读取失败", error.Message, InfoBarSeverity.Error); }
            finally { busy = false; installer.SetPreparationBusy(false); Update(); }
        }
        async Task Run(string title, Func<IAtLink, CancellationToken, Task> action, bool verify = true)
        {
            if (preview || busy || installer.IsBusy || Port == null) return;
            if (verify && (identity?.Supported != true || identifiedPort != Port)) { Status("请先识别模块", "仅对已确认的移远高通型号开放配置操作。", InfoBarSeverity.Warning); return; }
            string port = Port;
            var expected = identity;
            busy = true; installer.SetPreparationBusy(true); cancellation = new CancellationTokenSource(); Update();
            Status(title, "请保持连接，正在与模块通信。确认窗口出现前只读取信息。"); Log(title + " · " + port);
            try {
                using var link = await Task.Run(() => OpenLink(port));
                if (verify) await Task.Run(() => Qualcomm.VerifyIdentity(link, expected, cancellation.Token));
                await action(link, cancellation.Token);
            } catch (OperationCanceledException) {
                identity = null; Status("已停止后续操作", "已执行的指令不会自动撤销，请重新识别模块检查状态。", InfoBarSeverity.Warning); Log("用户停止操作。");
            } catch (Exception error) {
                identity = null; Status("操作未完成", error.Message, InfoBarSeverity.Error); Log(error.Message);
            } finally {
                busy = false; cancellation.Dispose(); cancellation = null;
                installer.SetPreparationBusy(false); Update();
                await installer.RefreshAfterPreparation();
            }
        }
        async Task<bool> Confirm(string title, string description, IEnumerable<string> commands, bool destructive = false)
        {
            var content = new StackPanel { Spacing = 12 };
            content.Children.Add(new TextBlock { Text = identity.Model + " · " + identifiedPort });
            content.Children.Add(new TextBlock { Text = description, TextWrapping = TextWrapping.Wrap });
            content.Children.Add(new TextBox { Text = string.Join("\n", commands), IsReadOnly = true, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MaxHeight = 240 });
            var agreement = new CheckBox { Content = "我确认恢复出厂，已备份需要保留的配置", IsChecked = false };
            if (destructive) content.Children.Add(agreement);
            var dialog = new ContentDialog {
                XamlRoot = window.Content.XamlRoot, Title = title, Content = content,
                PrimaryButtonText = destructive ? "恢复出厂" : "确认执行", CloseButtonText = "取消",
                DefaultButton = ContentDialogButton.Close, IsPrimaryButtonEnabled = !destructive,
                RequestedTheme = ((FrameworkElement)window.Content).ActualTheme
            };
            agreement.Checked += (s, e) => dialog.IsPrimaryButtonEnabled = true;
            agreement.Unchecked += (s, e) => dialog.IsPrimaryButtonEnabled = false;
            return await dialog.ShowAsync() == ContentDialogResult.Primary;
        }
        Task Identify() => Run("识别模块", async (link, token) => {
            var found = await Task.Run(() => Qualcomm.Identify(link, token));
            identity = found; identifiedPort = Port;
            Find<TextBlock>("ModuleSummary").Text = "型号：" + found.Model + "\n固件：" + found.Firmware + "\n厂商：" + found.Manufacturer +
                "\n设备标识：" + (found.Imei.Length == 15 ? "•••••••••••" + found.Imei.Substring(11) : "未读到，请重新识别");
            Log("识别结果：" + found.Model + " / " + found.Firmware);
            Status(found.Supported ? "已识别移远高通模块" : "型号暂未适配", found.Supported ? "可以检查 ADB，或展开下方网口配置。SimpleAdmin 安装仍需通过设备系统兼容性检查。" : "保留识别信息，不开放解锁或配置修改。紫光展锐/MTK 型号不能套用此流程。", found.Supported ? InfoBarSeverity.Success : InfoBarSeverity.Warning);
        }, false);
        Task Unlock() => Run("检查 ADB 配置", async (link, token) => {
            string response = await Task.Run(() => Qualcomm.Require(link, "AT+QCFG=\"usbcfg\"", token));
            Log("USB 配置原始返回：" + response);
            var profile = UsbProfile.Parse(response);
            if (profile.AdbEnabled) { Status("ADB 接口已开启", "未修改配置。请前往安装页刷新设备；若仍未出现，请检查驱动或手动重启模块。", InfoBarSeverity.Success); Log("USB 配置倒数第二项为 " + profile.Adb + "，ADB 已开启，跳过解锁。"); return; }
            string command = profile.EnableAdbCommand();
            if (!await Confirm("启用 ADB", "将 USB 配置倒数第二项从 0 改为 2，开启免授权 ADB；保留原 VID、PID 和其他接口。不计算或发送 ADB 密钥。", new[] { command })) { Status("已取消", "未修改模块配置。"); return; }
            bool changed = await Task.Run(() => Qualcomm.ApplyAdb(link, profile, token));
            if (!changed) { Status("ADB 接口已开启", "复查时倒数第二项已为 1 或 2，已跳过密钥和 USB 配置写入。", InfoBarSeverity.Success); Log("ADB 已开启，跳过解锁。"); return; }
            Status("ADB 配置已验证", "USB 配置中的 ADB 已开启。请重启模块后前往安装页刷新；ADB 连接可用后才能安装。", InfoBarSeverity.Success); Log("ADB USB 配置写入并复查通过，未自动重启。");
        });
        Task Ethernet(bool enable)
        {
            string driver = (Find<ComboBox>("EthernetDriver").SelectedItem as ComboBoxItem)?.Tag as string;
            var selection = (NetworkProfile)Find<ComboBox>("EthernetProfile").SelectedIndex;
            string name = selection == NetworkProfile.Pcie ? "PCIe 转网口" : selection == NetworkProfile.Ecm ? "ECM" : "RNDIS";
            return Run("检查 " + name + " 配置", async (link, token) => {
                string response = await Task.Run(() => Qualcomm.Require(link, "AT+QMAP=\"MPDN_RULE\"", token));
                Log("现有 MPDN 规则：" + (response.Length == 0 ? "无" : response));
                bool hasRule = Qualcomm.HasMpdnRuleZero(response);
                string[] commands = Qualcomm.EthernetPlan(driver, selection, enable, hasRule);
                string description = (hasRule ? "检测到 MPDN 规则 0，将先关闭该规则。" : "未配置 MPDN 规则 0，无需关闭。") +
                    (enable ? "使用 QMAPWAC 自动拨号，不创建 MPDN 规则。" : "关闭 QMAPWAC 自动拨号，网络将中断。") +
                    (selection == NetworkProfile.Pcie ? "PCIe 方案按转接板选择网卡型号。" : "USB 方案切换数据接口与网卡模式，不修改 PCIe 网卡或 SIM 检测。") +
                    "配置可能需重启生效，执行后可单独选择重启。";
                if (!await Confirm((enable ? "配置 " : "停用 ") + name, description, commands)) { Status("已取消", "未修改模块配置。"); return; }
                await Task.Run(() => Qualcomm.ExecutePlan(link, commands, token, Log));
                Status("配置指令已被接受", "可以重新读取设备信息核对；需要重启时点击“重启模块”，不会自动重启。", InfoBarSeverity.Success);
            });
        }        Task Change(string title, string[] commands, string description, bool disconnect = false, bool destructive = false) => Run(title, async (link, token) => {
            if (!await Confirm(title, description, commands, destructive)) { Status("已取消", "未执行配置修改。"); return; }
            try { await Task.Run(() => Qualcomm.ExecutePlan(link, commands, token, Log)); }
            catch (Exception error) when (disconnect && (error.GetBaseException() is IOException || error.GetBaseException() is TimeoutException)) {
                identity = null; Status("指令结果未确认", "模块可能正在断开或重启。请等待恢复并重新识别，不会自动重复发送。" + error.Message, InfoBarSeverity.Warning); return;
            }
            if (disconnect) identity = null;
            Status("指令已被模块接受", disconnect ? "请等待模块恢复连接，再前往安装页刷新设备。" : "配置类指令可能需重启生效。可重新读取设备信息核对，或前往安装后台。", InfoBarSeverity.Success);
        });
        Task ReadInfo() => Run("读取设备信息", async (link, token) => {
            var output = Find<StackPanel>("ModuleInfo"); output.Children.Clear();
            foreach (string command in Qualcomm.InfoCommands) {
                var reply = await Task.Run(() => link.Send(command, token));
                string label = Label(command);
                string value = reply.Ok ? Qualcomm.Body(reply) : "固件未提供";
                var card = new StackPanel { Spacing = 6 };
                card.Children.Add(new TextBlock { Text = label, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold });
                card.Children.Add(new TextBlock { Text = value.Length == 0 ? "无返回数据" : value, TextWrapping = TextWrapping.Wrap, IsTextSelectionEnabled = true });
                output.Children.Add(new Border { Child = card, Padding = new Thickness(14), CornerRadius = new CornerRadius(6), Background = (Microsoft.UI.Xaml.Media.Brush)Application.Current.Resources["CardBackgroundFillColorDefaultBrush"] });
                Log(label + "：" + value);
            }
            Status("设备信息读取完成", "未持续轮询；未读取短信，不支持的项目已标记。", InfoBarSeverity.Success);
        });
        static string Label(string command)
        {
            var labels = new[] { "SIM 状态", "模块功能", "温度传感器", "SIM 卡槽", "SIM 检测电平", "SIM 插入检测", "USB 接口", "PCIe 模式", "数据接口", "USB 网卡模式", "以太网驱动", "移动网络 IP", "局域网 IP", "MPDN 规则", "APN 配置", "运营商", "当前网络", "服务小区", "载波聚合", "RSRP", "RSRQ", "SINR", "CSQ", "网络偏好", "5G 模式限制", "LTE 频段", "NSA 频段", "SA 频段", "4G 小区锁定", "5G 小区锁定", "自动拨号" };
            int index = Array.IndexOf(Qualcomm.InfoCommands, command); return index >= 0 ? labels[index] : command;
        }
        public static string[] ParseCustom(string text)
        {
            return Qualcomm.ParseCustom(text);
        }
        static string LocalFolder => Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "SimpleAdmin");
        void SavePreset()
        {
            try { ParseCustom(Find<TextBox>("CustomAt").Text); Directory.CreateDirectory(LocalFolder); File.WriteAllText(Path.Combine(LocalFolder, "quectel-qualcomm-at.txt"), Find<TextBox>("CustomAt").Text); Status("已保存指令", "仅保存在此电脑，再次打开不会自动执行。", InfoBarSeverity.Success); }
            catch (Exception error) { Status("保存失败", error.Message, InfoBarSeverity.Warning); }
        }
        void LoadPreset()
        {
            try { string path = Path.Combine(LocalFolder, "quectel-qualcomm-at.txt"); if (new FileInfo(path).Length > 20000) throw new IOException("指令文件过大。"); string text = File.ReadAllText(path); ParseCustom(text); Find<TextBox>("CustomAt").Text = text; Status("指令已载入", "仅填入编辑框，尚未执行。"); }
            catch (Exception error) { Status("读取失败", error.Message, InfoBarSeverity.Warning); }
        }
        void SaveLog()
        {
            try { string dir = Path.Combine(LocalFolder, "Reports"); Directory.CreateDirectory(dir); string file = Path.Combine(dir, "serial-" + DateTime.Now.ToString("yyyyMMdd-HHmmss") + ".txt"); File.WriteAllText(file, Find<TextBox>("AtLog").Text); Status("操作记录已保存", file + "。记录可能包含网络地址和设备状态，分享前请检查。", InfoBarSeverity.Success); }
            catch (Exception error) { Status("保存失败", error.Message, InfoBarSeverity.Error); }
        }
        public void Preview()
        {
            Navigate(true);
            Find<ComboBox>("AtPorts").ItemsSource = new[] { new AtPort { Name = "COM8", Description = "Quectel USB AT Port（模拟设备）" } };
            Find<ComboBox>("AtPorts").SelectedIndex = 0;
            identity = new ModuleIdentity { Manufacturer = "Quectel", Model = "RM520N-EU", Firmware = "演示固件", Imei = "000000000000000" }; identifiedPort = "COM8";
            Find<TextBlock>("ModuleSummary").Text = "型号：RM520N-EU\n固件：演示固件\n厂商：Quectel\n设备标识：•••••••••••0000";
            Status("已识别移远高通模块", "下一步：检查 ADB 连接，或按需配置网口。", InfoBarSeverity.Success); Update();
        }
    }
}
