using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.IO.Compression;
using System.Linq;
using System.Reflection;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading.Tasks;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Markup;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Microsoft.UI.Dispatching;
using Windows.UI;

[assembly: AssemblyTitle("移远高通系列5G模块配置与维护")]
[assembly: AssemblyDescription("Quectel RGMII Toolkit Windows installer")]
[assembly: AssemblyVersion("1.3.1.0")]

namespace SimpleAdminSetup
{
    sealed class Device
    {
        public string Serial;
        public string State;
        public string Model;
        public override string ToString()
        {
            string label = State == "device" ? "已连接" : State == "unauthorized" ? "等待授权" : "离线";
            return (String.IsNullOrEmpty(Model) ? Serial : Model + "  ·  " + Serial) + "  —  " + label;
        }
    }

    sealed class Result
    {
        public int Code;
        public string Output;
    }

    sealed class Controller
    {
        readonly Window window;
        readonly string root;
        readonly DispatcherQueueTimer timer;
        readonly ComboBox devices;
        readonly TextBox log;
        readonly ProgressBar progress;
        readonly bool preview;
        bool busy;
        bool preparing;
        public bool IsBusy => busy;
        public event Action StateChanged;
        public void SetPreparationBusy(bool value) { preparing = value; UpdateActions(); }
        public Task RefreshAfterPreparation() => RefreshDevices();
        bool scanning;
        bool closed;
        bool completed;
        bool reboot;
        bool failedEvent;
        string report;
        string webUrl;
        string mode;
        string resultEvent;
        string failureReason;
        string currentStage;
        string deviceHttpPort;
        readonly Brush blue = new SolidColorBrush(Color.FromArgb(255, 37, 99, 235));
        readonly Brush gray = new SolidColorBrush(Color.FromArgb(255, 113, 128, 150));

        T Find<T>(string name) where T : FrameworkElement { return (T)((FrameworkElement)window.Content).FindName(name); }
        void Text(string name, string value) { Find<TextBlock>(name).Text = value; }

        public Controller(Window window, string root, bool preview)
        {
            this.window = window; this.root = root; this.preview = preview;
            devices = Find<ComboBox>("Devices"); log = Find<TextBox>("Log"); progress = Find<ProgressBar>("Progress");
            timer = window.DispatcherQueue.CreateTimer(); timer.Interval = TimeSpan.FromSeconds(4);
            timer.Tick += async (s, e) => await RefreshDevices();
            Find<Button>("Refresh").Click += async (s, e) => await RefreshDevices();
            Find<Button>("Install").Click += async (s, e) => await Run("install");
            Find<Button>("Diagnose").Click += async (s, e) => await Run("diagnose");
            Find<Button>("OpenWeb").Click += async (s, e) => await Run("web");
            Find<Button>("Report").Click += (s, e) => OpenReport();
            Find<CheckBox>("ChangeHttpPort").Checked += (s, e) => UpdateActions();
            Find<CheckBox>("ChangeHttpPort").Unchecked += (s, e) => UpdateActions();
            foreach (string name in new[] { "ChangeWebCredentials", "ChangeRootPassword" }) {
                Find<CheckBox>(name).Checked += (s, e) => UpdateActions();
                Find<CheckBox>(name).Unchecked += (s, e) => UpdateActions();
            }
            devices.SelectionChanged += (s, e) => UpdateActions();
            window.AppWindow.Closing += (s, e) => {
                if ((busy || preparing) && !preview) {
                    e.Cancel = true;
                    _ = ShowMessage("操作进行中", "当前操作尚未结束，请等待结果。安装过程中关闭工具可能中断文件传输。");
                } else { closed = true; timer.Stop(); }
            };
            ((FrameworkElement)window.Content).Loaded += async (s, e) => { if (!preview) { await RefreshDevices(); timer.Start(); } };
            UpdateActions();
        }

        Task Dispatch(Action action)
        {
            var done = new TaskCompletionSource<bool>();
            if (!window.DispatcherQueue.TryEnqueue(DispatcherQueuePriority.Low, () => {
                try { action(); done.SetResult(true); } catch (Exception e) { done.SetException(e); }
            })) done.SetException(new InvalidOperationException("界面已关闭。"));
            return done.Task;
        }

        bool messageOpen;
        async Task ShowMessage(string title, string message)
        {
            if (messageOpen) return;
            messageOpen = true;
            try {
                await new ContentDialog {
                    XamlRoot = window.Content.XamlRoot,
                    Title = title, Content = message, CloseButtonText = "知道了",
                    RequestedTheme = ((FrameworkElement)window.Content).ActualTheme
                }.ShowAsync();
            } finally { messageOpen = false; }
        }

        static ScrollViewer FindScrollViewer(DependencyObject element)
        {
            if (element is ScrollViewer viewer) return viewer;
            for (int i = 0; i < VisualTreeHelper.GetChildrenCount(element); i++) {
                var found = FindScrollViewer(VisualTreeHelper.GetChild(element, i));
                if (found != null) return found;
            }
            return null;
        }

        void ScrollLogToEnd()
        {
            window.DispatcherQueue.TryEnqueue(() => {
                if (Find<CheckBox>("FollowLog").IsChecked == true) {
                    var viewer = FindScrollViewer(log);
                    viewer?.ChangeView(null, viewer.ScrollableHeight, null, true);
                }
            });
        }
        bool PackageAvailable()
        {
            return File.Exists(Path.Combine(root, "toolkit.ps1")) && File.Exists(Path.Combine(root, "adb.exe"));
        }

        void UpdateActions()
        {
            StateChanged?.Invoke();
            var device = devices.SelectedItem as Device;
            bool ready = !busy && !preparing && device != null && device.State == "device" && (preview || PackageAvailable());
            Find<Button>("Install").IsEnabled = ready && (preview || File.Exists(Path.Combine(root, "development", "SHA256SUMS")));
            Find<Button>("Diagnose").IsEnabled = ready;
            Find<Button>("OpenWeb").IsEnabled = ready;
            Find<Button>("Refresh").IsEnabled = !busy && !preparing && !scanning;
            devices.IsEnabled = !busy && !preparing;
            Find<CheckBox>("ChangeHttpPort").IsEnabled = !busy && !preparing;
            Find<TextBox>("HttpPort").IsEnabled = !busy && !preparing && Find<CheckBox>("ChangeHttpPort").IsChecked == true;
            foreach (string name in new[] { "ChangeWebCredentials", "ChangeRootPassword" }) Find<CheckBox>(name).IsEnabled = !busy && !preparing;
            bool editWeb = !busy && !preparing && Find<CheckBox>("ChangeWebCredentials").IsChecked == true;
            Find<TextBox>("WebUsername").IsEnabled = editWeb;
            Find<PasswordBox>("WebPassword").IsEnabled = editWeb;
            Find<PasswordBox>("RootPassword").IsEnabled = !busy && !preparing && Find<CheckBox>("ChangeRootPassword").IsChecked == true;
            if (busy || preparing) return;
            if (!preview && !PackageAvailable()) {
                Text("DeviceBadge", "安装包不完整");
                Text("DeviceHint", "内置安装资源不完整，请重新下载单文件设备助手。");
            } else if (device == null) {
                Text("DeviceBadge", devices.Items.Count > 0 ? "请选择设备" : "未连接");
                Text("DeviceHint", devices.Items.Count > 0 ? "检测到多个设备，请明确选择本次操作的模块。" : "未发现设备，请检查 USB 连接和 ADB 驱动，然后点击刷新。");
            } else {
                Text("DeviceBadge", device.State == "device" ? "● 已连接" : "需要处理");
                Text("DeviceHint", device.State == "device" ? "ADB 已连接；操作前会检查是否为兼容模块，请勿选择手机或模拟器。" : device.State == "unauthorized" ? "设备尚未授权 ADB，请完成设备端授权后刷新。" : "设备处于离线状态，请重新连接 USB 后刷新。");
            }
            if (!completed) {
                Text("StatusTitle", ready ? "可以开始安装" : "等待连接设备");
                Text("StatusDetail", ready ? "也可以先运行故障诊断，或直接打开已安装的管理页面。" : "连接模块后选择设备，安装按钮会自动启用。");
            }
        }

        static string Quote(string arg)
        {
            // Windows CreateProcess quoting, including embedded quotes and trailing slashes.
            return "\"" + Regex.Replace(arg, @"(\\*)(""|$)", match => new string('\\', match.Groups[1].Length * 2) + (match.Groups[2].Value == "\"" ? "\\\"" : "")) + "\"";
        }

        async Task<Result> Execute(string executable, string arguments, string serial, int timeout, Action<string> output, string standardInput = null)
        {
            return await Task.Run(() => {
                var lines = new StringBuilder();
                var sync = new object();
                var info = new ProcessStartInfo(executable, arguments) {
                    UseShellExecute = false, CreateNoWindow = true, WorkingDirectory = root,
                    RedirectStandardOutput = true, RedirectStandardError = true, RedirectStandardInput = standardInput != null,
                    StandardOutputEncoding = Encoding.UTF8, StandardErrorEncoding = Encoding.UTF8
                };
                if (serial != null) info.EnvironmentVariables["ANDROID_SERIAL"] = serial;
                info.EnvironmentVariables["SIMPLEADMIN_REPORT_DIR"] = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "SimpleAdmin", "Reports");
                using (var process = new Process { StartInfo = info }) {
                    DataReceivedEventHandler receive = (sender, e) => {
                        if (e.Data == null) return;
                        lock (sync) { if (lines.Length < 1024 * 1024) lines.AppendLine(e.Data); }
                        if (output != null && !closed) window.DispatcherQueue.TryEnqueue(() => output(e.Data));
                    };
                    process.OutputDataReceived += receive; process.ErrorDataReceived += receive;
                    process.Start(); process.BeginOutputReadLine(); process.BeginErrorReadLine();
                    if (standardInput != null) {
                        using var input = new StreamWriter(process.StandardInput.BaseStream, new UTF8Encoding(false));
                        input.Write(standardInput);
                    }
                    if (timeout > 0 && !process.WaitForExit(timeout)) {
                        process.Kill(); process.WaitForExit();
                        return new Result { Code = -1, Output = "设备检测超时，请检查 ADB 连接后重试。" };
                    }
                    process.WaitForExit();
                    return new Result { Code = process.ExitCode, Output = lines.ToString() };
                }
            });
        }

        public static List<Device> ParseDevices(string output)
        {
            var found = new List<Device>();
            foreach (var line in output.Split('\n')) {
                var match = Regex.Match(line.Trim(), @"^(\S+)\s+(device|unauthorized|offline)(?:\s|$)");
                if (!match.Success) continue;
                var model = Regex.Match(line, @"\bmodel:(\S+)");
                found.Add(new Device { Serial = match.Groups[1].Value, State = match.Groups[2].Value, Model = model.Success ? model.Groups[1].Value.Replace('_', ' ') : "" });
            }
            return found;
        }

        async Task RefreshDevices()
        {
            if (busy || preparing || scanning || closed || preview) return;
            if (!PackageAvailable()) { UpdateActions(); return; }
            scanning = true; Find<Button>("Refresh").IsEnabled = false;
            try {
                var result = await Execute(Path.Combine(root, "adb.exe"), "devices -l", null, 10000, null);
                if (closed) return;
                string selected = (devices.SelectedItem as Device ?? new Device()).Serial;
                var found = result.Code == 0 ? ParseDevices(result.Output) : new List<Device>();
                devices.ItemsSource = found;
                devices.SelectedItem = found.FirstOrDefault(d => d.Serial == selected);
                if (devices.SelectedItem == null && found.Count == 1) devices.SelectedIndex = 0;
                UpdateActions();
                if (result.Code != 0) Text("DeviceHint", "设备检测失败或超时。请检查 USB 和 ADB 驱动，再点击刷新。");
            } catch (Exception e) {
                Text("DeviceBadge", "检测失败"); Text("DeviceHint", "无法运行 ADB，请完整解压安装包后重试。"); Append(e.Message);
            } finally { scanning = false; Find<Button>("Refresh").IsEnabled = !busy && !preparing; }
        }

        void Append(string line)
        {
            if (String.IsNullOrWhiteSpace(line)) return;
            if (log.Text.Length > 90000) log.Text = log.Text.Substring(log.Text.Length - 60000);
            log.Text += ("[" + DateTime.Now.ToString("HH:mm:ss") + "] " + line + Environment.NewLine);
            if (Find<CheckBox>("FollowLog").IsChecked == true) ScrollLogToEnd();
#if GUI_TEST_HARNESS
            if (line == "STREAMING_PROBE" && busy && Find<Expander>("Details").IsExpanded && log.Visibility == Visibility.Visible)
                File.WriteAllText(Path.Combine(root, "streaming-observed"), "visible before process exit");
#endif
        }

        void Stage(string stage)
        {
            currentStage = stage;
            int active = 0;
            switch (stage) {
                case "device": active = 1; Text("StatusTitle", "正在检查设备连接"); break;
                case "upload": active = 2; Text("StatusTitle", "正在上传安装文件"); break;
                case "install": active = 3; Text("StatusTitle", "正在安装并启动服务"); break;
                case "verify": active = 4; Text("StatusTitle", "正在验证管理页面"); break;
                case "diagnose": active = 2; Text("StatusTitle", "正在收集诊断信息"); break;
            }
            Text("ProgressLabel", mode == "install" ? "第 " + active + " / 4 步" : "正在处理");
            for (int i = 1; i <= 4; i++) Find<TextBlock>("Step" + i).Foreground = i <= active ? blue : gray;
            progress.IsIndeterminate = true;
        }

        void Line(string line)
        {
            if (!line.StartsWith("@@SIMPLEADMIN|", StringComparison.Ordinal)) { Append(line); return; }
            var parts = line.Split(new[] { '|' }, 3);
            if (parts.Length != 3) return;
            switch (parts[1]) {
                case "stage": Stage(parts[2]); break;
                case "report":
                    string candidate = parts[2];
                    if (Path.IsPathRooted(candidate) && String.Equals(Path.GetExtension(candidate), ".txt", StringComparison.OrdinalIgnoreCase)) report = candidate;
                    break;
                case "url":
                    Uri uri;
                    if (Uri.TryCreate(parts[2], UriKind.Absolute, out uri) && uri.Scheme == "http" && uri.Host == "127.0.0.1" && uri.AbsolutePath == "/" && uri.Port > 0) webUrl = uri.AbsoluteUri;
                    break;
                case "reboot": reboot = true; break;
                case "http-port":
                    int parsedPort;
                    if (Int32.TryParse(parts[2], out parsedPort) && parsedPort > 0 && parsedPort <= 65535) deviceHttpPort = parsedPort.ToString();
                    break;
                case "failure": failedEvent = true; break;
                case "error": failureReason = parts[2]; break;
                case "result": resultEvent = parts[2]; break;
            }
        }

        async Task Run(string operation)
        {
            var device = devices.SelectedItem as Device;
            if (busy || preparing || device == null || device.State != "device") return;
            if (preview) return;
            string requestedPort = null;
            if (operation == "install" && Find<CheckBox>("ChangeHttpPort").IsChecked == true) {
                int port;
                string input = Find<TextBox>("HttpPort").Text.Trim();
                if (!Regex.IsMatch(input, @"^[0-9]{1,5}$") || !Int32.TryParse(input, out port) || port < 1 || port > 65535) {
                    Text("StatusTitle", "请检查 HTTP 端口");
                    Text("StatusDetail", "请输入 1–65535 的整数，例如 80 或 8080。尚未修改设备。");
                    Find<TextBox>("HttpPort").Focus(FocusState.Programmatic);
                    return;
                }
                requestedPort = port.ToString();
            }
            string credentials = null;
            if (operation == "install") {
                try {
                    credentials = InstallOptions.Credentials(
                        Find<CheckBox>("ChangeWebCredentials").IsChecked == true, Find<TextBox>("WebUsername").Text,
                        Find<PasswordBox>("WebPassword").Password, Find<CheckBox>("ChangeRootPassword").IsChecked == true,
                        Find<PasswordBox>("RootPassword").Password);
                } catch (ArgumentException error) {
                    Text("StatusTitle", "请检查账号密码"); Text("StatusDetail", error.Message + " 尚未修改设备。"); return;
                }
            }
            busy = true; mode = operation; completed = false; reboot = false; failedEvent = false;
            report = null; webUrl = null; resultEvent = null; deviceHttpPort = null;
            failureReason = null; currentStage = "device";
            Find<TextBlock>("StatusTitle").ClearValue(TextBlock.ForegroundProperty);
            Find<Button>("Report").IsEnabled = false;
            log.Text = ""; timer.Stop(); UpdateActions();
            Find<Expander>("Details").IsExpanded = true;
            Append(operation == "install" ? "开始安装 / 升级，正在连接设备…" : "开始检查设备…");
            await Dispatch(() => log.StartBringIntoView());
            Text("Step1", "① 检查连接");
            Text("Step2", operation == "install" ? "② 上传文件" : operation == "diagnose" ? "② 收集状态" : "② 建立通道");
            Text("Step3", operation == "install" ? "③ 安装程序" : operation == "diagnose" ? "③ 生成报告" : "③ 检查连接");
            Text("Step4", "④ 验证网页");
            Stage("device");
            Text("StatusDetail", operation == "install" ? "请保持 USB 连接，安装期间不要断电。完成后会验证实际网页是否可用。" : operation == "diagnose" ? "只收集服务与网络状态，不读取短信、不执行 AT 指令，也不修改设备设置。" : "正在检查公开登录页面，成功后通过 USB / ADB 打开浏览器。");
            try {
                string powershell = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.System), @"WindowsPowerShell\v1.0\powershell.exe");
                string arguments = "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File " + Quote(Path.Combine(root, "toolkit.ps1"));
                if (operation == "diagnose") arguments += " -DiagnoseOnly";
                if (operation == "web") arguments += " -OpenWebOnly";
                if (requestedPort != null) arguments += " -HttpPort " + requestedPort;
                if (credentials != null) arguments += " -CredentialsFromStdin";
                var result = await Execute(powershell, arguments, device.Serial, 0, Line, credentials);
                // Drain queued output before evaluating the structured final result.
                await Dispatch(() => { });
                bool ok = result.Code == 0 && resultEvent == "0" && !failedEvent;
                if ((operation == "install" || operation == "web") && webUrl == null) ok = false;
                progress.IsIndeterminate = false; progress.Value = ok ? 100 : 0;
                if (ok) {
                    Text("StatusTitle", operation == "install" ? "安装完成，管理页面已就绪" : operation == "diagnose" ? "诊断完成，网页检查通过" : "管理页面已就绪");
                    Text("ProgressLabel", "全部检查通过");
                    Text("StatusDetail", reboot ? "需要手动重启模块以应用网络配置。重启后请重新打开管理页面。" : operation == "install" ? "程序与网页检查通过。点击“打开管理页面”即可使用，账号密码按本次选择保存；未勾选的项目保留原值。" : operation == "diagnose" ? "ADB 通道访问正常。如果模块 IP 仍打不开，请查看报告中的网卡、路由和防火墙信息。" : "已通过 USB / ADB 建立访问通道。保持设备连接，即可在浏览器中使用。" );
                    for (int i = 1; i <= 4; i++) Find<TextBlock>("Step" + i).Foreground = blue;
                    if (deviceHttpPort != null) {
                        Find<TextBlock>("StatusDetail").Text += " 局域网地址：http://模块IP" + (deviceHttpPort == "80" ? "/" : ":" + deviceHttpPort + "/") + "。";
                    }
                    if (operation == "web") {
                        try { Process.Start(new ProcessStartInfo(webUrl) { UseShellExecute = true }); }
                        catch (Exception e) { Append(e.Message); Text("StatusDetail", "浏览器未能自动打开，请复制此地址访问：" + webUrl); }
                    }
                } else {
                    Text("StatusTitle", currentStage == "device" ? "设备检查未通过" : currentStage == "verify" ? "网页检查未通过" : operation == "install" ? "安装未通过检查" : "诊断未完成");
                    Text("ProgressLabel", "需要处理");
                    Text("StatusDetail", String.IsNullOrEmpty(failureReason) ? "请查看实时日志或点击“查看报告”了解失败原因。" : failureReason);
                    Find<TextBlock>("StatusTitle").Foreground = new SolidColorBrush(Color.FromArgb(255, 180, 83, 9));
                }
            } catch (Exception e) {
                Append(e.ToString()); progress.IsIndeterminate = false; progress.Value = 0;
                Text("StatusTitle", "操作未能完成"); Text("ProgressLabel", "需要处理");
                Text("StatusDetail", "请查看下方实时日志中的错误信息，必要时重新下载设备助手。");
            } finally {
                busy = false; completed = true;
                Find<Button>("Report").IsEnabled = report != null && File.Exists(report);
                UpdateActions(); timer.Start();
            }
        }

        void OpenReport()
        {
            if (report == null || !File.Exists(report)) {
                _ = ShowMessage("查看报告", "报告文件尚未生成或已被移动，请重新运行诊断。"); return;
            }
            try { Process.Start(new ProcessStartInfo("notepad.exe", Quote(report)) { UseShellExecute = false }); }
            catch (Exception e) { _ = ShowMessage("查看报告", "无法打开报告：" + e.Message); }
        }

        public void Preview(string state)
        {
            if (state.EndsWith("small", StringComparison.Ordinal)) { window.AppWindow.Resize(new Windows.Graphics.SizeInt32(920, 760)); }
            devices.ItemsSource = new[] { new Device { Serial = "DEMO-DEVICE", State = "device", Model = "RM520N-EU" } };
            devices.SelectedIndex = 0; mode = "install"; UpdateActions();
            if (state == "running") {
                busy = true; UpdateActions(); Stage("install");
                Text("StatusDetail", "正在校验安装文件、更新程序并启动服务。请保持 USB 连接。");
            } else if (state == "success") {
                completed = true; progress.Value = 100;
                Text("StatusTitle", "安装完成，管理页面已就绪"); Text("ProgressLabel", "全部检查通过");
                Text("StatusDetail", "程序与网页检查通过。点击“打开管理页面”即可使用，账号密码按本次选择保存；未勾选的项目保留原值。");
                for (int i = 1; i <= 4; i++) Find<TextBlock>("Step" + i).Foreground = blue;
            } else if (state == "error") {
                completed = true;
                Text("StatusTitle", "安装未通过检查"); Text("ProgressLabel", "需要处理");
                Text("StatusDetail", "请检查 USB 连接，并点击“查看报告”了解原因。可以把报告发给维护者，报告不包含短信或密码。");
                Find<TextBlock>("StatusTitle").Foreground = new SolidColorBrush(Color.FromArgb(255, 180, 83, 9));
                Append("[检查] 安装文件已校验\n[错误] 模拟网页连接超时，请查看诊断报告。");
                Find<Expander>("Details").IsExpanded = true;
            }
        }

#if GUI_TEST_HARNESS
        public async Task TestRun(string operation, string destination)
        {
            while (scanning) await Task.Delay(50);
            await RefreshDevices();
            string testCase = Environment.GetEnvironmentVariable("SIMPLEADMIN_TEST_CASE");
            if (testCase == "custom-port" || testCase == "invalid-input") {
                Find<CheckBox>("ChangeHttpPort").IsChecked = true;
                Find<TextBox>("HttpPort").Text = testCase == "custom-port" ? "8080" : "65536";
            }
            if (testCase == "credentials" || testCase == "invalid-credentials" || testCase == "credential-transfer-failed") {
                Find<CheckBox>("ChangeWebCredentials").IsChecked = true;
                Find<TextBox>("WebUsername").Text = testCase == "invalid-credentials" ? "bad:name" : "owner";
                Find<PasswordBox>("WebPassword").Password = "web-secret:\"$value";
                Find<CheckBox>("ChangeRootPassword").IsChecked = true;
                Find<PasswordBox>("RootPassword").Password = "root-secret:$value";
            }
            await Run(operation);
            File.WriteAllLines(destination, new[] {
                "RESULT=" + resultEvent,
                "FAILED=" + failedEvent,
                "TITLE=" + Find<TextBlock>("StatusTitle").Text,
                "REPORT=" + Find<Button>("Report").IsEnabled,
                "INSTALL=" + Find<Button>("Install").IsEnabled,
                "URL=" + webUrl,
                "BUSY=" + busy
            });
        }
#endif
    }

}
