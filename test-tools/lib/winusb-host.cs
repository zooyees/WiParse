using System;
using System.Collections.Generic;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Web.Script.Serialization;

namespace WiParse
{
    internal static class WinusbHost
    {
        static readonly Guid GuidUsbDevice = new Guid("A5DCBF10-6530-11D2-901F-00C04FB951ED");
        static readonly Guid GuidWinusb = new Guid("88BAE032-5A81-49F0-BC3D-A4FF138216D6");
        static readonly Guid GuidCmsis = new Guid("CDB3B5AD-293B-4663-AA36-1AAE46463776");
        static readonly Guid GuidUart = new Guid("7B3E9C41-2A18-4F6D-9E50-1C8A6D4B0101");
        static readonly Guid GuidI2c = new Guid("7B3E9C41-2A18-4F6D-9E50-1C8A6D4B0103");
        static readonly Guid GuidNet = new Guid("7B3E9C41-2A18-4F6D-9E50-1C8A6D4B0105");

        const int DIGCF_PRESENT = 0x2;
        const int DIGCF_ALLCLASSES = 0x4;
        const int DIGCF_DEVICEINTERFACE = 0x10;
        const uint SPDRP_DEVICEDESC = 0;
        const uint SPDRP_FRIENDLYNAME = 0x0C;
        const uint SPDRP_PDO = 0x0E;
        const uint GENERIC_READ = 0x80000000;
        const uint GENERIC_WRITE = 0x40000000;
        const uint FILE_SHARE_READ = 1;
        const uint FILE_SHARE_WRITE = 2;
        const uint OPEN_EXISTING = 3;
        const uint FILE_ATTRIBUTE_NORMAL = 0x80;
        const uint FILE_FLAG_OVERLAPPED = 0x40000000;
        const uint PIPE_TRANSFER_TIMEOUT = 0x03;
        const int INVALID_HANDLE_VALUE32 = -1;

        [StructLayout(LayoutKind.Sequential)]
        struct SP_DEVICE_INTERFACE_DATA
        {
            public int cbSize;
            public Guid InterfaceClassGuid;
            public int Flags;
            public IntPtr Reserved;
        }

        [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
        struct SP_DEVICE_INTERFACE_DETAIL_DATA
        {
            public int cbSize;
            [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 1024)]
            public string DevicePath;
        }

        [StructLayout(LayoutKind.Sequential)]
        struct SP_DEVINFO_DATA
        {
            public int cbSize;
            public Guid ClassGuid;
            public int DevInst;
            public IntPtr Reserved;
        }

        [StructLayout(LayoutKind.Sequential, Pack = 1)]
        struct WINUSB_SETUP_PACKET
        {
            public byte RequestType;
            public byte Request;
            public ushort Value;
            public ushort Index;
            public ushort Length;
        }

        [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        static extern IntPtr SetupDiGetClassDevs(ref Guid classGuid, string enumerator, IntPtr hwndParent, int flags);

        [DllImport("setupapi.dll", SetLastError = true)]
        static extern bool SetupDiEnumDeviceInterfaces(IntPtr h, IntPtr info, ref Guid guid, int index, ref SP_DEVICE_INTERFACE_DATA data);

        [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        static extern bool SetupDiGetDeviceInterfaceDetail(IntPtr h, ref SP_DEVICE_INTERFACE_DATA iface, IntPtr detail, int detailSize, out int required, IntPtr devinfo);

        [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        static extern bool SetupDiGetDeviceInterfaceDetail(IntPtr h, ref SP_DEVICE_INTERFACE_DATA iface, ref SP_DEVICE_INTERFACE_DETAIL_DATA detail, int detailSize, out int required, ref SP_DEVINFO_DATA devinfo);

        [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        static extern IntPtr SetupDiGetClassDevs(IntPtr classGuid, string enumerator, IntPtr hwndParent, int flags);

        [DllImport("setupapi.dll", SetLastError = true)]
        static extern bool SetupDiEnumDeviceInfo(IntPtr h, int index, ref SP_DEVINFO_DATA data);

        [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        static extern bool SetupDiGetDeviceInstanceId(IntPtr h, ref SP_DEVINFO_DATA data, StringBuilder buf, int bufLen, out int required);

        [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        static extern bool SetupDiGetDeviceRegistryProperty(IntPtr h, ref SP_DEVINFO_DATA data, uint prop, out uint regType, byte[] buf, uint bufLen, out uint required);

        [DllImport("setupapi.dll", SetLastError = true)]
        static extern bool SetupDiDestroyDeviceInfoList(IntPtr h);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        static extern IntPtr CreateFile(string name, uint access, uint share, IntPtr sec, uint disp, uint flags, IntPtr template);

        [DllImport("kernel32.dll", SetLastError = true)]
        static extern bool CloseHandle(IntPtr h);

        [DllImport("winusb.dll", SetLastError = true)]
        static extern bool WinUsb_Initialize(IntPtr handle, out IntPtr iface);

        [DllImport("winusb.dll", SetLastError = true)]
        static extern bool WinUsb_GetAssociatedInterface(IntPtr handle, byte index, out IntPtr associated);

        [DllImport("winusb.dll", SetLastError = true)]
        static extern bool WinUsb_Free(IntPtr iface);

        [DllImport("winusb.dll", SetLastError = true)]
        static extern bool WinUsb_WritePipe(IntPtr iface, byte pipe, byte[] buf, int len, out int transferred, IntPtr overlapped);

        [DllImport("winusb.dll", SetLastError = true)]
        static extern bool WinUsb_ReadPipe(IntPtr iface, byte pipe, byte[] buf, int len, out int transferred, IntPtr overlapped);

        [DllImport("winusb.dll", SetLastError = true)]
        static extern bool WinUsb_ControlTransfer(IntPtr iface, WINUSB_SETUP_PACKET setup, byte[] buf, int len, out int transferred, IntPtr overlapped);

        [DllImport("winusb.dll", SetLastError = true)]
        static extern bool WinUsb_SetPipePolicy(IntPtr iface, byte pipe, uint policy, int valueLength, ref uint value);

        [DllImport("winusb.dll", SetLastError = true)]
        static extern bool WinUsb_GetOverlappedResult(IntPtr iface, IntPtr overlapped, out int transferred, bool wait);

        [DllImport("kernel32.dll", SetLastError = true)]
        static extern IntPtr CreateEvent(IntPtr sa, bool manual, bool initial, string name);

        [DllImport("kernel32.dll", SetLastError = true)]
        static extern uint WaitForSingleObject(IntPtr handle, uint ms);

        [StructLayout(LayoutKind.Sequential)]
        struct OVERLAPPED
        {
            public IntPtr Internal;
            public IntPtr InternalHigh;
            public int Offset;
            public int OffsetHigh;
            public IntPtr EventHandle;
        }

        static readonly JavaScriptSerializer Json = new JavaScriptSerializer();
        static IntPtr fileHandle = new IntPtr(INVALID_HANDLE_VALUE32);
        static IntPtr winusbParent = IntPtr.Zero;
        static IntPtr winusb = IntPtr.Zero;

        static int Main()
        {
            string line;
            while ((line = Console.In.ReadLine()) != null)
            {
                line = line.Trim();
                if (line.Length == 0) continue;
                Dictionary<string, object> msg;
                try { msg = Json.Deserialize<Dictionary<string, object>>(line); }
                catch (Exception ex)
                {
                    Reply(0, false, ex.Message);
                    continue;
                }
                int id = GetInt(msg, "id", 0);
                string op = GetStr(msg, "op").ToLowerInvariant();
                try
                {
                    if (op == "quit" || op == "exit") { Reply(id, true, null); break; }
                    if (op == "list") ListDevs(id, msg);
                    else if (op == "open") OpenDev(id, msg);
                    else if (op == "write") WritePipe(id, msg);
                    else if (op == "read") ReadPipe(id, msg);
                    else if (op == "control") Control(id, msg);
                    else if (op == "close") { CloseDev(); Reply(id, true, null); }
                    else Reply(id, false, "unknown op " + op);
                }
                catch (Exception ex)
                {
                    Reply(id, false, ex.Message);
                }
            }
            CloseDev();
            return 0;
        }

        static void ListDevs(int id, Dictionary<string, object> msg)
        {
            string vid = NormalizeHex(GetStr(msg, "vid"), "C251");
            string pid = NormalizeHex(GetStr(msg, "pid"), "F01A");
            string needle = ("VID_" + vid + "&PID_" + pid).ToUpperInvariant();
            var items = new List<object>();
            var seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            foreach (var g in new[] { GuidNet, GuidUart, GuidI2c, GuidCmsis, GuidWinusb, GuidUsbDevice })
            {
                foreach (var path in Enumerate(g))
                {
                    if (path.ToUpperInvariant().IndexOf(needle) < 0) continue;
                    if (!seen.Add("p:" + path)) continue;
                    items.Add(new Dictionary<string, object> { { "path", path }, { "mi", ParseMi(path) } });
                }
            }
            foreach (var inst in EnumerateUsbInstances(needle))
            {
                if (!seen.Add("i:" + inst.InstanceId)) continue;
                string guess = FirstWorkingPath(inst.InstanceId, inst.Pdo);
                items.Add(new Dictionary<string, object> {
                    { "path", guess ?? "" },
                    { "instance_id", inst.InstanceId },
                    { "name", inst.Name },
                    { "mi", inst.Mi },
                    { "pdo", inst.Pdo }
                });
            }
            var extra = new Dictionary<string, object> { { "ok", true }, { "id", id }, { "devices", items } };
            Console.Out.WriteLine(Json.Serialize(extra));
            Console.Out.Flush();
        }

        static void OpenDev(int id, Dictionary<string, object> msg)
        {
            CloseDev();
            string explicitPath = GetStr(msg, "path");
            string vid = NormalizeHex(GetStr(msg, "vid"), "C251");
            string pid = NormalizeHex(GetStr(msg, "pid"), "F01A");
            int mi = GetInt(msg, "mi", -1);
            int associate = GetInt(msg, "associate", mi == 5 ? 0 : 1);
            var candidates = new List<string>();
            if (!string.IsNullOrEmpty(explicitPath)) candidates.Add(explicitPath);
            string needle = ("VID_" + vid + "&PID_" + pid).ToUpperInvariant();
            string miTok = mi >= 0 ? ("MI_" + mi.ToString("X2")) : "";
            foreach (var g in new[] { GuidNet, GuidUart, GuidI2c, GuidCmsis, GuidWinusb, GuidUsbDevice })
            {
                foreach (var p in Enumerate(g))
                {
                    string u = p.ToUpperInvariant();
                    if (u.IndexOf(needle) < 0) continue;
                    if (miTok.Length > 0 && u.IndexOf(miTok) < 0) continue;
                    candidates.Add(p);
                }
            }
            foreach (var inst in EnumerateUsbInstances(needle))
            {
                if (mi >= 0 && inst.Mi != mi) continue;
                foreach (var p in CandidatePaths(inst.InstanceId, inst.Pdo)) candidates.Add(p);
            }
            Exception last = null;
            foreach (var path in Unique(candidates))
            {
                try
                {
                    TryOpenPath(path, associate != 0);
                    Reply(id, true, null, new Dictionary<string, object> { { "path", path } });
                    return;
                }
                catch (Exception ex) { last = ex; CloseDev(); }
            }
            throw last ?? new Exception("device path not found");
        }

        static void TryOpenPath(string path, bool associate)
        {
            fileHandle = CreateFile(path, GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE, IntPtr.Zero, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OVERLAPPED, IntPtr.Zero);
            if (fileHandle == IntPtr.Zero || fileHandle.ToInt64() == INVALID_HANDLE_VALUE32)
                throw new Exception("CreateFile failed err=" + Marshal.GetLastWin32Error() + " path=" + path);
            if (!WinUsb_Initialize(fileHandle, out winusbParent))
            {
                int err = Marshal.GetLastWin32Error();
                CloseHandle(fileHandle);
                fileHandle = new IntPtr(INVALID_HANDLE_VALUE32);
                winusbParent = IntPtr.Zero;
                throw new Exception("WinUsb_Initialize failed err=" + err + " path=" + path);
            }
            IntPtr assoc;
            if (associate && WinUsb_GetAssociatedInterface(winusbParent, 0, out assoc) && assoc != IntPtr.Zero)
                winusb = assoc;
            else
                winusb = winusbParent;
        }

        static int OverlappedXfer(bool write, byte pipe, byte[] buf, int len, uint timeoutMs)
        {
            IntPtr ev = CreateEvent(IntPtr.Zero, true, false, null);
            var ov = new OVERLAPPED();
            ov.EventHandle = ev;
            IntPtr pov = Marshal.AllocHGlobal(Marshal.SizeOf(ov));
            try
            {
                Marshal.StructureToPtr(ov, pov, false);
                int n;
                bool ok = write
                    ? WinUsb_WritePipe(winusb, pipe, buf, len, out n, pov)
                    : WinUsb_ReadPipe(winusb, pipe, buf, len, out n, pov);
                if (ok) return n;
                int err = Marshal.GetLastWin32Error();
                if (err != 997) throw new Exception((write ? "WritePipe" : "ReadPipe") + " err=" + err);
                uint wr = WaitForSingleObject(ev, timeoutMs == 0 ? 1 : timeoutMs);
                if (wr != 0)
                {
                    if (!write) return 0;
                    throw new Exception("pipe timeout");
                }
                if (!WinUsb_GetOverlappedResult(winusb, pov, out n, false))
                {
                    err = Marshal.GetLastWin32Error();
                    if (!write && (err == 995 || err == 121)) return 0;
                    throw new Exception("GetOverlappedResult err=" + err);
                }
                return n;
            }
            finally
            {
                Marshal.FreeHGlobal(pov);
                CloseHandle(ev);
            }
        }

        static void WritePipe(int id, Dictionary<string, object> msg)
        {
            EnsureOpen();
            byte pipe = (byte)GetInt(msg, "pipe", 1);
            uint timeout = (uint)GetInt(msg, "timeout_ms", 1000);
            WinUsb_SetPipePolicy(winusb, pipe, PIPE_TRANSFER_TIMEOUT, 4, ref timeout);
            byte[] data = HexToBytes(GetStr(msg, "hex"));
            int n = OverlappedXfer(true, pipe, data, data.Length, timeout);
            Reply(id, true, null, new Dictionary<string, object> { { "transferred", n } });
        }

        static void ReadPipe(int id, Dictionary<string, object> msg)
        {
            EnsureOpen();
            byte pipe = (byte)GetInt(msg, "pipe", 0x81);
            int max = GetInt(msg, "max", 512);
            if (max < 1) max = 1;
            if (max > 65536) max = 65536;
            uint timeout = (uint)GetInt(msg, "timeout_ms", 1000);
            WinUsb_SetPipePolicy(winusb, pipe, PIPE_TRANSFER_TIMEOUT, 4, ref timeout);
            byte[] buf = new byte[max];
            int n = OverlappedXfer(false, pipe, buf, buf.Length, timeout);
            Reply(id, true, null, new Dictionary<string, object> { { "transferred", n }, { "hex", BytesToHex(buf, Math.Max(0, n)) } });
        }

        static void Control(int id, Dictionary<string, object> msg)
        {
            EnsureOpen();
            byte[] data = HexToBytes(GetStr(msg, "hex"));
            var setup = new WINUSB_SETUP_PACKET();
            setup.RequestType = (byte)GetInt(msg, "type", 0x21);
            setup.Request = (byte)GetInt(msg, "request", 0x20);
            setup.Value = (ushort)GetInt(msg, "value", 0);
            setup.Index = (ushort)GetInt(msg, "index", 1);
            setup.Length = (ushort)data.Length;
            bool inDir = (setup.RequestType & 0x80) != 0;
            if (inDir && data.Length == 0)
            {
                data = new byte[Math.Max(8, GetInt(msg, "max", 8))];
                setup.Length = (ushort)data.Length;
            }
            IntPtr ev = CreateEvent(IntPtr.Zero, true, false, null);
            var ov = new OVERLAPPED();
            ov.EventHandle = ev;
            IntPtr pov = Marshal.AllocHGlobal(Marshal.SizeOf(ov));
            try
            {
                Marshal.StructureToPtr(ov, pov, false);
                int n;
                bool ok = WinUsb_ControlTransfer(ControlIface(), setup, data.Length == 0 ? null : data, setup.Length, out n, pov);
                if (!ok)
                {
                    int err = Marshal.GetLastWin32Error();
                    if (err != 997) throw new Exception("ControlTransfer err=" + err);
                    uint wr = WaitForSingleObject(ev, (uint)GetInt(msg, "timeout_ms", 2000));
                    if (wr != 0) throw new Exception("control timeout");
                    if (!WinUsb_GetOverlappedResult(ControlIface(), pov, out n, false))
                        throw new Exception("Control overlapped err=" + Marshal.GetLastWin32Error());
                }
                Reply(id, true, null, new Dictionary<string, object> { { "transferred", n }, { "hex", BytesToHex(data, Math.Max(0, n)) } });
            }
            finally
            {
                Marshal.FreeHGlobal(pov);
                CloseHandle(ev);
            }
        }

        struct UsbInst
        {
            public string InstanceId;
            public string Name;
            public int Mi;
            public string Pdo;
        }

        static IEnumerable<UsbInst> EnumerateUsbInstances(string needle)
        {
            IntPtr h = SetupDiGetClassDevs(IntPtr.Zero, "USB", IntPtr.Zero, DIGCF_PRESENT | DIGCF_ALLCLASSES);
            if (h == IntPtr.Zero || h.ToInt64() == INVALID_HANDLE_VALUE32) yield break;
            try
            {
                for (int i = 0; i < 512; i++)
                {
                    var info = new SP_DEVINFO_DATA();
                    info.cbSize = Marshal.SizeOf(info);
                    if (!SetupDiEnumDeviceInfo(h, i, ref info)) break;
                    var sb = new StringBuilder(1024);
                    int needed;
                    if (!SetupDiGetDeviceInstanceId(h, ref info, sb, sb.Capacity, out needed)) continue;
                    string id = sb.ToString();
                    if (id.ToUpperInvariant().IndexOf(needle) < 0) continue;
                    string name = GetProp(h, ref info, SPDRP_FRIENDLYNAME);
                    if (string.IsNullOrEmpty(name)) name = GetProp(h, ref info, SPDRP_DEVICEDESC);
                    yield return new UsbInst { InstanceId = id, Name = name, Mi = ParseMi(id), Pdo = GetProp(h, ref info, SPDRP_PDO) };
                }
            }
            finally { SetupDiDestroyDeviceInfoList(h); }
        }

        static string GetProp(IntPtr h, ref SP_DEVINFO_DATA info, uint prop)
        {
            uint type, needed;
            var buf = new byte[1024];
            if (!SetupDiGetDeviceRegistryProperty(h, ref info, prop, out type, buf, (uint)buf.Length, out needed)) return "";
            try { return Encoding.Unicode.GetString(buf, 0, (int)needed).TrimEnd('\0'); }
            catch { return ""; }
        }

        static IEnumerable<string> CandidatePaths(string instanceId, string pdo)
        {
            if (string.IsNullOrEmpty(instanceId)) yield break;
            string hash = instanceId.Replace('\\', '#');
            string body = hash;
            if (!body.StartsWith(@"\\?\", StringComparison.Ordinal)) body = @"\\?\" + body;
            int mi = ParseMi(instanceId);
            if (mi == 1) yield return body + "#" + GuidUart.ToString("B");
            if (mi == 3) yield return body + "#" + GuidI2c.ToString("B");
            if (mi == 5) yield return body + "#" + GuidNet.ToString("B");
            if (mi == 0) yield return body + "#" + GuidCmsis.ToString("B");
            if (!string.IsNullOrEmpty(pdo))
            {
                yield return @"\\.\GLOBALROOT" + pdo;
                yield return @"\\?\GLOBALROOT" + pdo;
            }
        }

        static string FirstWorkingPath(string instanceId, string pdo)
        {
            foreach (var p in CandidatePaths(instanceId, pdo))
            {
                IntPtr h = CreateFile(p, GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE, IntPtr.Zero, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OVERLAPPED, IntPtr.Zero);
                if (h == IntPtr.Zero || h.ToInt64() == INVALID_HANDLE_VALUE32) continue;
                CloseHandle(h);
                return p;
            }
            return null;
        }

        static IEnumerable<string> Unique(IEnumerable<string> items)
        {
            var seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            foreach (var s in items)
            {
                if (string.IsNullOrEmpty(s) || !seen.Add(s)) continue;
                yield return s;
            }
        }

        static IEnumerable<string> Enumerate(Guid guid)
        {
            var g = guid;
            IntPtr h = SetupDiGetClassDevs(ref g, null, IntPtr.Zero, DIGCF_PRESENT | DIGCF_DEVICEINTERFACE);
            if (h == IntPtr.Zero || h.ToInt64() == INVALID_HANDLE_VALUE32) yield break;
            try
            {
                for (int i = 0; i < 256; i++)
                {
                    var data = new SP_DEVICE_INTERFACE_DATA();
                    data.cbSize = Marshal.SizeOf(data);
                    if (!SetupDiEnumDeviceInterfaces(h, IntPtr.Zero, ref g, i, ref data)) break;
                    int required;
                    SetupDiGetDeviceInterfaceDetail(h, ref data, IntPtr.Zero, 0, out required, IntPtr.Zero);
                    var detail = new SP_DEVICE_INTERFACE_DETAIL_DATA();
                    detail.cbSize = IntPtr.Size == 8 ? 8 : 6;
                    var info = new SP_DEVINFO_DATA();
                    info.cbSize = Marshal.SizeOf(info);
                    if (!SetupDiGetDeviceInterfaceDetail(h, ref data, ref detail, required, out required, ref info)) continue;
                    if (!string.IsNullOrEmpty(detail.DevicePath)) yield return detail.DevicePath;
                }
            }
            finally { SetupDiDestroyDeviceInfoList(h); }
        }

        static IntPtr ControlIface()
        {
            return winusbParent != IntPtr.Zero ? winusbParent : winusb;
        }

        static void EnsureOpen()
        {
            if (winusb == IntPtr.Zero) throw new Exception("not open");
        }

        static void CloseDev()
        {
            if (winusb != IntPtr.Zero && winusb != winusbParent) { try { WinUsb_Free(winusb); } catch { } }
            winusb = IntPtr.Zero;
            if (winusbParent != IntPtr.Zero) { try { WinUsb_Free(winusbParent); } catch { } }
            winusbParent = IntPtr.Zero;
            if (fileHandle != IntPtr.Zero && fileHandle.ToInt64() != INVALID_HANDLE_VALUE32)
            {
                try { CloseHandle(fileHandle); } catch { }
                fileHandle = new IntPtr(INVALID_HANDLE_VALUE32);
            }
        }

        static void Reply(int id, bool ok, string error)
        {
            Reply(id, ok, error, null);
        }

        static void Reply(int id, bool ok, string error, Dictionary<string, object> extra)
        {
            var d = extra ?? new Dictionary<string, object>();
            d["id"] = id;
            d["ok"] = ok;
            if (!ok) d["error"] = error ?? "error";
            Console.Out.WriteLine(Json.Serialize(d));
            Console.Out.Flush();
        }

        static string GetStr(Dictionary<string, object> m, string k)
        {
            object v;
            if (m == null || !m.TryGetValue(k, out v) || v == null) return "";
            return Convert.ToString(v);
        }

        static int GetInt(Dictionary<string, object> m, string k, int dflt)
        {
            object v;
            if (m == null || !m.TryGetValue(k, out v) || v == null) return dflt;
            try { return Convert.ToInt32(v); } catch { return dflt; }
        }

        static string NormalizeHex(string s, string dflt)
        {
            if (string.IsNullOrEmpty(s)) return dflt;
            s = s.Trim();
            if (s.StartsWith("0x", StringComparison.OrdinalIgnoreCase)) s = s.Substring(2);
            if (s.Length == 0) return dflt;
            return s.PadLeft(4, '0').ToUpperInvariant();
        }

        static int ParseMi(string path)
        {
            string u = path.ToUpperInvariant();
            int i = u.IndexOf("MI_");
            if (i < 0) return -1;
            int n;
            if (int.TryParse(u.Substring(i + 3, 2), System.Globalization.NumberStyles.HexNumber, null, out n)) return n;
            return -1;
        }

        static byte[] HexToBytes(string hex)
        {
            if (string.IsNullOrEmpty(hex)) return new byte[0];
            hex = hex.Replace(" ", "").Replace(",", "");
            if ((hex.Length & 1) != 0) throw new Exception("odd hex length");
            byte[] b = new byte[hex.Length / 2];
            for (int i = 0; i < b.Length; i++) b[i] = Convert.ToByte(hex.Substring(i * 2, 2), 16);
            return b;
        }

        static string BytesToHex(byte[] b, int n)
        {
            if (b == null || n <= 0) return "";
            var sb = new StringBuilder(n * 2);
            for (int i = 0; i < n; i++) sb.Append(b[i].ToString("X2"));
            return sb.ToString();
        }
    }
}
