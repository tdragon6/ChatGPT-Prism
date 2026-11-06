using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Web.Script.Serialization;

// Observe window events for the verified executables supplied by the host.
internal static class WinEventHost
{
    private const uint WM_QUIT = 0x0012;
    private const uint WM_SNAPSHOT = 0x8001;
    private const uint EVENT_SYSTEM_FOREGROUND = 0x0003;
    private const uint EVENT_SYSTEM_MOVESIZESTART = 0x000A;
    private const uint EVENT_SYSTEM_MOVESIZEEND = 0x000B;
    private const uint EVENT_OBJECT_CREATE = 0x8000;
    private const uint EVENT_OBJECT_DESTROY = 0x8001;
    private const uint EVENT_OBJECT_SHOW = 0x8002;
    private const uint EVENT_OBJECT_HIDE = 0x8003;
    private const uint EVENT_OBJECT_STATECHANGE = 0x800A;
    private const uint EVENT_OBJECT_LOCATIONCHANGE = 0x800B;
    private static readonly HashSet<string> Allowed = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
    private static readonly Dictionary<IntPtr, Identity> Known = new Dictionary<IntPtr, Identity>();
    private static readonly Dictionary<IntPtr, Snapshot> Previous = new Dictionary<IntPtr, Snapshot>();
    private static readonly HashSet<IntPtr> Owned = new HashSet<IntPtr>();
    private static readonly object OwnedLock = new object();
    private static readonly List<IntPtr> Hooks = new List<IntPtr>();
    private static readonly JavaScriptSerializer Json = new JavaScriptSerializer();
    private static readonly object OutputLock = new object();
    private static readonly WinEventProc Callback = OnEvent;
    private static readonly EnumWindowsProc EnumerateCallback = OnEnumerate;
    private static uint ThreadId;
    private static string StopReason = "message_loop_end";
    private static int StopRequested;
    private static int OutputClosed;
    private static IntPtr ParentHandle;
    private static Timer ParentWatch;
    private static Timer WindowWatch;
    private static int SnapshotQueued;
    private static uint ParentPid;
    private static readonly Dictionary<uint, ProcessMetadata> ProcessCache = new Dictionary<uint, ProcessMetadata>();
    private static ulong LastCachePrune;
    private static int PollInterval = 500;

    private sealed class Identity { public uint Pid; public string Path; public string StartFileTime; public string StartedAt; }
    private sealed class ProcessMetadata { public IntPtr Handle; public Identity Identity; public ulong LastUsed; }
    private sealed class Snapshot {
        public Rect Window, Frame;
        public bool WindowOk, FrameOk, Visible, Minimized, Maximized, Fullscreen, Foreground, Eligible;
        public int? Cloaked;
        public uint Dpi;
        public List<Rect> Clips = new List<Rect>();
        public Rect Bounds { get { return FrameOk ? Frame : Window; } }
        public string Signature {
            get {
                Rect r = Bounds;
                StringBuilder s = new StringBuilder();
                s.Append(r.Left).Append(',').Append(r.Top).Append(',').Append(r.Right).Append(',').Append(r.Bottom)
                    .Append('|').Append(Visible).Append('|').Append(Minimized).Append('|').Append(Maximized)
                    .Append('|').Append(Fullscreen).Append('|').Append(Foreground).Append('|').Append(Eligible)
                    .Append('|').Append(Cloaked).Append('|').Append(Dpi);
                foreach (Rect clip in Clips) s.Append('|').Append(clip.Left).Append(',').Append(clip.Top).Append(',').Append(clip.Right).Append(',').Append(clip.Bottom);
                return s.ToString();
            }
        }
    }
    [StructLayout(LayoutKind.Sequential)] private struct Rect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] private struct Point { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] private struct Msg { public IntPtr Hwnd; public uint Message; public UIntPtr WParam; public IntPtr LParam; public uint Time; public Point Pt; public uint Private; }
    [StructLayout(LayoutKind.Sequential)] private struct FileTime { public uint Low, High; }
    [StructLayout(LayoutKind.Sequential)] private struct MonitorInfo { public int Size; public Rect Monitor, Work; public uint Flags; }
    private delegate void WinEventProc(IntPtr hook, uint evt, IntPtr hwnd, int objectId, int childId, uint eventThread, uint eventTime);
    private delegate bool EnumWindowsProc(IntPtr hwnd, IntPtr param);

    [DllImport("user32.dll", SetLastError = true)] private static extern IntPtr SetWinEventHook(uint min, uint max, IntPtr module, WinEventProc proc, uint pid, uint thread, uint flags);
    [DllImport("user32.dll")] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool UnhookWinEvent(IntPtr hook);
    [DllImport("user32.dll")] private static extern IntPtr GetAncestor(IntPtr hwnd, uint flags);
    [DllImport("user32.dll")] private static extern IntPtr GetWindow(IntPtr hwnd, uint command);
    [DllImport("user32.dll")] private static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] private static extern IntPtr OpenInputDesktop(uint flags, bool inherit, uint access);
    [DllImport("user32.dll")] private static extern bool CloseDesktop(IntPtr desktop);
    [DllImport("user32.dll", EntryPoint = "GetWindowLongW")] private static extern int GetWindowLong32(IntPtr hwnd, int index);
    [DllImport("user32.dll", EntryPoint = "GetWindowLongPtrW")] private static extern IntPtr GetWindowLong64(IntPtr hwnd, int index);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll")] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool GetWindowRect(IntPtr hwnd, out Rect rect);
    [DllImport("user32.dll")] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool IsZoomed(IntPtr hwnd);
    [DllImport("user32.dll")] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool IsIconic(IntPtr hwnd);
    [DllImport("user32.dll")] private static extern uint GetDpiForWindow(IntPtr hwnd);
    [DllImport("user32.dll")] private static extern IntPtr MonitorFromWindow(IntPtr hwnd, uint flags);
    [DllImport("user32.dll")] private static extern bool GetMonitorInfo(IntPtr monitor, ref MonitorInfo info);
    [DllImport("user32.dll")] [return: MarshalAs(UnmanagedType.Bool)] private static extern bool EnumWindows(EnumWindowsProc callback, IntPtr param);
    [DllImport("user32.dll")] private static extern int GetMessage(out Msg msg, IntPtr hwnd, uint min, uint max);
    [DllImport("user32.dll")] private static extern bool PeekMessage(out Msg msg, IntPtr hwnd, uint min, uint max, uint remove);
    [DllImport("user32.dll")] private static extern bool TranslateMessage(ref Msg msg);
    [DllImport("user32.dll")] private static extern IntPtr DispatchMessage(ref Msg msg);
    [DllImport("user32.dll", SetLastError = true)] private static extern bool PostThreadMessage(uint thread, uint message, UIntPtr wp, IntPtr lp);
    [DllImport("user32.dll")] private static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("kernel32.dll")] private static extern uint GetCurrentThreadId();
    [DllImport("kernel32.dll")] private static extern ulong GetTickCount64();
    [DllImport("kernel32.dll", SetLastError = true)] private static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)] private static extern bool QueryFullProcessImageName(IntPtr process, uint flags, StringBuilder path, ref uint size);
    [DllImport("kernel32.dll")] private static extern bool GetProcessTimes(IntPtr process, out FileTime creation, out FileTime exit, out FileTime kernel, out FileTime user);
    [DllImport("kernel32.dll")] private static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll")] private static extern uint WaitForSingleObject(IntPtr handle, uint milliseconds);
    [DllImport("kernel32.dll")] private static extern uint SetErrorMode(uint mode);
    [DllImport("dwmapi.dll")] private static extern int DwmGetWindowAttribute(IntPtr hwnd, uint attr, out Rect value, int size);
    [DllImport("dwmapi.dll", EntryPoint = "DwmGetWindowAttribute")] private static extern int DwmGetWindowAttributeInt(IntPtr hwnd, uint attr, out int value, int size);

    private static void Emit(object value)
    {
        if (Interlocked.CompareExchange(ref OutputClosed, 0, 0) != 0) return;
        lock (OutputLock)
        {
            try { Console.WriteLine(Json.Serialize(value)); Console.Out.Flush(); }
            catch { Interlocked.Exchange(ref OutputClosed, 1); RequestStop("stdout_closed"); }
        }
    }
    private static string Normalize(string path)
    {
        if (path.StartsWith(@"\\?\UNC\", StringComparison.OrdinalIgnoreCase))
            path = @"\\" + path.Substring(8);
        else if (path.StartsWith(@"\\?\", StringComparison.Ordinal) && path.Length > 6 && path[5] == ':')
            path = path.Substring(4);
        return Path.GetFullPath(path).TrimEnd('\\');
    }
    private static void PruneProcessCache(bool all)
    {
        ulong now = GetTickCount64();
        if (!all && now - LastCachePrune < 2000) return;
        LastCachePrune = now;
        List<uint> remove = new List<uint>();
        foreach (KeyValuePair<uint, ProcessMetadata> entry in ProcessCache)
            if (all || now - entry.Value.LastUsed > 10000 || WaitForSingleObject(entry.Value.Handle, 0) != 0x102) remove.Add(entry.Key);
        foreach (uint pid in remove) { CloseHandle(ProcessCache[pid].Handle); ProcessCache.Remove(pid); }
    }
    private static Identity Identify(uint pid, bool cacheUnrelated = false)
    {
        if (pid == 0) return null;
        ProcessMetadata cached;
        if (ProcessCache.TryGetValue(pid, out cached)) {
            if (WaitForSingleObject(cached.Handle, 0) == 0x102) { cached.LastUsed = GetTickCount64(); return cached.Identity; }
            CloseHandle(cached.Handle); ProcessCache.Remove(pid);
        }
        IntPtr process = OpenProcess(0x00101000, false, pid); // Read metadata while retaining the original process lifetime.
        if (process == IntPtr.Zero) return null;
        try
        {
            uint size = 32768; StringBuilder path = new StringBuilder((int)size);
            if (!QueryFullProcessImageName(process, 0, path, ref size)) return null;
            string full = Normalize(path.ToString());
            Identity identity = null;
            if (Allowed.Contains(full)) {
                FileTime created, exited, kernel, user;
                if (!GetProcessTimes(process, out created, out exited, out kernel, out user)) return null;
                long start = (long)(((ulong)created.High << 32) | created.Low);
                identity = new Identity { Pid = pid, Path = full,
                    StartFileTime = start.ToString(CultureInfo.InvariantCulture),
                    StartedAt = DateTime.FromFileTimeUtc(start).ToString("o", CultureInfo.InvariantCulture) };
            }
            if ((identity != null || cacheUnrelated) && ProcessCache.Count < 512) {
                ProcessCache[pid] = new ProcessMetadata { Handle = process, Identity = identity, LastUsed = GetTickCount64() };
                process = IntPtr.Zero;
            }
            return identity;
        }
        finally { if (process != IntPtr.Zero) CloseHandle(process); }
    }
    private static Identity IdentifyWindow(IntPtr hwnd)
    {
        uint pid; GetWindowThreadProcessId(hwnd, out pid); return Identify(pid, true);
    }
    private static int SnapshotInterval()
    {
        foreach (Snapshot snapshot in Previous.Values)
            if (snapshot.Visible && !snapshot.Minimized && snapshot.Cloaked == 0 &&
                snapshot.Bounds.Right - snapshot.Bounds.Left >= 100 && snapshot.Bounds.Bottom - snapshot.Bounds.Top >= 100) return 80;
        return 500;
    }
    private static void UpdateSnapshotInterval()
    {
        int next = SnapshotInterval();
        if (next == PollInterval || WindowWatch == null) return;
        PollInterval = next;
        try { WindowWatch.Change(next, next); } catch (ObjectDisposedException) { }
    }
    private static bool IsMainWindowCandidate(IntPtr hwnd)
    {
        if (GetAncestor(hwnd, 2) != hwnd || GetWindow(hwnd, 4) != IntPtr.Zero) return false;
        long style = IntPtr.Size == 8 ? GetWindowLong64(hwnd, -20).ToInt64() : GetWindowLong32(hwnd, -20);
        return (style & 0x80) == 0; // WS_EX_TOOLWINDOW: no tooltip/palette/tool windows.
    }
    private static object Bounds(Rect r) { return new { x = r.Left, y = r.Top, left = r.Left, top = r.Top, right = r.Right, bottom = r.Bottom, width = r.Right - r.Left, height = r.Bottom - r.Top }; }
    private static bool Intersect(Rect a, Rect b, out Rect result)
    {
        result = new Rect { Left = Math.Max(a.Left, b.Left), Top = Math.Max(a.Top, b.Top), Right = Math.Min(a.Right, b.Right), Bottom = Math.Min(a.Bottom, b.Bottom) };
        return result.Right > result.Left && result.Bottom > result.Top;
    }
    private static List<Rect> Subtract(List<Rect> regions, Rect block)
    {
        List<Rect> result = new List<Rect>();
        foreach (Rect region in regions) {
            Rect hit;
            if (!Intersect(region, block, out hit)) { result.Add(region); continue; }
            if (hit.Top > region.Top) result.Add(new Rect { Left = region.Left, Top = region.Top, Right = region.Right, Bottom = hit.Top });
            if (hit.Bottom < region.Bottom) result.Add(new Rect { Left = region.Left, Top = hit.Bottom, Right = region.Right, Bottom = region.Bottom });
            if (hit.Left > region.Left) result.Add(new Rect { Left = region.Left, Top = hit.Top, Right = hit.Left, Bottom = hit.Bottom });
            if (hit.Right < region.Right) result.Add(new Rect { Left = hit.Right, Top = hit.Top, Right = region.Right, Bottom = hit.Bottom });
            // A pathological window arrangement must hide, not overdraw others.
            if (result.Count > 128) return new List<Rect>();
        }
        return result;
    }
    private static bool CanDecorateDesktop()
    {
        IntPtr desktop = OpenInputDesktop(0, false, 1); // DESKTOP_READOBJECTS; no desktop switching.
        if (desktop == IntPtr.Zero) return false;
        CloseDesktop(desktop); return true;
    }
    private static Snapshot Capture(IntPtr hwnd, IntPtr foreground)
    {
        Snapshot s = new Snapshot();
        s.WindowOk = GetWindowRect(hwnd, out s.Window);
        s.FrameOk = DwmGetWindowAttribute(hwnd, 9, out s.Frame, Marshal.SizeOf(typeof(Rect))) == 0;
        int cloaked; bool cloakOk = DwmGetWindowAttributeInt(hwnd, 14, out cloaked, 4) == 0;
        s.Cloaked = cloakOk ? (int?)cloaked : null;
        try { s.Dpi = GetDpiForWindow(hwnd); } catch (EntryPointNotFoundException) { }
        s.Minimized = IsIconic(hwnd); s.Maximized = IsZoomed(hwnd); s.Visible = IsWindowVisible(hwnd);
        MonitorInfo monitor = new MonitorInfo(); monitor.Size = Marshal.SizeOf(typeof(MonitorInfo));
        Rect visibleFrame = s.Bounds;
        s.Fullscreen = !s.Minimized && (s.FrameOk || s.WindowOk) && GetMonitorInfo(MonitorFromWindow(hwnd, 2), ref monitor)
            && Math.Abs(visibleFrame.Left - monitor.Monitor.Left) <= 2 && Math.Abs(visibleFrame.Top - monitor.Monitor.Top) <= 2
            && Math.Abs(visibleFrame.Right - monitor.Monitor.Right) <= 2 && Math.Abs(visibleFrame.Bottom - monitor.Monitor.Bottom) <= 2;
        s.Foreground = foreground == hwnd;
        s.Eligible = s.Visible && !s.Minimized && s.Cloaked == 0 && (s.FrameOk || s.WindowOk) && CanDecorateDesktop();
        if (s.Eligible) {
            List<Rect> regions = new List<Rect>(); regions.Add(visibleFrame);
            HashSet<IntPtr> owned; lock (OwnedLock) { owned = new HashSet<IntPtr>(Owned); }
            IntPtr above = GetWindow(hwnd, 3); // GW_HWNDPREV: all higher windows, not just foreground.
            HashSet<IntPtr> visited = new HashSet<IntPtr>();
            while (above != IntPtr.Zero && visited.Add(above) && visited.Count <= 1024 && regions.Count > 0) {
                uint pid; GetWindowThreadProcessId(above, out pid);
                if (!(owned.Contains(above) && pid == ParentPid) && IsWindowVisible(above) && !IsIconic(above)) {
                    int hidden; bool knownCloak = DwmGetWindowAttributeInt(above, 14, out hidden, 4) == 0;
                    Rect block;
                    if ((!knownCloak || hidden == 0) &&
                        (DwmGetWindowAttribute(above, 9, out block, Marshal.SizeOf(typeof(Rect))) == 0 || GetWindowRect(above, out block)))
                        regions = Subtract(regions, block);
                }
                above = GetWindow(above, 3);
            }
            foreach (Rect r in regions) s.Clips.Add(new Rect { Left = r.Left - visibleFrame.Left, Top = r.Top - visibleFrame.Top, Right = r.Right - visibleFrame.Left, Bottom = r.Bottom - visibleFrame.Top });
            if (s.Clips.Count == 0) s.Eligible = false;
        }
        return s;
    }
    private static object WindowEvent(string eventName, IntPtr hwnd, Identity id, bool baseline, Snapshot s)
    {
        List<object> clips = new List<object>(); foreach (Rect r in s.Clips) clips.Add(Bounds(r));
        return new {
            type = "window", eventName = eventName, baseline = baseline,
            hwnd = "0x" + hwnd.ToInt64().ToString("X"),
            pid = id.Pid, executablePath = id.Path, processStartFileTime = id.StartFileTime, processStartedAt = id.StartedAt,
            dwmFrameBoundsPhysical = s.FrameOk ? Bounds(s.Frame) : null,
            windowRectPhysical = s.WindowOk ? Bounds(s.Window) : null,
            rect = s.FrameOk || s.WindowOk ? Bounds(s.Bounds) : null,
            dpi = s.Dpi, maximized = s.Maximized, minimized = s.Minimized, visible = s.Visible,
            state = s.Minimized ? "minimized" : s.Fullscreen ? "fullscreen" : s.Maximized ? "maximized" : "normal",
            cloaked = s.Cloaked.GetValueOrDefault(1) != 0, foreground = s.Foreground, overlayEligible = s.Eligible,
            clipRects = clips
        };
    }
    private static bool SameIdentity(Identity first, Identity second)
    {
        return first != null && second != null && first.Pid == second.Pid && first.StartFileTime == second.StartFileTime;
    }
    private static void TrackWindow(IntPtr hwnd, Identity identity)
    {
        Identity previous;
        if (Known.TryGetValue(hwnd, out previous) && !SameIdentity(previous, identity)) {
            Previous.Remove(hwnd);
        }
        Known[hwnd] = identity;
    }
    private static bool IsWindowMetadataEvent(uint evt)
    {
        return evt == EVENT_OBJECT_CREATE || evt == EVENT_OBJECT_DESTROY || evt == EVENT_OBJECT_SHOW ||
            evt == EVENT_OBJECT_HIDE || evt == EVENT_OBJECT_LOCATIONCHANGE || evt == EVENT_OBJECT_STATECHANGE ||
            evt == EVENT_SYSTEM_MOVESIZESTART || evt == EVENT_SYSTEM_MOVESIZEEND;
    }
    private static int EnumerateProcesses()
    {
        int count = 0;
        foreach (Process process in Process.GetProcesses())
        {
            try
            {
                Identity id = Identify((uint)process.Id); if (id == null) continue;
                Emit(new { type = "process", eventName = "BASELINE", baseline = true, pid = id.Pid,
                    executablePath = id.Path, processStartFileTime = id.StartFileTime, processStartedAt = id.StartedAt });
                count++;
            }
            catch { /* A process may disappear during enumeration. */ }
            finally { process.Dispose(); }
        }
        return count;
    }
    private static bool OnEnumerate(IntPtr hwnd, IntPtr unused)
    {
        try
        {
            if (!IsMainWindowCandidate(hwnd)) return true;
            Identity id = IdentifyWindow(hwnd); if (id == null) return true;
            TrackWindow(hwnd, id);
            Snapshot snapshot = Capture(hwnd, GetAncestor(GetForegroundWindow(), 2));
            Previous[hwnd] = snapshot;
            Emit(WindowEvent("BASELINE", hwnd, id, true, snapshot));
        }
        catch { Emit(new { type = "error", code = "window_baseline_failed" }); }
        return true;
    }
    private static void OnEvent(IntPtr hook, uint evt, IntPtr hwnd, int objectId, int childId, uint eventThread, uint eventTime)
    {
        try
        {
            if (evt == EVENT_SYSTEM_FOREGROUND) { RequestSnapshot(); return; }
            if (hwnd == IntPtr.Zero || objectId != 0 || childId != 0 || !IsWindowMetadataEvent(evt)) return;
            if (Known.ContainsKey(hwnd)) { RequestSnapshot(); return; }
            if (GetAncestor(hwnd, 2) != hwnd) return;
            if (!IsMainWindowCandidate(hwnd) || IdentifyWindow(hwnd) != null) RequestSnapshot();
        }
        catch { Emit(new { type = "error", code = "window_event_failed" }); }
    }
    private static void RequestSnapshot()
    {
        if (ThreadId == 0 || Interlocked.CompareExchange(ref SnapshotQueued, 1, 0) != 0) return;
        if (!PostThreadMessage(ThreadId, WM_SNAPSHOT, UIntPtr.Zero, IntPtr.Zero)) Interlocked.Exchange(ref SnapshotQueued, 0);
    }
    private static void SnapshotWindows()
    {
        HashSet<IntPtr> observed = new HashSet<IntPtr>();
        List<object> changes = new List<object>();
        IntPtr foreground = GetAncestor(GetForegroundWindow(), 2);
        bool enumerated = EnumWindows(delegate(IntPtr hwnd, IntPtr ignored) {
            try {
                if (!IsMainWindowCandidate(hwnd)) return true;
                Identity id = IdentifyWindow(hwnd); if (id == null) return true;
                observed.Add(hwnd);
                Snapshot next = Capture(hwnd, foreground), previous;
                Identity oldIdentity;
                bool existed = Known.TryGetValue(hwnd, out oldIdentity) && oldIdentity.Pid == id.Pid && oldIdentity.StartFileTime == id.StartFileTime;
                if (!existed && oldIdentity != null)
                    changes.Add(WindowEvent("DESTROY", hwnd, oldIdentity, false, new Snapshot()));
                TrackWindow(hwnd, id);
                if (!existed || !Previous.TryGetValue(hwnd, out previous) || previous.Signature != next.Signature) {
                    string name = "STATE";
                    if (existed && Previous.TryGetValue(hwnd, out previous)) {
                        if (previous.Visible && !next.Visible) name = "HIDE";
                        else if (!previous.Visible && next.Visible) name = "SHOW";
                        else {
                            Rect a = previous.Bounds, b = next.Bounds;
                            if (a.Left != b.Left || a.Top != b.Top || a.Right != b.Right || a.Bottom != b.Bottom || previous.Minimized != next.Minimized)
                                name = "LOCATIONCHANGE";
                        }
                    } else if (next.Visible) name = "SHOW";
                    Previous[hwnd] = next;
                    changes.Add(WindowEvent(name, hwnd, id, false, next));
                }
            } catch { Emit(new { type = "error", code = "window_snapshot_failed" }); }
            return true;
        }, IntPtr.Zero);
        if (enumerated) {
            List<IntPtr> removed = new List<IntPtr>();
            foreach (KeyValuePair<IntPtr, Identity> pair in Known) if (!observed.Contains(pair.Key)) removed.Add(pair.Key);
            foreach (IntPtr hwnd in removed) {
                changes.Add(WindowEvent("DESTROY", hwnd, Known[hwnd], false, new Snapshot()));
                Known.Remove(hwnd); Previous.Remove(hwnd);
            }
        }
        if (changes.Count > 0) {
            Emit(new { type = "WINDOW_SNAPSHOT_BEGIN" });
            foreach (object change in changes) Emit(change);
            Emit(new { type = "WINDOW_SNAPSHOT_END" });
        }
        PruneProcessCache(false);
        UpdateSnapshotInterval();
    }
    private static bool SetOwnedWindows(string line)
    {
        if (line.Length > 8192) return false;
        Dictionary<string, object> command;
        try { command = new JavaScriptSerializer().Deserialize<Dictionary<string, object>>(line); } catch { return false; }
        if (command == null || command.Count != 2 || !command.ContainsKey("type") || !command.ContainsKey("ids") ||
            !(command["type"] is string) || (string)command["type"] != "owned-windows" || !(command["ids"] is System.Collections.IList)) return false;
        System.Collections.IList ids = (System.Collections.IList)command["ids"]; if (ids.Count > 128) return false;
        HashSet<IntPtr> next = new HashSet<IntPtr>();
        foreach (object value in ids) {
            string text = value as string; long numeric;
            if (text == null || text.Length < 1 || text.Length > 20) return false;
            bool hex = text.StartsWith("0x", StringComparison.OrdinalIgnoreCase);
            if (!Int64.TryParse(hex ? text.Substring(2) : text, hex ? NumberStyles.AllowHexSpecifier : NumberStyles.None, CultureInfo.InvariantCulture, out numeric) || numeric <= 0) return false;
            IntPtr hwnd; try { hwnd = new IntPtr(numeric); } catch { return false; }
            uint pid; GetWindowThreadProcessId(hwnd, out pid);
            // Never exclude another application's window, including a guessed ID.
            if (pid == ParentPid) next.Add(hwnd);
        }
        lock (OwnedLock) { Owned.Clear(); foreach (IntPtr hwnd in next) Owned.Add(hwnd); }
        RequestSnapshot(); return true;
    }
    private static void RequestStop(string reason)
    {
        if (Interlocked.Exchange(ref StopRequested, 1) != 0) return;
        StopReason = reason;
        if (ThreadId != 0) PostThreadMessage(ThreadId, WM_QUIT, UIntPtr.Zero, IntPtr.Zero);
    }
    public static int Main(string[] args)
    {
        uint parentPid = 0;
        // Avoid operating-system fault dialogs; errors travel through the private pipe.
        SetErrorMode(0x0001 | 0x0002 | 0x8000);
        try
        {
            Console.OutputEncoding = new UTF8Encoding(false);
            Console.InputEncoding = new UTF8Encoding(false);
            for (int i = 0; i < args.Length; i++)
            {
                if (args[i] == "--allow" && i + 1 < args.Length)
                {
                    string file = args[++i];
                    if (!Path.IsPathRooted(file) || !file.EndsWith(".exe", StringComparison.OrdinalIgnoreCase) || !File.Exists(file)) throw new ArgumentException();
                    Allowed.Add(Normalize(file));
                }
                else if (args[i] == "--parent-pid" && i + 1 < args.Length) parentPid = UInt32.Parse(args[++i], CultureInfo.InvariantCulture);
                else throw new ArgumentException();
            }
            if (Allowed.Count == 0 || Allowed.Count > 4 || parentPid == 0 || parentPid == (uint)Process.GetCurrentProcess().Id) throw new ArgumentException();
            ParentPid = parentPid;
            ParentHandle = OpenProcess(0x00100000, false, parentPid); // SYNCHRONIZE; retains identity across PID reuse.
            if (ParentHandle == IntPtr.Zero || WaitForSingleObject(ParentHandle, 0) != 0x102) throw new InvalidOperationException();
            try { SetThreadDpiAwarenessContext(new IntPtr(-4)); } catch (EntryPointNotFoundException) { }
            ThreadId = GetCurrentThreadId();
            Msg first; PeekMessage(out first, IntPtr.Zero, 0, 0, 0);
            uint[][] ranges = new uint[][] {
                new uint[] { EVENT_OBJECT_CREATE, EVENT_OBJECT_LOCATIONCHANGE },
                new uint[] { EVENT_SYSTEM_MOVESIZESTART, EVENT_SYSTEM_MOVESIZEEND },
                new uint[] { EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND }
            };
            foreach (uint[] range in ranges)
            {
                IntPtr hook = SetWinEventHook(range[0], range[1], IntPtr.Zero, Callback, 0, 0, 0); // WINEVENT_OUTOFCONTEXT; module must remain NULL.
                if (hook == IntPtr.Zero) throw new InvalidOperationException();
                Hooks.Add(hook);
            }
            int processes = EnumerateProcesses();
            if (!EnumWindows(EnumerateCallback, IntPtr.Zero)) throw new InvalidOperationException();
            Emit(new { type = "READY", protocolVersion = 1, hooksRegistered = Hooks.Count,
                baselineWindows = Known.Count, baselineProcesses = processes, helperPid = Process.GetCurrentProcess().Id });
            ParentWatch = new Timer(delegate(object ignored) {
                if (WaitForSingleObject(ParentHandle, 0) != 0x102) RequestStop("parent_exit");
            }, null, 250, 250);
            // Geometry/stacking metadata only. No window titles, contents, input,
            // screenshots, client files or application control are accessed.
            PollInterval = SnapshotInterval();
            WindowWatch = new Timer(delegate(object ignored) { RequestSnapshot(); }, null, PollInterval, PollInterval);
            Thread stdin = new Thread(delegate() {
                try
                {
                    string line;
                    while ((line = Console.ReadLine()) != null)
                    {
                        if (line == "stop") { RequestStop("stdin_stop"); return; }
                        if (SetOwnedWindows(line)) continue;
                        // Reject unknown commands without executing anything.
                        RequestStop("invalid_command"); return;
                    }
                    RequestStop("stdin_eof");
                }
                catch { RequestStop("stdin_closed"); }
            });
            stdin.IsBackground = true; stdin.Start();
            Msg msg; int result;
            while ((result = GetMessage(out msg, IntPtr.Zero, 0, 0)) > 0) {
                if (msg.Message == WM_SNAPSHOT) { Interlocked.Exchange(ref SnapshotQueued, 0); SnapshotWindows(); }
                else { TranslateMessage(ref msg); DispatchMessage(ref msg); }
            }
            if (result < 0) StopReason = "getmessage_error";
            return result < 0 ? 1 : 0;
        }
        catch { StopReason = "error"; Emit(new { type = "error", code = "native_start_failed" }); return 1; }
        finally
        {
            if (ParentWatch != null) ParentWatch.Dispose();
            if (WindowWatch != null) WindowWatch.Dispose();
            PruneProcessCache(true);
            int unhooked = 0; foreach (IntPtr hook in Hooks) if (UnhookWinEvent(hook)) unhooked++;
            if (ParentHandle != IntPtr.Zero) CloseHandle(ParentHandle);
            Emit(new { type = "STOP", reason = StopReason, hooksRegistered = Hooks.Count, hooksUnregistered = unhooked });
            GC.KeepAlive(Callback); GC.KeepAlive(EnumerateCallback);
        }
    }
}
