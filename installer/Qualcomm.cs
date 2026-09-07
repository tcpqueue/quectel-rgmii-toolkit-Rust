using System;
using System.Collections.Generic;
using System.IO;
using System.Globalization;
using System.IO.Ports;
using System.Linq;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading;
using Microsoft.Win32;

namespace SimpleAdminSetup
{
    sealed class AtReply
    {
        public string Text;
        public bool Ok;
        public static bool IsTerminal(string line) => line == "OK" || line == "ERROR" ||
            line.StartsWith("+CME ERROR:", StringComparison.Ordinal) || line.StartsWith("+CMS ERROR:", StringComparison.Ordinal);
    }

    interface IAtLink : IDisposable { AtReply Send(string command, CancellationToken cancel); }

    sealed class SerialAtLink : IAtLink
    {
        readonly SerialPort serial;
        bool failed;
        public SerialAtLink(string port)
        {
            if (!Regex.IsMatch(port, @"^COM[1-9][0-9]*$")) throw new ArgumentException("串口名称无效。");
            serial = new SerialPort(port, 115200, Parity.None, 8, StopBits.One) {
                ReadTimeout = 150, WriteTimeout = 2000, DtrEnable = true, RtsEnable = true,
                Handshake = Handshake.None, Encoding = Encoding.ASCII
            };
            try {
                serial.Open();
                // USB AT drivers may release queued replies when DTR/RTS are asserted.
                // Wait for quiet before the first transaction; never pair an old OK with a new query.
                var settle = System.Diagnostics.Stopwatch.StartNew();
                long lastData = 0;
                while (settle.ElapsedMilliseconds < 2000) {
                    if (serial.ReadExisting().Length > 0) lastData = settle.ElapsedMilliseconds;
                    if (settle.ElapsedMilliseconds - lastData >= 200) break;
                    Thread.Sleep(20);
                }
                if (settle.ElapsedMilliseconds - lastData < 200) throw new IOException("串口持续有积压响应，请关闭其他串口工具后重试。");
                serial.DiscardInBuffer();
            } catch { serial.Dispose(); throw; }
        }
        public AtReply Send(string command, CancellationToken cancel)
        {
            Qualcomm.ValidateCommand(command);
            if (failed) throw new IOException("上一条指令未完成，请重新识别模块后再试。");
            cancel.ThrowIfCancellationRequested();
            try {
                serial.DiscardInBuffer();
                serial.Write(command + "\r");
                var timer = System.Diagnostics.Stopwatch.StartNew();
                var output = new StringBuilder();
                var line = new StringBuilder();
                while (timer.Elapsed < TimeSpan.FromSeconds(8)) {
                    cancel.ThrowIfCancellationRequested();
                    string chunk = serial.ReadExisting();
                    foreach (char c in chunk) {
                        if (c == '\r' || c == '\n') {
                            string value = line.ToString().Trim(); line.Clear();
                            if (value.Length == 0 || value.Equals(command, StringComparison.OrdinalIgnoreCase)) continue;
                            output.AppendLine(value);
                            if (AtReply.IsTerminal(value)) return new AtReply { Ok = value == "OK", Text = output.ToString() };
                        } else line.Append(c);
                        if (output.Length + line.Length > 32768) throw new IOException("串口响应过长，已停止接收。");
                    }
                    if (cancel.WaitHandle.WaitOne(20)) cancel.ThrowIfCancellationRequested();
                }
                throw new TimeoutException("模块未在 8 秒内返回完整结果。请确认选择的是 AT 串口，并关闭其他占用串口的软件。");
            } catch { failed = true; serial.Close(); throw; }
        }
        public void Dispose() => serial.Dispose();
    }

    sealed class AtPort
    {
        public string Name;
        public string Description;
        public override string ToString() => Name + (string.IsNullOrEmpty(Description) ? "" : " · " + Description);
        public static List<AtPort> List()
        {
            var labels = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
            using (var usb = Registry.LocalMachine.OpenSubKey(@"SYSTEM\CurrentControlSet\Enum\USB")) {
                if (usb != null) foreach (var hardware in usb.GetSubKeyNames().Where(n => n.StartsWith("VID_2C7C", StringComparison.OrdinalIgnoreCase))) {
                    using (var group = usb.OpenSubKey(hardware)) {
                        if (group == null) continue;
                        foreach (var instance in group.GetSubKeyNames()) {
                            using (var device = group.OpenSubKey(instance))
                            using (var parameters = device?.OpenSubKey("Device Parameters")) {
                                string name = parameters?.GetValue("PortName") as string;
                                if (name != null) labels[name] = device.GetValue("FriendlyName") as string ?? "Quectel USB 串口";
                            }
                        }
                    }
                }
            }
            return SerialPort.GetPortNames().Where(n => Regex.IsMatch(n, @"^COM[1-9][0-9]*$"))
                .OrderBy(n => int.Parse(n.Substring(3))).Select(n => new AtPort { Name = n, Description = labels.TryGetValue(n, out var label) ? label : "" }).ToList();
        }
    }

    sealed class ModuleIdentity
    {
        public string Manufacturer;
        public string Model;
        public string Firmware;
        public string Imei;
        public bool Supported => Manufacturer.Contains("QUECTEL", StringComparison.OrdinalIgnoreCase) &&
            Regex.IsMatch(Model, @"^(RM500Q|RM502Q|RM520N|RM521F|RG500Q|RG502Q|RG520N|RG520F|RG521F)([-\s]|$)", RegexOptions.IgnoreCase);
        public bool SameDevice(ModuleIdentity other) => other != null && Manufacturer == other.Manufacturer && Model == other.Model && Imei == other.Imei && !string.IsNullOrEmpty(Imei);
    }

    sealed class UsbProfile
    {
        public string[] Fields;
        public int Adb => int.Parse(Fields[Fields.Length - 2], CultureInfo.InvariantCulture);
        public bool AdbEnabled => Adb == 1 || Adb == 2;
        public string EnableAdbCommand()
        {
            var copy = (string[])Fields.Clone(); copy[copy.Length - 2] = "2";
            return "AT+QCFG=\"usbcfg\"," + string.Join(",", copy);
        }
        static bool UsbId(string value, out uint id)
        {
            bool hex = value.StartsWith("0x", StringComparison.OrdinalIgnoreCase);
            string digits = hex ? value.Substring(2) : value;
            id = 0;
            return Regex.IsMatch(digits, hex ? @"\A[0-9A-Fa-f]{1,4}\z" : @"\A[0-9]{1,5}\z") &&
                uint.TryParse(digits, hex ? NumberStyles.AllowHexSpecifier : NumberStyles.None, CultureInfo.InvariantCulture, out id) && id <= 65535;
        }
        public static UsbProfile Parse(string response)
        {
            var matches = Regex.Matches(response, @"(?:\A|[\r\n])\s*\+QCFG:\s*""usbcfg""\s*,([^\r\n]+)", RegexOptions.IgnoreCase);
            if (matches.Count != 1)
                throw new InvalidOperationException("未收到唯一完整的 USB 配置响应，请查看上方原始返回值后重新检查。未发送解锁指令。");
            var fields = matches[0].Groups[1].Value.Split(',').Select(s => s.Trim()).ToArray();
            if (fields.Length != 9)
                throw new InvalidOperationException("USB 配置返回 " + fields.Length + " 个字段，当前需要 VID、PID 和 7 个接口参数；未修改配置。");
            if (!UsbId(fields[0], out uint vid) || vid != 0x2C7C)
                throw new InvalidOperationException("USB VID 不是移远 2C7C，未修改配置。");
            if (!UsbId(fields[1], out _))
                throw new InvalidOperationException("USB PID 不是有效的 16 位编号，未修改配置。");
            if (fields.Skip(2).Any(s => !Regex.IsMatch(s, @"\A[012]\z")))
                throw new InvalidOperationException("USB 接口参数不在 0/1/2 范围内，未修改配置。");
            return new UsbProfile { Fields = fields };
        }
    }

    enum NetworkProfile { Pcie, Ecm, Rndis }
    static class Qualcomm
    {
        public static readonly string[] InfoCommands = {
            "AT+CPIN?", "AT+CFUN?", "AT+QTEMP", "AT+QUIMSLOT?", "AT+QSIMDET?", "AT+QSIMSTAT?",
            "AT+QCFG=\"usbcfg\"", "AT+QCFG=\"pcie/mode\"", "AT+QCFG=\"data_interface\"", "AT+QCFG=\"usbnet\"",
            "AT+QETH=\"eth_driver\"", "AT+QMAP=\"WWAN\"", "AT+QMAP=\"LANIP\"", "AT+QMAP=\"MPDN_rule\"",
            "AT+CGDCONT?", "AT+QSPN", "AT+QNWINFO", "AT+QENG=\"servingcell\"", "AT+QCAINFO",
            "AT+QRSRP", "AT+QRSRQ", "AT+QSINR", "AT+CSQ", "AT+QNWPREFCFG=\"mode_pref\"",
            "AT+QNWPREFCFG=\"nr5g_disable_mode\"", "AT+QNWPREFCFG=\"lte_band\"",
            "AT+QNWPREFCFG=\"nsa_nr5g_band\"", "AT+QNWPREFCFG=\"nr5g_band\"",
            "AT+QNWLOCK=\"common/4g\"", "AT+QNWLOCK=\"common/5g\"", "AT+QMAPWAC?"
        };
        public static void ValidateCommand(string command)
        {
            if (string.IsNullOrWhiteSpace(command) || command.Length > 512 || !command.StartsWith("AT", StringComparison.OrdinalIgnoreCase) ||
                command.Any(c => c < 32 || c > 126) || command.Contains(';'))
                throw new ArgumentException("每行只填一条 AT 指令（最多 512 个字符），不使用分号拼接或控制字符。");
        }
        public static string Body(AtReply reply) => string.Join("\n", reply.Text.Split('\n').Select(s => s.Trim())
            .Where(s => s.Length > 0 && !AtReply.IsTerminal(s) && !s.StartsWith("AT", StringComparison.OrdinalIgnoreCase)));
        public static string Require(IAtLink link, string command, CancellationToken cancel)
        {
            var reply = link.Send(command, cancel);
            if (!reply.Ok) throw new InvalidOperationException(command + " 返回错误：" + reply.Text.Trim());
            return Body(reply);
        }
        public static ModuleIdentity Identify(IAtLink link, CancellationToken cancel)
        {
            Require(link, "AT", cancel);
            var manufacturer = Require(link, "AT+CGMI", cancel);
            var model = Require(link, "AT+GMM", cancel).Replace("+GMM:", "").Trim().Trim('"');
            var firmware = Require(link, "AT+GMR", cancel);
            var imei = Regex.Match(Require(link, "AT+CGSN", cancel), @"(?<!\d)\d{15}(?!\d)").Value;
            return new ModuleIdentity { Manufacturer = manufacturer, Model = model, Firmware = firmware, Imei = imei };
        }
        public static void VerifyIdentity(IAtLink link, ModuleIdentity expected, CancellationToken cancel)
        {
            var current = Identify(link, cancel);
            if (!current.Supported || !current.SameDevice(expected))
                throw new InvalidOperationException("串口上的模块与刚才识别的不一致或型号未适配，请重新识别；未执行配置修改。");
        }
        public static bool ApplyAdb(IAtLink link, UsbProfile profile, CancellationToken token)
        {
            var latest = UsbProfile.Parse(Require(link, "AT+QCFG=\"usbcfg\"", token));
            if (latest.AdbEnabled) return false;
            if (!latest.Fields.SequenceEqual(profile.Fields))
                throw new InvalidOperationException("USB 配置已变化，请重新检查；未修改配置。");
            string command = profile.EnableAdbCommand();
            Require(link, command, token);
            var verified = UsbProfile.Parse(Require(link, "AT+QCFG=\"usbcfg\"", token));
            if (verified.Adb != 2 || verified.EnableAdbCommand() != command)
                throw new InvalidOperationException("USB 配置复查不一致，请重新识别模块；不要重复发送指令。");
            return true;
        }        public static string[] ParseCustom(string text)
        {
            var commands = text.Split(new[] { '\r', '\n' }, StringSplitOptions.RemoveEmptyEntries).Select(s => s.Trim()).Where(s => s.Length > 0).ToArray();
            if (commands.Length == 0 || commands.Length > 32) throw new ArgumentException("请输入 1–32 条 AT 指令，每行一条。");
            foreach (string command in commands) {
                ValidateCommand(command);
                if (Regex.IsMatch(command, @"^AT\+(?:CMGS|CMSS|CMGW|CMGD|CMGC|QCMGS|QCMGD|QADBKEY)\b", RegexOptions.IgnoreCase))
                    throw new ArgumentException("此工具不提供短信发送/删除或 ADB 密钥操作，请使用上方 ADB 接口配置。");
            }
            return commands;
        }
        public static bool HasMpdnRuleZero(string response)
        {
            var matches = Regex.Matches(response, @"(?:\A|[\r\n])\s*\+QMAP:\s*""MPDN_RULE""\s*,\s*0\s*,([^\r\n]+)", RegexOptions.IgnoreCase);
            if (matches.Count == 0) {
                if (response.Trim().Length == 0 || response.Contains("+QMAP:", StringComparison.OrdinalIgnoreCase)) return false;
                throw new InvalidOperationException("未能识别 MPDN 规则查询结果，未修改配置。");
            }
            if (matches.Count != 1) throw new InvalidOperationException("MPDN 规则 0 返回多次，未修改配置。");
            var fields = matches[0].Groups[1].Value.Split(',').Select(s => s.Trim()).ToArray();
            if (fields.Length < 4 || !Regex.IsMatch(fields[0], @"\A[0-9]+\z"))
                throw new InvalidOperationException("MPDN 规则 0 格式无效，未修改配置。");
            return fields[0] != "0";
        }
        public static string[] EthernetPlan(string driver, NetworkProfile profile, bool enable, bool hasMpdnRuleZero)
        {
            if (!Enum.IsDefined(typeof(NetworkProfile), profile)) throw new ArgumentException("请选择连接方案。");
            var commands = new List<string>();
            if (hasMpdnRuleZero) commands.Add("AT+QMAP=\"MPDN_RULE\",0");
            if (profile == NetworkProfile.Pcie) {
                if (driver != "r8125" && driver != "r8168") throw new ArgumentException("请选择网卡型号。");
                if (enable) {
                    commands.Add("AT+QCFG=\"data_interface\",1,0");
                    commands.Add("AT+QCFG=\"pcie/mode\",1");
                }
                commands.Add("AT+QETH=\"eth_driver\",\"" + driver + "\"," + (enable ? 1 : 0));
            } else if (enable) {
                commands.Add("AT+QCFG=\"data_interface\",0,0");
                commands.Add("AT+QCFG=\"usbnet\"," + (profile == NetworkProfile.Ecm ? 1 : 3));
            }
            commands.Add("AT+QMAPWAC=" + (enable ? 1 : 0));
            return commands.ToArray();
        }        public static void ExecutePlan(IAtLink link, IEnumerable<string> commands, CancellationToken cancel, Action<string> progress)
        {
            int done = 0;
            foreach (string command in commands) {
                cancel.ThrowIfCancellationRequested();
                progress("发送：" + command);
                try { string result = Require(link, command, cancel); if (result.Length > 0) progress(result); }
                catch (Exception e) when (!(e is OperationCanceledException)) {
                    throw new InvalidOperationException("已完成 " + done + " 条，后续已停止。已执行的配置不会自动回滚。" + e.Message, e);
                }
                done++; progress("已确认：" + command);
            }
        }
    }
}
