using System;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Markup;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.Graphics;
using Windows.Graphics.Imaging;
using Windows.Storage;
using System.Runtime.InteropServices.WindowsRuntime;

[assembly: System.Runtime.Versioning.SupportedOSPlatform("windows10.0.19041.0")]

namespace SimpleAdminSetup
{
    public sealed partial class InstallerView : UserControl
    {
        public InstallerView()
        {
            InitializeComponent();
            SizeChanged += (s, e) => { NavigationColumn.Width = new GridLength(e.NewSize.Width < 950 ? 180 : 236); NavigationPanel.Padding = new Thickness(e.NewSize.Width < 950 ? 12 : 22, 28, e.NewSize.Width < 950 ? 12 : 22, 22); };

        }
    }

    public sealed partial class InstallerApp : Application
    {
        Window window;
        Controller controller;
        Preparation preparation;
        readonly string[] args;
        public InstallerApp(string[] arguments)
        {
            args = arguments;
            InitializeComponent();
            UnhandledException += (s, e) => { Program.ReportError(e.Exception); e.Handled = true; Exit(); };
        }

        protected override void OnLaunched(LaunchActivatedEventArgs e)
        {
            try {
                window = new Window { Title = "移远高通系列5G模块配置与维护" };
                window.Content = new InstallerView();
                var content = (FrameworkElement)window.Content;
                double scale = GetDpiForWindow(WinRT.Interop.WindowNative.GetWindowHandle(window)) / 96.0;
                var area = DisplayArea.GetFromWindowId(window.AppWindow.Id, DisplayAreaFallback.Primary).WorkArea;
                int width = Math.Min(area.Width, (int)(1060 * scale));
                int height = Math.Min(area.Height, (int)(1040 * scale));
                window.AppWindow.Resize(new SizeInt32(width, height));
                window.AppWindow.Move(new PointInt32(area.X + (area.Width - width) / 2, area.Y + (area.Height - height) / 2));
                bool preview = args.Length >= 2 && args[0] == "--preview";
                controller = new Controller(window, AppContext.BaseDirectory, preview);
                preparation = new Preparation(window, controller, preview);
                window.Closed += (s, ev) => Exit();
#if GUI_TEST_HARNESS
                if (args.Length == 3 && args[0] == "--test-at-read-only") {
                    window.AppWindow.Move(new PointInt32(-10000, -10000));
                    window.AppWindow.IsShownInSwitchers = false;
                    content.Loaded += async (s, ev) => {
                        try { await preparation.TestReadOnly(args[1], args[2]); }
                        catch (Exception error) { Program.ReportError(error); }
                        window.Close();
                    };
                }
                if (args.Length == 3 && args[0] == "--test-run") {
                    window.AppWindow.Move(new PointInt32(-10000, -10000));
                    window.AppWindow.IsShownInSwitchers = false;
                    content.Loaded += async (s, ev) => {
                        try { await controller.TestRun(args[1], args[2]); }
                        catch (Exception error) { Program.ReportError(error); }
                        window.Close();
                    };
                }
#endif
                if (preview) {
                    window.AppWindow.Move(new PointInt32(-10000, -10000));
                    window.AppWindow.IsShownInSwitchers = false;
                    content.Loaded += async (s, ev) => {
                        try {
                            string state = args.Length > 2 ? args[2] : "ready";
                            content.RequestedTheme = state.EndsWith("dark", StringComparison.Ordinal) ? ElementTheme.Dark : ElementTheme.Light;
                            controller.Preview(state);
                            if (state.StartsWith("prepare", StringComparison.Ordinal)) preparation.Preview();
                            if (!state.StartsWith("prepare", StringComparison.Ordinal)) { preparation.SelectStep("GoInstall"); ((FrameworkElement)content.FindName("InstallPage")).StartBringIntoView(new BringIntoViewOptions { AnimationDesired = false, VerticalAlignmentRatio = 0 }); }
                            await Task.Delay(600);
                            var bitmap = new RenderTargetBitmap();
                            await bitmap.RenderAsync(content);
                            var pixels = await bitmap.GetPixelsAsync();
                            var file = await StorageFile.GetFileFromPathAsync(CreatePreviewFile(args[1]));
                            using (var output = await file.OpenAsync(FileAccessMode.ReadWrite)) {
                                var encoder = await BitmapEncoder.CreateAsync(BitmapEncoder.PngEncoderId, output);
                                encoder.SetPixelData(BitmapPixelFormat.Bgra8, BitmapAlphaMode.Premultiplied,
                                    (uint)bitmap.PixelWidth, (uint)bitmap.PixelHeight, 96, 96, pixels.ToArray());
                                await encoder.FlushAsync();
                            }
                        } catch (Exception error) { Program.ReportError(error); }
                        window.Close();
                    };
                }
                window.Activate();
            } catch (Exception error) { Program.ReportError(error); Exit(); }
        }

        static string CreatePreviewFile(string path)
        {
            string full = Path.GetFullPath(path);
            Directory.CreateDirectory(Path.GetDirectoryName(full));
            File.WriteAllBytes(full, Array.Empty<byte>());
            return full;
        }
        [DllImport("user32.dll")]
        static extern uint GetDpiForWindow(IntPtr hwnd);
    }

    static class Program
    {
        [DllImport("user32.dll", CharSet = CharSet.Unicode)]
        static extern int MessageBoxW(IntPtr hwnd, string text, string caption, uint type);
        static bool interactive;

        [ComImport, Guid("82BA7092-4C88-427D-A7BC-16DD93FEB67E"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
        interface IRestrictedErrorInfo
        {
            void GetErrorDetails([MarshalAs(UnmanagedType.BStr)] out string description, out int code,
                [MarshalAs(UnmanagedType.BStr)] out string restricted, [MarshalAs(UnmanagedType.BStr)] out string capability);
            void GetReference([MarshalAs(UnmanagedType.BStr)] out string reference);
        }
        [DllImport("combase.dll")]
        static extern int GetRestrictedErrorInfo(out IRestrictedErrorInfo info);

        internal static void ReportError(Exception error)
        {
            Environment.ExitCode = 1;
            string folder = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "SimpleAdmin", "Reports");
            Directory.CreateDirectory(folder);
            string report = Path.Combine(folder, "winui-startup-" + DateTime.Now.ToString("yyyyMMdd-HHmmss") + ".txt");
            string details = error.ToString();
            try {
                if (GetRestrictedErrorInfo(out var info) == 0 && info != null) {
                    info.GetErrorDetails(out var description, out var code, out var restricted, out var capability);
                    details += "\n" + description + "\n" + restricted;
                }
            } catch { }
            File.WriteAllText(report, details);
            if (interactive) MessageBoxW(IntPtr.Zero, "设备助手无法启动：" + error.Message + "\n报告：" + report, "SimpleAdmin", 0x10);
        }

        [STAThread]
        static int Main(string[] args)
        {
            interactive = args.Length == 0;
            try {
                if (args.Length > 0 && args[0] == "--self-test") {
                    var devices = Controller.ParseDevices("List of devices attached\nabc device model:RM520N_EU\nbad unauthorized\noff offline\n");
                    return devices.Count == 3 && devices[0].Model == "RM520N EU" && devices[1].State == "unauthorized" ? 0 : 1;
                }
                if (args.Length > 0 && args[0] == "--verify-payload") {
                    foreach (string name in new[] { "adb.exe", "AdbWinApi.dll", "AdbWinUsbApi.dll", "toolkit.ps1", "development/SHA256SUMS", "development/simpleadmin/simpleadmin-httpd.armv7" })
                        if (!File.Exists(Path.Combine(AppContext.BaseDirectory, name))) throw new FileNotFoundException("内置安装资源缺失", name);
                    var info = new ProcessStartInfo(Path.Combine(AppContext.BaseDirectory, "adb.exe"), "version") { UseShellExecute = false, CreateNoWindow = true, RedirectStandardOutput = true };
                    using (var process = Process.Start(info)) { process.StandardOutput.ReadToEnd(); process.WaitForExit(); return process.ExitCode; }
                }
                WinRT.ComWrappersSupport.InitializeComWrappers();
                Application.Start(p => {
                    SynchronizationContext.SetSynchronizationContext(new DispatcherQueueSynchronizationContext(DispatcherQueue.GetForCurrentThread()));
                    new InstallerApp(args);
                });
                return Environment.ExitCode;
            } catch (Exception error) { ReportError(error); return 1; }
        }
    }
}
