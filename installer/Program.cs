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
using System.Windows;
using System.Windows.Controls;
using System.Windows.Markup;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;

[assembly: AssemblyTitle("SimpleAdmin 设备助手")]
[assembly: AssemblyDescription("Quectel RGMII Toolkit Windows installer")]
[assembly: AssemblyVersion("1.0.0.0")]

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
        readonly DispatcherTimer timer;
        readonly ComboBox devices;
        readonly TextBox log;
        readonly ProgressBar progress;
        readonly bool preview;
        bool busy;
        bool scanning;
        bool closed;
        bool completed;
        bool reboot;
        bool failedEvent;
        string report;
        string webUrl;
        string mode;
        string resultEvent;
        readonly Brush blue = new SolidColorBrush(Color.FromRgb(37, 99, 235));
        readonly Brush gray = new SolidColorBrush(Color.FromRgb(113, 128, 150));

        T Find<T>(string name) where T : FrameworkElement { return (T)window.FindName(name); }
        void Text(string name, string value) { Find<TextBlock>(name).Text = value; }

        public Controller(Window window, string root, bool preview)
        {
            this.window = window; this.root = root; this.preview = preview;
            devices = Find<ComboBox>("Devices"); log = Find<TextBox>("Log"); progress = Find<ProgressBar>("Progress");
            timer = new DispatcherTimer { Interval = TimeSpan.FromSeconds(4) };
            timer.Tick += async (s, e) => await RefreshDevices();
            Find<Button>("Refresh").Click += async (s, e) => await RefreshDevices();
            Find<Button>("Install").Click += async (s, e) => await Run("install");
            Find<Button>("Diagnose").Click += async (s, e) => await Run("diagnose");
            Find<Button>("OpenWeb").Click += async (s, e) => await Run("web");
            Find<Button>("Report").Click += (s, e) => OpenReport();
            devices.SelectionChanged += (s, e) => UpdateActions();
            Find<Expander>("Details").Expanded += (s, e) => { window.Height = Math.Max(window.Height, Math.Min(920, SystemParameters.WorkArea.Height)); };
            window.Closing += (s, e) => {
                if (busy && !preview) {
                    e.Cancel = true;
                    MessageBox.Show(window, "当前操作尚未结束，请等待结果。安装过程中关闭工具可能中断文件传输。", "操作进行中", MessageBoxButton.OK, MessageBoxImage.Information);
                } else { closed = true; timer.Stop(); }
            };
            window.Loaded += async (s, e) => { if (!preview) { await RefreshDevices(); timer.Start(); } };
            UpdateActions();
        }

        bool PackageAvailable()
        {
            return File.Exists(Path.Combine(root, "toolkit.ps1")) && File.Exists(Path.Combine(root, "adb.exe"));
        }

        void UpdateActions()
        {
            var device = devices.SelectedItem as Device;
            bool ready = !busy && device != null && device.State == "device" && (preview || PackageAvailable());
            Find<Button>("Install").IsEnabled = ready && (preview || File.Exists(Path.Combine(root, "development", "SHA256SUMS")));
            Find<Button>("Diagnose").IsEnabled = ready;
            Find<Button>("OpenWeb").IsEnabled = ready;
            Find<Button>("Refresh").IsEnabled = !busy && !scanning;
            devices.IsEnabled = !busy;
            if (busy) return;
            if (!preview && !PackageAvailable()) {
                Text("DeviceBadge", "安装包不完整");
                Text("DeviceHint", "请完整解压安装包，将设备助手与 adb.exe、toolkit.ps1 放在同一目录。");
            } else if (device == null) {
                Text("DeviceBadge", devices.Items.Count > 0 ? "请选择设备" : "未连接");
                Text("DeviceHint", devices.Items.Count > 0 ? "检测到多个设备，请明确选择本次操作的模块。" : "未发现设备，请检查 USB 连接和 ADB 驱动，然后点击刷新。");
            } else {
                Text("DeviceBadge", device.State == "device" ? "● 已连接" : "需要处理");
                Text("DeviceHint", device.State == "device" ? "已锁定所选设备，操作不会发送到其他模块。" : device.State == "unauthorized" ? "设备尚未授权 ADB，请完成设备端授权后刷新。" : "设备处于离线状态，请重新连接 USB 后刷新。");
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

        async Task<Result> Execute(string executable, string arguments, string serial, int timeout, Action<string> output)
        {
            return await Task.Run(() => {
                var lines = new StringBuilder();
                var sync = new object();
                var info = new ProcessStartInfo(executable, arguments) {
                    UseShellExecute = false, CreateNoWindow = true, WorkingDirectory = root,
                    RedirectStandardOutput = true, RedirectStandardError = true,
                    StandardOutputEncoding = Encoding.UTF8, StandardErrorEncoding = Encoding.UTF8
                };
                if (serial != null) info.EnvironmentVariables["ANDROID_SERIAL"] = serial;
                info.EnvironmentVariables["SIMPLEADMIN_REPORT_DIR"] = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "SimpleAdmin", "Reports");
                using (var process = new Process { StartInfo = info }) {
                    DataReceivedEventHandler receive = (sender, e) => {
                        if (e.Data == null) return;
                        lock (sync) { if (lines.Length < 1024 * 1024) lines.AppendLine(e.Data); }
                        if (output != null && !closed) window.Dispatcher.BeginInvoke(new Action(() => output(e.Data)));
                    };
                    process.OutputDataReceived += receive; process.ErrorDataReceived += receive;
                    process.Start(); process.BeginOutputReadLine(); process.BeginErrorReadLine();
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
            if (busy || scanning || closed || preview) return;
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
            } finally { scanning = false; Find<Button>("Refresh").IsEnabled = !busy; }
        }

        void Append(string line)
        {
            if (log.Text.Length > 90000) log.Text = log.Text.Substring(log.Text.Length - 60000);
            log.AppendText(line + Environment.NewLine); log.ScrollToEnd();
        }

        void Stage(string stage)
        {
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
                case "failure": failedEvent = true; break;
                case "result": resultEvent = parts[2]; break;
            }
        }

        async Task Run(string operation)
        {
            var device = devices.SelectedItem as Device;
            if (busy || device == null || device.State != "device") return;
            if (preview) return;
            busy = true; mode = operation; completed = false; reboot = false; failedEvent = false;
            report = null; webUrl = null; resultEvent = null;
            Find<TextBlock>("StatusTitle").Foreground = new SolidColorBrush(Color.FromRgb(23, 35, 57));
            Find<Button>("Report").IsEnabled = false;
            log.Clear(); timer.Stop(); UpdateActions();
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
                var result = await Execute(powershell, arguments, device.Serial, 0, Line);
                // Drain queued output before evaluating the structured final result.
                await window.Dispatcher.InvokeAsync(() => { }, DispatcherPriority.ApplicationIdle);
                bool ok = result.Code == 0 && resultEvent == "0" && !failedEvent;
                if ((operation == "install" || operation == "web") && webUrl == null) ok = false;
                progress.IsIndeterminate = false; progress.Value = ok ? 100 : 0;
                if (ok) {
                    Text("StatusTitle", operation == "install" ? "安装完成，管理页面已就绪" : operation == "diagnose" ? "诊断完成，网页检查通过" : "管理页面已就绪");
                    Text("ProgressLabel", "全部检查通过");
                    Text("StatusDetail", reboot ? "需要手动重启模块以应用网络配置。重启后请重新打开管理页面。" : operation == "install" ? "程序与网页检查通过。点击“打开管理页面”即可使用，升级后的登录密码保持不变。" : operation == "diagnose" ? "ADB 通道访问正常。如果模块 IP 仍打不开，请查看报告中的网卡、路由和防火墙信息。" : "已通过 USB / ADB 建立访问通道。保持设备连接，即可在浏览器中使用。" );
                    for (int i = 1; i <= 4; i++) Find<TextBlock>("Step" + i).Foreground = blue;
                    if (operation == "web") {
                        try { Process.Start(new ProcessStartInfo(webUrl) { UseShellExecute = true }); }
                        catch (Exception e) { Append(e.Message); Text("StatusDetail", "浏览器未能自动打开，请复制此地址访问：" + webUrl); }
                    }
                } else {
                    Text("StatusTitle", operation == "install" ? "安装未通过检查" : "网页检查未通过");
                    Text("ProgressLabel", "需要处理");
                    Text("StatusDetail", "请检查 USB 连接，并点击“查看报告”了解原因。可以把报告发给维护者，报告不包含短信或密码。");
                    Find<TextBlock>("StatusTitle").Foreground = new SolidColorBrush(Color.FromRgb(180, 83, 9));
                }
            } catch (Exception e) {
                Append(e.ToString()); progress.IsIndeterminate = false; progress.Value = 0;
                Text("StatusTitle", "操作未能完成"); Text("ProgressLabel", "需要处理");
                Text("StatusDetail", "请完整解压安装包后重试。展开运行详情可以查看错误信息。");
            } finally {
                busy = false; completed = true;
                Find<Button>("Report").IsEnabled = report != null && File.Exists(report);
                UpdateActions(); timer.Start();
            }
        }

        void OpenReport()
        {
            if (report == null || !File.Exists(report)) {
                MessageBox.Show(window, "报告文件尚未生成或已被移动，请重新运行诊断。", "查看报告"); return;
            }
            try { Process.Start(new ProcessStartInfo("notepad.exe", Quote(report)) { UseShellExecute = false }); }
            catch (Exception e) { MessageBox.Show(window, "无法打开报告：" + e.Message, "查看报告"); }
        }

        public void Preview(string state)
        {
            if (state == "small") { window.Width = 920; window.Height = 700; }
            devices.ItemsSource = new[] { new Device { Serial = "DEMO-DEVICE", State = "device", Model = "RM520N-EU" } };
            devices.SelectedIndex = 0; mode = "install"; UpdateActions();
            if (state == "running") {
                busy = true; UpdateActions(); Stage("install");
                Text("StatusDetail", "正在校验安装文件、更新程序并启动服务。请保持 USB 连接。");
            } else if (state == "success") {
                completed = true; progress.Value = 100;
                Text("StatusTitle", "安装完成，管理页面已就绪"); Text("ProgressLabel", "全部检查通过");
                Text("StatusDetail", "程序与网页检查通过。点击“打开管理页面”即可使用，升级后的登录密码保持不变。");
                for (int i = 1; i <= 4; i++) Find<TextBlock>("Step" + i).Foreground = blue;
            } else if (state == "error") {
                completed = true;
                Text("StatusTitle", "安装未通过检查"); Text("ProgressLabel", "需要处理");
                Text("StatusDetail", "请检查 USB 连接，并点击“查看报告”了解原因。可以把报告发给维护者，报告不包含短信或密码。");
                Find<TextBlock>("StatusTitle").Foreground = new SolidColorBrush(Color.FromRgb(180, 83, 9));
                Append("[检查] 安装文件已校验\n[错误] 模拟网页连接超时，请查看诊断报告。");
                Find<Expander>("Details").IsExpanded = true;
            }
        }

#if GUI_TEST_HARNESS
        public async Task TestRun(string operation, string destination)
        {
            while (scanning) await Task.Delay(50);
            await RefreshDevices();
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

    static class Program
    {
        static string ExtractPayload()
        {
            string destination = Path.Combine(Path.GetTempPath(), "SimpleAdmin-" + Guid.NewGuid().ToString("N"));
            Directory.CreateDirectory(destination);
            using (var resource = Assembly.GetExecutingAssembly().GetManifestResourceStream("payload.zip")) {
                if (resource == null) throw new InvalidDataException("内置安装资源缺失，请重新下载设备助手。");
                using (var archive = new ZipArchive(resource, ZipArchiveMode.Read)) {
                    foreach (var entry in archive.Entries) {
                        string target = Path.GetFullPath(Path.Combine(destination, entry.FullName));
                        if (!target.StartsWith(destination + Path.DirectorySeparatorChar, StringComparison.OrdinalIgnoreCase)) throw new InvalidDataException("安装资源路径无效。");
                        if (String.IsNullOrEmpty(entry.Name)) { Directory.CreateDirectory(target); continue; }
                        Directory.CreateDirectory(Path.GetDirectoryName(target));
                        using (var input = entry.Open()) using (var output = File.Create(target)) input.CopyTo(output);
                    }
                }
            }
            foreach (string file in new[] { "adb.exe", "AdbWinApi.dll", "AdbWinUsbApi.dll", "toolkit.ps1", "development/install_simpleadmin_rust.sh", "development/SHA256SUMS", "development/simpleadmin/simpleadmin-httpd.armv7", "development/simpleadmin/www/index.html" })
                if (!File.Exists(Path.Combine(destination, file))) throw new InvalidDataException("内置安装资源不完整：" + file);
            return destination;
        }

        [STAThread]
        static int Main(string[] args)
        {
            string extracted = null;
            try {
                if (args.Length > 0 && args[0] == "--verify-payload") {
                    extracted = ExtractPayload();
                    var check = new ProcessStartInfo(Path.Combine(extracted, "adb.exe"), "version") { UseShellExecute = false, CreateNoWindow = true, RedirectStandardOutput = true };
                    using (var process = Process.Start(check)) { process.StandardOutput.ReadToEnd(); process.WaitForExit(); return process.ExitCode; }
                }
                if (args.Length > 0 && args[0] == "--self-test") {
                    var list = Controller.ParseDevices("List of devices attached\nabc device product:foo model:RM520N_EU transport_id:1\nbad unauthorized\noff offline\nnoise\n");
                    if (list.Count != 3 || list[0].Model != "RM520N EU" || list[1].State != "unauthorized") return 1;
                    return 0;
                }
                var app = new Application();
                Window window;
                using (var stream = Assembly.GetExecutingAssembly().GetManifestResourceStream("MainWindow.xaml")) window = (Window)XamlReader.Load(stream);
                bool preview = args.Length >= 2 && args[0] == "--preview";
                string runtimeRoot = AppDomain.CurrentDomain.BaseDirectory;
#if !GUI_TEST_HARNESS
                if (!preview) runtimeRoot = extracted = ExtractPayload();
#endif
                var controller = new Controller(window, runtimeRoot, preview);
#if GUI_TEST_HARNESS
                if (args.Length == 3 && args[0] == "--test-run") {
                    window.ShowActivated = false; window.ShowInTaskbar = false;
                    window.Left = -10000; window.Top = -10000; window.WindowStartupLocation = WindowStartupLocation.Manual;
                    window.Loaded += async (s, e) => {
                        await controller.TestRun(args[1], args[2]);
                        app.Shutdown();
                    };
                }
#endif
                if (preview) {
                    window.ShowActivated = false; window.ShowInTaskbar = false;
                    window.Left = -10000; window.Top = -10000; window.WindowStartupLocation = WindowStartupLocation.Manual;
                    window.Loaded += (s, e) => window.Dispatcher.BeginInvoke(new Action(() => {
                        controller.Preview(args.Length > 2 ? args[2] : "ready");
                        window.UpdateLayout();
                        var content = (FrameworkElement)window.Content;
                        var bitmap = new RenderTargetBitmap((int)content.ActualWidth, (int)content.ActualHeight, 96, 96, PixelFormats.Pbgra32);
                        bitmap.Render(content);
                        var encoder = new PngBitmapEncoder(); encoder.Frames.Add(BitmapFrame.Create(bitmap));
                        using (var file = File.Create(args[1])) encoder.Save(file);
                        app.Shutdown();
                    }), DispatcherPriority.ApplicationIdle);
                }
                return app.Run(window);
            } catch (Exception e) {
                if (args.Length > 0) { File.WriteAllText(Path.Combine(Path.GetTempPath(), "simpleadmin-setup-error.txt"), e.ToString()); return 1; }
                MessageBox.Show("设备助手无法启动：" + e.Message + "\n请重新下载完整的单文件设备助手。", "SimpleAdmin", MessageBoxButton.OK, MessageBoxImage.Error);
                return 1;
            } finally {
                // A shared ADB server may still hold its executable; never kill it to clean up.
                if (extracted != null) { try { Directory.Delete(extracted, true); } catch (IOException) { } catch (UnauthorizedAccessException) { } }
            }
        }
    }
}
