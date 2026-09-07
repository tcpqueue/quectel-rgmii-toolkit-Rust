using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading;
using SimpleAdminSetup;

static class Tests
{
    static int passed;
    static readonly CancellationToken Token = CancellationToken.None;
    const string Query = "AT+QCFG=\"usbcfg\"";
    const string Closed = "+QCFG: \"usbcfg\",0x2C7C,0x0801,1,0,1,1,0,0,1";
    static string Profile(int adb) => Closed.Substring(0, Closed.Length - 3) + adb + ",1";
    static void Check(bool condition) { if (!condition) throw new Exception("Assertion failed"); }
    static void Case(string name, Action test) { test(); passed++; Console.WriteLine("PASS " + name); }
    static void Reject<T>(Action test) where T : Exception {
        try { test(); } catch (T) { return; } throw new Exception("Expected " + typeof(T).Name);
    }
    sealed class Link : IAtLink {
        readonly Queue<(string command, string response, bool ok)> pending = new();
        public Link Add(string command, string response = "", bool ok = true) { pending.Enqueue((command,response,ok)); return this; }
        public AtReply Send(string command, CancellationToken token) {
            token.ThrowIfCancellationRequested();
            if (pending.Count == 0) throw new Exception("Unexpected command: " + command);
            var next = pending.Dequeue(); Check(command == next.command);
            return new AtReply { Ok = next.ok, Text = next.response + (next.ok ? "\r\nOK\r\n" : "\r\nERROR\r\n") };
        }
        public void Done() => Check(pending.Count == 0);
        public void Dispose() {}
    }
    static Link Identity(string imei = "123456789012345") => new Link().Add("AT").Add("AT+CGMI","Quectel").Add("AT+GMM","RM520N-EU").Add("AT+GMR","test").Add("AT+CGSN",imei);
    public static void Main() {
        Case("md5-crypt OpenSSL independent vectors", () => {
            Check(Qualcomm.UnlockKey("12345678") == "0jXKXQwSwMxYoegx0S.I.1".Substring(0,15));
            Check(Qualcomm.UnlockKey("1234") == "DYRcQGIywMfppi0mNc/qc1".Substring(0,15));
            Check(Qualcomm.UnlockKey("0") == "Uznjfx7C6nIVY6.IT8gJV1".Substring(0,15));
        });
        Case("invalid challenge rejected", () => { foreach(var value in new[]{"", "123456789", "1x","12\n"}) Reject<ArgumentException>(()=>Qualcomm.UnlockKey(value)); });
        Case("USB field preservation", () => {
            var original=UsbProfile.Parse(Closed); Check(!original.AdbEnabled);
            Check(original.EnableAdbCommand()=="AT+QCFG=\"usbcfg\",0x2C7C,0x0801,1,0,1,1,0,1,1");
            Check(UsbProfile.Parse(Closed.Replace("0801","0800")).Fields[1]=="0x0800");
        });
        foreach (int adb in new[]{1,2}) Case("ADB " + adb + " skips every write and key query", () => {
            var link=new Link().Add(Query,Profile(adb));
            Check(UsbProfile.Parse(Profile(adb)).AdbEnabled);
            Check(!Qualcomm.ApplyAdb(link,UsbProfile.Parse(Profile(adb)),"1234",Token)); link.Done();
        });
        Case("ADB enabled while dialog open skips unlock", () => {
            var link=new Link().Add(Query,Profile(2));
            Check(!Qualcomm.ApplyAdb(link,UsbProfile.Parse(Closed),"1234",Token)); link.Done();
        });
        Case("unknown USB profiles rejected", () => {
            foreach(var response in new[]{Closed.Replace("0801","0900"), Closed.Replace("2C7C","1234"),Closed+",1",Profile(3),"ERROR"})
                Reject<InvalidOperationException>(()=>UsbProfile.Parse(response));
        });
        string key="AT+QADBKEY=\"" + Qualcomm.UnlockKey("1234") + "\"";
        string setter=UsbProfile.Parse(Closed).EnableAdbCommand();
        Case("successful unlock checks exact readback", () => {
            var link=new Link().Add(Query,Closed).Add("AT+QADBKEY?","+QADBKEY: 1234").Add(key).Add(setter).Add(Query,Profile(1));
            Check(Qualcomm.ApplyAdb(link,UsbProfile.Parse(Closed),"1234",Token)); link.Done();
        });
        Case("rejected key never changes USB", () => {
            var link=new Link().Add(Query,Closed).Add("AT+QADBKEY?","+QADBKEY: 1234").Add(key,"",false);
            Reject<InvalidOperationException>(()=>Qualcomm.ApplyAdb(link,UsbProfile.Parse(Closed),"1234",Token)); link.Done();
        });
        Case("changed challenge never sends key", () => {
            var link=new Link().Add(Query,Closed).Add("AT+QADBKEY?","+QADBKEY: 5678");
            Reject<InvalidOperationException>(()=>Qualcomm.ApplyAdb(link,UsbProfile.Parse(Closed),"1234",Token)); link.Done();
        });
        Case("changed USB never sends key", () => {
            var link=new Link().Add(Query,Closed.Replace("0801","0800"));
            Reject<InvalidOperationException>(()=>Qualcomm.ApplyAdb(link,UsbProfile.Parse(Closed),"1234",Token)); link.Done();
        });
        Case("rejected USB setter stops before readback", () => {
            var link=new Link().Add(Query,Closed).Add("AT+QADBKEY?","+QADBKEY: 1234").Add(key).Add(setter,"",false);
            Reject<InvalidOperationException>(()=>Qualcomm.ApplyAdb(link,UsbProfile.Parse(Closed),"1234",Token)); link.Done();
        });
        Case("USB readback mismatch is failure", () => {
            var link=new Link().Add(Query,Closed).Add("AT+QADBKEY?","+QADBKEY: 1234").Add(key).Add(setter).Add(Query,Closed);
            Reject<InvalidOperationException>(()=>Qualcomm.ApplyAdb(link,UsbProfile.Parse(Closed),"1234",Token)); link.Done();
        });
        Case("canceled unlock sends nothing", () => Reject<OperationCanceledException>(()=>Qualcomm.ApplyAdb(new Link(),UsbProfile.Parse(Closed),"1234",new CancellationToken(true))));
        Case("device identity verified", () => {
            var link=Identity(); var identity=Qualcomm.Identify(link,Token); Check(identity.Supported); link.Done();
            var other=Identity("999999999999999");
            Reject<InvalidOperationException>(()=>Qualcomm.VerifyIdentity(other,identity,Token)); other.Done();
        });
        Case("unsupported chipsets remain locked", () => {
            foreach(var model in new[]{"RM500U-CN","RG650V","FM350","RM520NXYZ","RM500QTEST"})
                Check(!new ModuleIdentity{Manufacturer="Quectel",Model=model}.Supported);
        });
        Case("batch stops at first rejection", () => {
            var link=new Link().Add("AT").Add("AT+CFUN?","",false);
            Reject<InvalidOperationException>(()=>Qualcomm.ExecutePlan(link,new[]{"AT","AT+CFUN?","AT+QNWINFO"},Token,_=>{})); link.Done();
        });
        Case("custom SMS and key commands blocked", () => {
            foreach(var command in new[]{"AT+CMGS=\"123\"","at+cmgd=1","AT+CMSS=1","AT+QADBKEY?","AT+CMGW","AT+CMGC","AT+QCMGS"})
                Reject<ArgumentException>(()=>Qualcomm.ParseCustom(command));
        });
        Case("custom command boundaries", () => {
            Check(Qualcomm.ParseCustom("AT\r\nAT+QTEMP\n").Length==2);
            foreach(var command in new[]{"","AT;AT","AT\t+CSQ","AT\u001a",new string('A',513),string.Join("\n",Enumerable.Repeat("AT",33))})
                Reject<ArgumentException>(()=>Qualcomm.ParseCustom(command));
        });
        Case("NAT plan matches intended four commands", () => {
            Check(Qualcomm.EthernetPlan("r8125",1,true,false).SequenceEqual(new[]{"AT+QCFG=\"data_interface\",1,0","AT+QCFG=\"pcie/mode\",1","AT+QETH=\"eth_driver\",\"r8125\",1","AT+QMAP=\"MPDN_rule\",0,1,0,0,1"}));
            Check(Qualcomm.EthernetPlan("r8168",0,false,false).SequenceEqual(new[]{"AT+QETH=\"eth_driver\",\"r8168\",0"}));
        });
        Case("reference profile is explicit six-command plan", () => {
            Check(Qualcomm.EthernetPlan("r8125",0,true,true).SequenceEqual(new[]{"AT+QCFG=\"data_interface\",0,0","AT+QCFG=\"pcie/mode\",1","AT+QETH=\"eth_driver\",\"r8125\",1","AT+QCFG=\"usbnet\",1","AT+QSIMDET=1,1","AT+QMAPWAC=1"}));
        });
        Case("exact response terminators", () => {
            foreach(var line in new[]{"OK","ERROR","+CME ERROR: 10","+CMS ERROR: 500"}) Check(AtReply.IsTerminal(line));
            foreach(var line in new[]{"BOOK","NOT OK","+QIND: \"OK\""}) Check(!AtReply.IsTerminal(line));
        });
        Console.WriteLine(passed + " Qualcomm tests passed; no physical serial port opened.");
    }
}