param(
    [ValidateSet('Native', 'WinForms')]
    [string]$Pump,
    [ValidateSet('PostMessage', 'SendInput')]
    [string]$Source,
    [switch]$DiagnosticLoad,
    [ValidateRange(1, 120)]
    [int]$DurationSeconds = 15,
    [ValidateNotNullOrEmpty()]
    [string]$OutputRoot,
    [switch]$CompileOnly
)

$ErrorActionPreference = 'Stop'
if (-not [Environment]::UserInteractive) { throw 'Pump probe requires an interactive Windows desktop' }
if ([string]::IsNullOrWhiteSpace($Pump) -or [string]::IsNullOrWhiteSpace($Source) -or
    [string]::IsNullOrWhiteSpace($OutputRoot)) {
    throw 'Pump, Source, and OutputRoot are required'
}
$probeRoot = [System.IO.Path]::GetFullPath($OutputRoot)
$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if ($probeRoot.StartsWith($repositoryRoot.TrimEnd('\') + '\', [System.StringComparison]::OrdinalIgnoreCase) -or
    [string]::Equals($probeRoot, $repositoryRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw 'Diagnostic evidence must be outside the repository checkout'
}
New-Item -ItemType Directory -Force -Path $probeRoot | Out-Null

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies @(
    [System.Windows.Forms.Form].Assembly.Location,
    [System.Windows.Forms.Message].Assembly.Location,
    [System.Drawing.Font].Assembly.Location,
    [System.ComponentModel.Component].Assembly.Location,
    (Join-Path $PSHOME 'ref\System.Collections.dll'),
    (Join-Path $PSHOME 'ref\System.Runtime.dll'),
    (Join-Path $PSHOME 'ref\System.Text.Encoding.dll'),
    (Join-Path $PSHOME 'ref\System.Text.Encoding.Extensions.dll'),
    (Join-Path $PSHOME 'ref\System.Threading.dll'),
    (Join-Path $PSHOME 'ref\System.Threading.Thread.dll'),
    (Join-Path $PSHOME 'ref\System.Diagnostics.Process.dll'),
    (Join-Path $PSHOME 'ref\System.Diagnostics.TraceSource.dll')
) -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Windows.Forms;

public static class SchedulerPumpProbeNative {
    public const int WM_KEYDOWN = 0x0100;
    public const int WM_KEYUP = 0x0101;
    public const int WM_SYSKEYDOWN = 0x0104;
    public const int WM_SYSKEYUP = 0x0105;
    public const int WM_APP = 0x8000;
    public const int WM_TIMER = 0x0113;
    public const int QS_KEY = 0x0001;
    public const uint PM_REMOVE = 0x0001;
    public const uint WS_OVERLAPPEDWINDOW = 0x00CF0000;
    public const int SW_SHOW = 5;
    public const uint INPUT_KEYBOARD = 1;
    public const uint KEYEVENTF_KEYUP = 0x0002;

    [StructLayout(LayoutKind.Sequential)]
    public struct Point { public int X; public int Y; }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    public struct Msg {
        public IntPtr HWnd;
        public uint Message;
        public UIntPtr WParam;
        public IntPtr LParam;
        public uint Time;
        public Point Pt;
        public uint Private;
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    public struct WndClassEx {
        public uint Size;
        public uint Style;
        public WndProcDelegate WndProc;
        public int ClassExtra;
        public int WindowExtra;
        public IntPtr Instance;
        public IntPtr Icon;
        public IntPtr Cursor;
        public IntPtr Background;
        public string MenuName;
        public string ClassName;
        public IntPtr SmallIcon;
    }

    [UnmanagedFunctionPointer(CallingConvention.Winapi)]
    public delegate IntPtr WndProcDelegate(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);
    public delegate bool EnumWindowsDelegate(IntPtr hwnd, IntPtr parameter);

    [StructLayout(LayoutKind.Sequential)]
    public struct KeyboardInput {
        public ushort VirtualKey;
        public ushort ScanCode;
        public uint Flags;
        public uint Time;
        public UIntPtr ExtraInfo;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct MouseInput {
        public int X;
        public int Y;
        public uint Data;
        public uint Flags;
        public uint Time;
        public UIntPtr ExtraInfo;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct HardwareInput { public uint Message; public ushort ParamLow; public ushort ParamHigh; }

    [StructLayout(LayoutKind.Explicit)]
    public struct InputUnion {
        [FieldOffset(0)] public MouseInput Mouse;
        [FieldOffset(0)] public KeyboardInput Keyboard;
        [FieldOffset(0)] public HardwareInput Hardware;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct Input { public uint Type; public InputUnion Data; }

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern ushort RegisterClassEx(ref WndClassEx windowClass);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern IntPtr CreateWindowEx(uint exStyle, string className, string windowName,
        uint style, int x, int y, int width, int height, IntPtr parent, IntPtr menu,
        IntPtr instance, IntPtr parameter);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hwnd, int command);
    [DllImport("user32.dll")] public static extern bool UpdateWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);
    [DllImport("user32.dll")] public static extern int GetMessage(out Msg message, IntPtr hwnd, uint min, uint max);
    [DllImport("user32.dll")] public static extern bool TranslateMessage(ref Msg message);
    [DllImport("user32.dll")] public static extern IntPtr DispatchMessage(ref Msg message);
    [DllImport("user32.dll")] public static extern IntPtr DefWindowProc(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool PostThreadMessage(uint threadId, uint message, IntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll")] public static extern uint GetQueueStatus(uint flags);
    [DllImport("user32.dll")] public static extern IntPtr SetTimer(IntPtr hwnd, UIntPtr id, uint milliseconds, IntPtr callback);
    [DllImport("user32.dll")] public static extern bool KillTimer(IntPtr hwnd, UIntPtr id);
    [DllImport("user32.dll")] public static extern bool DestroyWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsDelegate callback, IntPtr parameter);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(IntPtr hwnd, StringBuilder name, int maximum);
    [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr hwnd, int command);
    [DllImport("user32.dll", SetLastError = true)] public static extern uint SendInput(uint count, Input[] inputs, int size);
    [DllImport("kernel32.dll")] public static extern IntPtr GetModuleHandle(string name);
    [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
    [DllImport("kernel32.dll")] public static extern void PostQuitMessage(int exitCode);
}

public struct SchedulerPumpEvent {
    public long Sequence;
    public long RetrievedQpc;
    public long HandlerStartQpc;
    public long HandlerEndQpc;
    public long ForegroundCheckUs;
    public long Hwnd;
    public int Pid;
    public long ForegroundHwnd;
    public int ForegroundPid;
    public int Message;
    public int VirtualKey;
    public int ScanCode;
    public bool Extended;
    public uint QueueStatus;
}

public sealed class SchedulerPumpRing {
    private readonly object _gate = new object();
    private readonly SchedulerPumpEvent[] _items = new SchedulerPumpEvent[32768];
    private readonly AutoResetEvent _changed = new AutoResetEvent(false);
    private int _head, _tail, _count;
    private bool _completed, _overflowed;
    private long _seen, _enqueued, _written, _flushed, _dropped, _highWater, _writerFailures;
    public bool TryEnqueue(SchedulerPumpEvent item) {
        lock (_gate) {
            _seen++;
            if (_completed || _overflowed || _count == _items.Length) {
                _overflowed = true; _dropped++; _changed.Set(); return false;
            }
            item.Sequence = _seen;
            item.HandlerEndQpc = Stopwatch.GetTimestamp();
            _items[_tail] = item; _tail = (_tail + 1) % _items.Length; _count++;
            _enqueued++; if (_count > _highWater) _highWater = _count;
        }
        _changed.Set(); return true;
    }
    public bool TryDequeue(out SchedulerPumpEvent item) {
        while (true) {
            lock (_gate) {
                if (_count > 0) { item = _items[_head]; _head = (_head + 1) % _items.Length; _count--; return true; }
                if (_completed) { item = default(SchedulerPumpEvent); return false; }
            }
            _changed.WaitOne(100);
        }
    }
    public void Complete() { lock (_gate) _completed = true; _changed.Set(); }
    public long EventsSeen { get { lock (_gate) return _seen; } }
    public void Written() { Interlocked.Increment(ref _written); }
    public void Flushed(long count) { Interlocked.Add(ref _flushed, count); }
    public void WriterFailed() { Interlocked.Increment(ref _writerFailures); }
    public object Snapshot() {
        lock (_gate) return new { seen = _seen, enqueued = _enqueued, written = Interlocked.Read(ref _written),
            flushed = Interlocked.Read(ref _flushed), dropped = _dropped, high_water = _highWater,
            capacity = _items.Length, depth = _count, overflow = _overflowed,
            writer_failures = Interlocked.Read(ref _writerFailures), drained = _completed && _count == 0 };
    }
}

public sealed class SchedulerPumpProbeContext {
    public const string ObserverVersion = "scheduler-receiveonly-pump-prototype-v1";
    public const int WM_NONKEY = 0x8001;
    private readonly string _pump, _source, _eventPath, _runId;
    private readonly bool _load;
    private readonly int _durationSeconds;
    private readonly SchedulerPumpRing _ring = new SchedulerPumpRing();
    private readonly Thread _writer;
    private readonly long[] _dispatchLagUs = new long[32768];
    private readonly long[] _handlerUs = new long[32768];
    private readonly long[] _foregroundUs = new long[32768];
    private readonly long[] _sourceCallUs = new long[32768];
    private readonly object _latencyGate = new object();
    private int _dispatchCount, _handlerCount, _foregroundCount, _sourceCallCount;
    private long _targetHwnd, _targetPid, _startQpc, _sourceCompletionQpc;
    private long _sourceRequested, _sourceInserted, _sourceAttempts, _sourceCompletions;
    private long _postAccepted, _postRejected, _wrongIdentity, _pumpNonKey, _timerCallbacks;
    private long _overlayTotalUs, _overlayMaxUs, _beginInvokeCount, _beginInvokeCompleted, _beginInvokeTotalUs;
    private long _focusCheckCount, _focusCheckTotalUs;
    private readonly List<string> _perSecond = new List<string>();
    private readonly object _secondGate = new object();
    private long _lastSecondQpc;
    private Form _form;
    private Label _label;
    private System.Windows.Forms.Timer _timer;
    private IntPtr _nativeHwnd;
    private uint _nativeThreadId;
    private static SchedulerPumpProbeContext _nativeContext;
private static SchedulerPumpProbeNative.WndProcDelegate _nativeWndProc;
    [ThreadStatic] private static long _filterQpc;
    [ThreadStatic] private static long _filterHwnd;
    [ThreadStatic] private static int _filterMessage;
    [ThreadStatic] private static long _nativeRetrievedQpc;

    public SchedulerPumpProbeContext(string pump, string source, bool load, int durationSeconds, string eventPath, string runId) {
        _pump = pump; _source = source; _load = load; _durationSeconds = durationSeconds; _eventPath = eventPath; _runId = runId;
        _writer = new Thread(Consume) { IsBackground = true, Name = "pump prototype writer" };
    }
    public long TargetHwnd { get { return _targetHwnd; } }
    public long TargetPid { get { return _targetPid; } }
    public static void CaptureFilteredMessage(int message, long hwnd) {
        _filterQpc = Stopwatch.GetTimestamp(); _filterHwnd = hwnd; _filterMessage = message;
    }
    public void StartWinForms() {
        _form = new ProbeForm(this); _form.Text = "Sky Auto Player — scheduler pump prototype v1 — WinForms";
        _form.Width = 640; _form.Height = 160; _form.StartPosition = FormStartPosition.CenterScreen;
        _label = new Label(); _label.Dock = DockStyle.Fill; _label.TextAlign = System.Drawing.ContentAlignment.MiddleCenter;
        _label.Font = new System.Drawing.Font("Segoe UI", 11);
        _label.Text = "Diagnostic only. Keep this project-owned window foreground during the run.";
        _form.Controls.Add(_label);
        _form.Shown += delegate {
            _form.Activate(); _form.Focus(); SchedulerPumpProbeNative.SetForegroundWindow(_form.Handle);
            InitializeIdentity(_form.Handle);
            _timer = new System.Windows.Forms.Timer(); _timer.Interval = _load ? 250 : 1000;
            _timer.Tick += delegate { TimerCallback(); }; _timer.Start();
            if (_load) StartNonKeyPoster();
            StartSource();
        };
        var filter = new ProbeMessageFilter();
        Application.AddMessageFilter(filter);
        Application.Run(_form);
        Application.RemoveMessageFilter(filter);
        if (_timer != null) _timer.Dispose();
        StopAndDrain();
    }
    public void StartNative() {
        _nativeContext = this; _nativeThreadId = SchedulerPumpProbeNative.GetCurrentThreadId();
        _nativeWndProc = NativeWindowProc;
        string className = "SkySchedulerPumpProbe" + Guid.NewGuid().ToString("N");
        var wc = new SchedulerPumpProbeNative.WndClassEx {
            Size = (uint)Marshal.SizeOf(typeof(SchedulerPumpProbeNative.WndClassEx)),
            WndProc = _nativeWndProc, Instance = SchedulerPumpProbeNative.GetModuleHandle(null),
            ClassName = className, Background = new IntPtr(6)
        };
        if (SchedulerPumpProbeNative.RegisterClassEx(ref wc) == 0) throw new InvalidOperationException("RegisterClassEx failed");
        _nativeHwnd = SchedulerPumpProbeNative.CreateWindowEx(0, className,
            "Sky Auto Player — scheduler pump prototype v1 — native Win32",
            SchedulerPumpProbeNative.WS_OVERLAPPEDWINDOW, 200, 180, 640, 160,
            IntPtr.Zero, IntPtr.Zero, wc.Instance, IntPtr.Zero);
        if (_nativeHwnd == IntPtr.Zero) throw new InvalidOperationException("CreateWindowEx failed");
        SchedulerPumpProbeNative.ShowWindow(_nativeHwnd, SchedulerPumpProbeNative.SW_SHOW);
        SchedulerPumpProbeNative.UpdateWindow(_nativeHwnd);
        SchedulerPumpProbeNative.SetForegroundWindow(_nativeHwnd);
        InitializeIdentity(_nativeHwnd);
        SchedulerPumpProbeNative.SetTimer(_nativeHwnd, new UIntPtr(1), _load ? 250U : 1000U, IntPtr.Zero);
        if (_load) {
            StartNonKeyPoster();
        }
        StartSource();
        SchedulerPumpProbeNative.Msg message;
        int result;
        while ((result = SchedulerPumpProbeNative.GetMessage(out message, IntPtr.Zero, 0, 0)) > 0) {
            _nativeRetrievedQpc = Stopwatch.GetTimestamp();
            SchedulerPumpProbeNative.TranslateMessage(ref message);
            SchedulerPumpProbeNative.DispatchMessage(ref message);
        }
        SchedulerPumpProbeNative.KillTimer(_nativeHwnd, new UIntPtr(1));
        SchedulerPumpProbeNative.DestroyWindow(_nativeHwnd);
        StopAndDrain();
    }
    private void InitializeIdentity(IntPtr hwnd) {
        _targetHwnd = hwnd.ToInt64(); _targetPid = Process.GetCurrentProcess().Id;
        long deadline = Stopwatch.GetTimestamp() + 60L * Stopwatch.Frequency;
        while (!IsTargetForeground() && Stopwatch.GetTimestamp() < deadline) {
            SchedulerPumpProbeNative.SetForegroundWindow(hwnd);
            Thread.Sleep(20);
        }
        if (!IsTargetForeground()) {
            IntPtr foreground = SchedulerPumpProbeNative.GetForegroundWindow(); uint foregroundPid;
            SchedulerPumpProbeNative.GetWindowThreadProcessId(foreground, out foregroundPid);
            throw new InvalidOperationException(String.Format(CultureInfo.InvariantCulture,
                "probe window failed strict foreground identity check (target HWND/PID={0}/{1}, foreground HWND/PID={2}/{3})",
                _targetHwnd, _targetPid, foreground.ToInt64(), foregroundPid));
        }
        _startQpc = Stopwatch.GetTimestamp(); _lastSecondQpc = _startQpc;
        _writer.Start();
    }
    private bool IsTargetForeground() {
        uint pid; SchedulerPumpProbeNative.GetWindowThreadProcessId(new IntPtr(_targetHwnd), out pid);
        return SchedulerPumpProbeNative.GetForegroundWindow().ToInt64() == _targetHwnd && pid == (uint)_targetPid;
    }
    private void StartSource() {
        var producer = new Thread(Produce) { IsBackground = true, Name = "pump prototype source" };
        producer.Start();
    }
    private void StartNonKeyPoster() {
        var poster = new Thread(delegate() {
            long end = Stopwatch.GetTimestamp() + (long)_durationSeconds * Stopwatch.Frequency;
            while (Stopwatch.GetTimestamp() < end) {
                SchedulerPumpProbeNative.PostMessage(new IntPtr(_targetHwnd), WM_NONKEY, IntPtr.Zero, IntPtr.Zero);
                Thread.Sleep(20);
            }
        }) { IsBackground = true, Name = "pump prototype non-key load" };
        poster.Start();
    }
    private void Produce() {
        long deadline = Stopwatch.GetTimestamp() + (long)_durationSeconds * Stopwatch.Frequency;
        long period = Math.Max(1, Stopwatch.Frequency / (_source == "PostMessage" ? 600 : 263));
        long next = Stopwatch.GetTimestamp();
        bool down = true;
        while (Stopwatch.GetTimestamp() < deadline) {
            long focusStart = Stopwatch.GetTimestamp();
            bool foreground = IsTargetForeground();
            long focusElapsed = Stopwatch.GetTimestamp() - focusStart;
            Interlocked.Increment(ref _focusCheckCount);
            Interlocked.Add(ref _focusCheckTotalUs, ToUs(focusElapsed));
            if (!foreground) { Interlocked.Increment(ref _wrongIdentity); break; }
            long callStart = Stopwatch.GetTimestamp();
            if (_source == "PostMessage") {
                int message = down ? SchedulerPumpProbeNative.WM_KEYDOWN : SchedulerPumpProbeNative.WM_KEYUP;
                long lParam = 1L | (0x1eL << 16);
                if (!down) lParam |= (1L << 30) | (1L << 31);
                Interlocked.Increment(ref _sourceRequested);
                Interlocked.Increment(ref _sourceAttempts);
                bool accepted = SchedulerPumpProbeNative.PostMessage(new IntPtr(_targetHwnd), (uint)message, new IntPtr(0x41), new IntPtr(lParam));
                if (accepted) { Interlocked.Increment(ref _sourceInserted); Interlocked.Increment(ref _postAccepted); }
                else { Interlocked.Increment(ref _postRejected); }
            } else {
                var inputs = new SchedulerPumpProbeNative.Input[2];
                inputs[0].Type = SchedulerPumpProbeNative.INPUT_KEYBOARD;
                inputs[0].Data.Keyboard.VirtualKey = 0x41;
                inputs[1].Type = SchedulerPumpProbeNative.INPUT_KEYBOARD;
                inputs[1].Data.Keyboard.VirtualKey = 0x41;
                inputs[1].Data.Keyboard.Flags = SchedulerPumpProbeNative.KEYEVENTF_KEYUP;
                Interlocked.Add(ref _sourceRequested, 2);
                Interlocked.Increment(ref _sourceAttempts);
                uint inserted = SchedulerPumpProbeNative.SendInput(2, inputs, Marshal.SizeOf(typeof(SchedulerPumpProbeNative.Input)));
                Interlocked.Add(ref _sourceInserted, inserted);
                if (inserted != 2) {
                    Interlocked.Increment(ref _postRejected);
                    if (inserted == 1 && IsTargetForeground()) {
                        var release = new SchedulerPumpProbeNative.Input[1];
                        release[0].Type = SchedulerPumpProbeNative.INPUT_KEYBOARD;
                        release[0].Data.Keyboard.VirtualKey = 0x41;
                        release[0].Data.Keyboard.Flags = SchedulerPumpProbeNative.KEYEVENTF_KEYUP;
                        SchedulerPumpProbeNative.SendInput(1, release, Marshal.SizeOf(typeof(SchedulerPumpProbeNative.Input)));
                    }
                    break;
                }
            }
            long callEnd = Stopwatch.GetTimestamp();
            Interlocked.Increment(ref _sourceCompletions);
            AddLatency(_sourceCallUs, ref _sourceCallCount, ToUs(callEnd - callStart));
            down = !down;
            next += period;
            while (Stopwatch.GetTimestamp() < next) Thread.Sleep(0);
        }
        _sourceCompletionQpc = Stopwatch.GetTimestamp();
        if (_pump == "Native") SchedulerPumpProbeNative.PostThreadMessage(_nativeThreadId, 0x0012, IntPtr.Zero, IntPtr.Zero);
        else if (_form != null && !_form.IsDisposed) {
            try { _form.BeginInvoke((MethodInvoker)delegate { _form.Close(); }); } catch { }
        }
    }
    private void AddLatency(long[] samples, ref int count, long value) {
        lock (_latencyGate) { if (count < samples.Length) samples[count++] = value; }
    }
    private long ToUs(long ticks) { return (long)(((decimal)ticks * 1000000m) / Stopwatch.Frequency); }
    public void Observe(uint message, IntPtr wParam, IntPtr lParam, long retrievedQpc, long messageHwnd) {
        long start = Stopwatch.GetTimestamp();
        long focusStart = Stopwatch.GetTimestamp();
        IntPtr foreground = SchedulerPumpProbeNative.GetForegroundWindow();
        uint fgPid; SchedulerPumpProbeNative.GetWindowThreadProcessId(foreground, out fgPid);
        long focusUs = ToUs(Stopwatch.GetTimestamp() - focusStart);
        int vk = unchecked((int)(wParam.ToInt64() & 0xffff));
        long nativeLParam = lParam.ToInt64();
        bool identity = messageHwnd == _targetHwnd && (long)foreground == _targetHwnd && (long)fgPid == _targetPid;
        if (!identity) Interlocked.Increment(ref _wrongIdentity);
        uint queueStatus = SchedulerPumpProbeNative.GetQueueStatus(SchedulerPumpProbeNative.QS_KEY);
        long end;
        var item = new SchedulerPumpEvent {
            RetrievedQpc = retrievedQpc, HandlerStartQpc = start, ForegroundCheckUs = focusUs,
            Hwnd = messageHwnd, Pid = (int)_targetPid, ForegroundHwnd = foreground.ToInt64(), ForegroundPid = (int)fgPid,
            Message = (int)message, VirtualKey = vk, ScanCode = (int)((nativeLParam >> 16) & 0xff),
            Extended = ((nativeLParam >> 24) & 1) != 0, QueueStatus = queueStatus
        };
        long lagUs = retrievedQpc > 0 && start >= retrievedQpc ? ToUs(start - retrievedQpc) : -1;
        AddLatency(_dispatchLagUs, ref _dispatchCount, lagUs);
        AddLatency(_foregroundUs, ref _foregroundCount, focusUs);
        if (_pump == "WinForms" && _form != null) {
            if (((_ring.EventsSeen + 1) % 64) == 0) {
                long begin = Stopwatch.GetTimestamp();
                try {
                    _form.BeginInvoke((MethodInvoker)delegate {
                        Interlocked.Increment(ref _beginInvokeCompleted);
                        if (_label != null && !_label.IsDisposed) _label.Text = "Diagnostic only — scheduler-pump-prototype-v1";
                    });
                    Interlocked.Increment(ref _beginInvokeCount);
                    Interlocked.Add(ref _beginInvokeTotalUs, ToUs(Stopwatch.GetTimestamp() - begin));
                } catch { Interlocked.Increment(ref _wrongIdentity); }
            }
        }
        _ring.TryEnqueue(item);
        end = Stopwatch.GetTimestamp();
        AddLatency(_handlerUs, ref _handlerCount, ToUs(end - start));
    }
    public void NonKeyCallback() { Interlocked.Increment(ref _pumpNonKey); }
    public void TimerCallback() {
        Interlocked.Increment(ref _timerCallbacks);
        long start = Stopwatch.GetTimestamp();
        if (_load) SchedulerPumpProbeContext.SuppressOverlayWindows((uint)_targetPid);
        long elapsed = ToUs(Stopwatch.GetTimestamp() - start);
        Interlocked.Add(ref _overlayTotalUs, elapsed);
        long old = Interlocked.Read(ref _overlayMaxUs);
        while (elapsed > old) { long seen = Interlocked.CompareExchange(ref _overlayMaxUs, elapsed, old); if (seen == old) break; old = seen; }
        if (Stopwatch.GetTimestamp() - _lastSecondQpc >= Stopwatch.Frequency) {
            CaptureSecond(); _lastSecondQpc = Stopwatch.GetTimestamp();
        }
    }
    private void CaptureSecond() {
        object ring = _ring.Snapshot();
        long seen = GetLong(ring, "seen");
        long written = GetLong(ring, "written");
        long flushed = GetLong(ring, "flushed");
        uint queueStatus = SchedulerPumpProbeNative.GetQueueStatus(SchedulerPumpProbeNative.QS_KEY);
        lock (_secondGate) _perSecond.Add(String.Format(CultureInfo.InvariantCulture,
            "{{\"elapsed_seconds\":{0},\"source_requested\":{1},\"source_inserted\":{2},\"source_attempts\":{3},\"source_completions\":{4},\"wndproc_seen\":{5},\"fifo_enqueued\":{6},\"writer_written\":{7},\"writer_flushed\":{8},\"queue_status\":{9},\"queue_nonempty_sample\":{10}}}",
            _perSecond.Count + 1, Interlocked.Read(ref _sourceRequested), Interlocked.Read(ref _sourceInserted),
            Interlocked.Read(ref _sourceAttempts), Interlocked.Read(ref _sourceCompletions), seen,
            GetLong(ring, "enqueued"), written, flushed,
            (queueStatus >> 16), ((queueStatus >> 16) & 0xffffU) != 0 ? "true" : "false"));
    }
    private void Consume() {
        int unflushed = 0;
        try {
            using (var writer = new StreamWriter(_eventPath, false, new UTF8Encoding(false), 65536)) {
                writer.WriteLine("{\"schema_version\":1,\"observer_version\":\"" + ObserverVersion + "\",\"run_id\":\"" + _runId + "\",\"qualification\":false,\"kind\":\"stream_start\",\"pump\":\"" + _pump + "\",\"source\":\"" + _source + "\",\"diagnostic_load\":" + (_load ? "true" : "false") + ",\"qpc_frequency_hz\":" + Stopwatch.Frequency.ToString(CultureInfo.InvariantCulture) + ",\"target_hwnd\":" + _targetHwnd.ToString(CultureInfo.InvariantCulture) + ",\"target_pid\":" + _targetPid.ToString(CultureInfo.InvariantCulture) + "}"); writer.Flush();
                SchedulerPumpEvent item;
                while (_ring.TryDequeue(out item)) {
                    writer.WriteLine(String.Format(CultureInfo.InvariantCulture,
                        "{{\"schema_version\":1,\"observer_version\":\"{0}\",\"sequence\":{1},\"kind\":\"{2}\",\"message\":{3},\"virtual_key\":{4},\"scan_code\":{5},\"extended\":{6},\"observed_qpc_ticks\":{7},\"queue_retrieved_qpc_ticks\":{8},\"wndproc_start_qpc_ticks\":{9},\"wndproc_end_qpc_ticks\":{10},\"foreground_check_us\":{11},\"target_hwnd\":{12},\"target_pid\":{13},\"foreground_hwnd\":{14},\"foreground_pid\":{15},\"queue_status\":{16},\"run_id\":\"{17}\",\"qualification\":false}}",
                        ObserverVersion, item.Sequence, (item.Message == SchedulerPumpProbeNative.WM_KEYDOWN || item.Message == SchedulerPumpProbeNative.WM_SYSKEYDOWN) ? "key_down" : "key_up",
                        item.Message, item.VirtualKey, item.ScanCode, item.Extended ? "true" : "false", item.HandlerStartQpc,
                        item.RetrievedQpc, item.HandlerStartQpc, item.HandlerEndQpc, item.ForegroundCheckUs,
                        item.Hwnd, item.Pid, item.ForegroundHwnd, item.ForegroundPid, item.QueueStatus, _runId));
                    _ring.Written(); unflushed++;
                    if (unflushed >= 64) { writer.Flush(); _ring.Flushed(unflushed); unflushed = 0; }
                }
                writer.Flush(); if (unflushed > 0) _ring.Flushed(unflushed);
            }
        } catch { _ring.WriterFailed(); }
    }
    public void StopAndDrain() {
        if (!_writer.IsAlive && _writer.ThreadState == System.Threading.ThreadState.Unstarted) return;
        _ring.Complete();
        if (!_writer.Join(30000)) throw new TimeoutException("diagnostic writer did not drain its bounded FIFO");
    }
    private static IntPtr NativeWindowProc(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam) {
        SchedulerPumpProbeContext context = _nativeContext;
        if (context == null) return SchedulerPumpProbeNative.DefWindowProc(hwnd, message, wParam, lParam);
        if (message == SchedulerPumpProbeNative.WM_TIMER) { context.TimerCallback(); return IntPtr.Zero; }
        if (message == WM_NONKEY) { context.NonKeyCallback(); return IntPtr.Zero; }
        if (message == SchedulerPumpProbeNative.WM_KEYDOWN || message == SchedulerPumpProbeNative.WM_KEYUP ||
            message == SchedulerPumpProbeNative.WM_SYSKEYDOWN || message == SchedulerPumpProbeNative.WM_SYSKEYUP) {
            context.Observe(message, wParam, lParam, _nativeRetrievedQpc, hwnd.ToInt64());
            return IntPtr.Zero;
        }
        return SchedulerPumpProbeNative.DefWindowProc(hwnd, message, wParam, lParam);
    }
    public static void SuppressOverlayWindows(uint pid) {
        SchedulerPumpProbeNative.EnumWindows(delegate(IntPtr hwnd, IntPtr parameter) {
            uint owner; SchedulerPumpProbeNative.GetWindowThreadProcessId(hwnd, out owner);
            if (owner == pid) {
                var className = new StringBuilder(256);
                SchedulerPumpProbeNative.GetClassName(hwnd, className, className.Capacity);
                string name = className.ToString();
                if (name.StartsWith("UAC", StringComparison.Ordinal) || name.StartsWith("Touch Tooltip", StringComparison.Ordinal))
                    SchedulerPumpProbeNative.ShowWindowAsync(hwnd, 0);
            }
            return true;
        }, IntPtr.Zero);
    }
    private long Percentile(long[] values, int count, double percentile) {
        long[] copy; lock (_latencyGate) { copy = new long[count]; Array.Copy(values, copy, count); }
        if (copy.Length == 0) return -1; Array.Sort(copy);
        return copy[Math.Min(copy.Length - 1, Math.Max(0, (int)Math.Ceiling(percentile * copy.Length) - 1))];
    }
    private string Percentiles(long[] values, int count) {
        return String.Format(CultureInfo.InvariantCulture, "{{\"count\":{0},\"p50_us\":{1},\"p95_us\":{2},\"p99_us\":{3},\"max_us\":{4}}}",
            count, Percentile(values, count, 0.50), Percentile(values, count, 0.95), Percentile(values, count, 0.99), Percentile(values, count, 1.0));
    }
    public string SummaryJson() {
        object ring = _ring.Snapshot();
        var perSecond = new StringBuilder(); lock (_secondGate) perSecond.Append(String.Join(",", _perSecond.ToArray()));
        return String.Format(CultureInfo.InvariantCulture,
            "{{\"schema_version\":1,\"observer_version\":\"{0}\",\"qualification\":false,\"pump\":\"{1}\",\"source\":\"{2}\",\"diagnostic_load\":{3},\"duration_seconds\":{4},\"qpc_frequency_hz\":{5},\"target_hwnd\":{6},\"target_pid\":{7},\"source_requested\":{8},\"source_inserted\":{9},\"source_attempts\":{10},\"source_completions\":{11},\"post_accepted\":{12},\"post_rejected\":{13},\"source_completion_qpc_ticks\":{14},\"wndproc_seen\":{15},\"fifo_enqueued\":{16},\"writer_written\":{17},\"writer_flushed\":{18},\"queue_dropped\":{19},\"queue_high_water\":{20},\"queue_capacity\":{21},\"queue_overflow\":{22},\"writer_failures\":{23},\"wrong_identity\":{24},\"pump_nonkey_callbacks\":{25},\"timer_callbacks\":{26},\"overlay_total_us\":{27},\"overlay_max_us\":{28},\"begin_invoke_calls\":{29},\"begin_invoke_completed\":{30},\"begin_invoke_total_us\":{31},\"foreground_check_count\":{32},\"foreground_check_total_us\":{33},\"dispatch_lag\":{34},\"wndproc_handler\":{35},\"foreground_check\":{36},\"source_call\":{37},\"per_second\":[{38}],\"raw_event_log\":\"{39}\",\"run_id\":\"{40}\"}}",
            ObserverVersion, _pump, _source, _load ? "true" : "false", _durationSeconds, Stopwatch.Frequency,
            _targetHwnd, _targetPid, Interlocked.Read(ref _sourceRequested), Interlocked.Read(ref _sourceInserted),
            Interlocked.Read(ref _sourceAttempts), Interlocked.Read(ref _sourceCompletions), Interlocked.Read(ref _postAccepted),
            Interlocked.Read(ref _postRejected), _sourceCompletionQpc,
            GetLong(ring, "seen"), GetLong(ring, "enqueued"), GetLong(ring, "written"), GetLong(ring, "flushed"),
            GetLong(ring, "dropped"), GetLong(ring, "high_water"), GetInt(ring, "capacity"), GetBool(ring, "overflow") ? "true" : "false",
            GetLong(ring, "writer_failures"), Interlocked.Read(ref _wrongIdentity),
            Interlocked.Read(ref _pumpNonKey), Interlocked.Read(ref _timerCallbacks), Interlocked.Read(ref _overlayTotalUs),
            Interlocked.Read(ref _overlayMaxUs), Interlocked.Read(ref _beginInvokeCount), Interlocked.Read(ref _beginInvokeCompleted), Interlocked.Read(ref _beginInvokeTotalUs),
            Interlocked.Read(ref _focusCheckCount), Interlocked.Read(ref _focusCheckTotalUs),
            Percentiles(_dispatchLagUs, _dispatchCount), Percentiles(_handlerUs, _handlerCount),
            Percentiles(_foregroundUs, _foregroundCount), Percentiles(_sourceCallUs, _sourceCallCount),
            perSecond.ToString(), _eventPath.Replace("\\", "\\\\").Replace("\"", "\\\""), _runId);
    }
    private sealed class ProbeMessageFilter : IMessageFilter {
        public bool PreFilterMessage(ref Message message) {
            CaptureFilteredMessage(message.Msg, message.HWnd.ToInt64()); return false;
        }
    }
    private static long GetLong(object value, string name) { return Convert.ToInt64(value.GetType().GetProperty(name).GetValue(value, null), CultureInfo.InvariantCulture); }
    private static int GetInt(object value, string name) { return Convert.ToInt32(value.GetType().GetProperty(name).GetValue(value, null), CultureInfo.InvariantCulture); }
    private static bool GetBool(object value, string name) { return Convert.ToBoolean(value.GetType().GetProperty(name).GetValue(value, null), CultureInfo.InvariantCulture); }
    public sealed class ProbeForm : Form {
        private readonly SchedulerPumpProbeContext _context;
        public ProbeForm(SchedulerPumpProbeContext context) { _context = context; }
        protected override void WndProc(ref Message message) {
            if (message.Msg == SchedulerPumpProbeNative.WM_KEYDOWN || message.Msg == SchedulerPumpProbeNative.WM_KEYUP ||
                message.Msg == SchedulerPumpProbeNative.WM_SYSKEYDOWN || message.Msg == SchedulerPumpProbeNative.WM_SYSKEYUP) {
                long retrieved = _filterMessage == message.Msg && _filterHwnd == message.HWnd.ToInt64() ? _filterQpc : 0;
                _context.Observe((uint)message.Msg, message.WParam, message.LParam, retrieved, message.HWnd.ToInt64());
            } else if (message.Msg == WM_NONKEY) _context.NonKeyCallback();
            base.WndProc(ref message);
        }
    }
}

public static class SchedulerPumpProbe {
    public static string Run(string pump, string source, bool load, int durationSeconds, string eventPath, string runId) {
        var context = new SchedulerPumpProbeContext(pump, source, load, durationSeconds, eventPath, runId);
        if (pump == "Native") context.StartNative(); else context.StartWinForms();
        return context.SummaryJson();
    }
}

"@

if ($CompileOnly) {
    Write-Output 'scheduler-receiveonly-pump-prototype-v1 C# support: COMPILE PASS'
    exit 0
}
$runId = 'scheduler-pump-prototype-' + $Pump.ToLowerInvariant() + '-' + $Source.ToLowerInvariant() + '-' + [guid]::NewGuid().ToString('N').Substring(0, 8)
$eventsPath = Join-Path $probeRoot ($runId + '.jsonl')
$summaryPath = Join-Path $probeRoot ($runId + '.summary.json')
Write-Host 'Foreground gate: keep the visible project-owned scheduler pump probe window active; no input is sent until exact HWND/PID identity matches.'
try {
    $summaryJson = [SchedulerPumpProbe]::Run($Pump, $Source, [bool]$DiagnosticLoad, $DurationSeconds, $eventsPath, $runId)
} catch {
    $failureMessage = $_.Exception.Message
    $foregroundMatch = [regex]::Match($failureMessage, 'target HWND/PID=(\d+)/(\d+), foreground HWND/PID=(\d+)/(\d+)')
    $failureRecord = [ordered]@{
        schema_version = 1
        observer_version = 'scheduler-receiveonly-pump-prototype-v1'
        qualification = $false
        run_id = $runId
        pump = $Pump
        source = $Source
        diagnostic_load = [bool]$DiagnosticLoad
        duration_seconds = $DurationSeconds
        failure_phase = if ($failureMessage -match 'probe window failed strict foreground identity check') { 'foreground_identity_start_gate' } else { 'probe_startup_or_runtime' }
        source_started = if ($failureMessage -match 'probe window failed strict foreground identity check') { $false } else { $null }
        source_requested = 0
        source_inserted = 0
        source_attempts = 0
        wndproc_seen = 0
        raw_event_log = if (Test-Path -LiteralPath $eventsPath) { $eventsPath } else { $null }
        target_hwnd = if ($foregroundMatch.Success) { [long]$foregroundMatch.Groups[1].Value } else { $null }
        target_pid = if ($foregroundMatch.Success) { [int]$foregroundMatch.Groups[2].Value } else { $null }
        foreground_hwnd = if ($foregroundMatch.Success) { [long]$foregroundMatch.Groups[3].Value } else { $null }
        foreground_pid = if ($foregroundMatch.Success) { [int]$foregroundMatch.Groups[4].Value } else { $null }
        failure = $failureMessage
        source_revision = (& git -C $repositoryRoot rev-parse HEAD).Trim()
        source_tree_sha = (& git -C $repositoryRoot show -s --format=%T HEAD).Trim()
    }
    $failureRecord | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $summaryPath -Encoding utf8
    throw
}
Set-Content -LiteralPath $summaryPath -Value $summaryJson -Encoding utf8
$summary = $summaryJson | ConvertFrom-Json
$records = @(Get-Content -LiteralPath $eventsPath | ForEach-Object { $_ | ConvertFrom-Json })
if ($records.Count -lt 1 -or $records[0].kind -ne 'stream_start' -or
    $records[0].observer_version -ne 'scheduler-receiveonly-pump-prototype-v1' -or
    $records[0].run_id -ne $runId -or [bool]$records[0].qualification) {
    throw 'Diagnostic event stream is missing distinct prototype provenance'
}
$previousQpc = -1L
for ($index = 1; $index -lt $records.Count; $index++) {
    $event = $records[$index]
    $expectedKind = if ([int]$event.message -in @(0x0100, 0x0104)) { 'key_down' } else { 'key_up' }
    if ($event.schema_version -ne 1 -or $event.observer_version -ne 'scheduler-receiveonly-pump-prototype-v1' -or
        $event.run_id -ne $runId -or $event.sequence -ne $index -or $event.kind -ne $expectedKind -or
        $event.virtual_key -le 0 -or $event.message -notin @(0x0100, 0x0101, 0x0104, 0x0105) -or
        $event.target_hwnd -ne $summary.target_hwnd -or $event.target_pid -ne $summary.target_pid -or
        $event.foreground_hwnd -ne $summary.target_hwnd -or $event.foreground_pid -ne $summary.target_pid -or
        $event.observed_qpc_ticks -lt $previousQpc) {
        throw ('Diagnostic event stream identity, order, or metadata failed at sequence ' + $index)
    }
    $previousQpc = [long]$event.observed_qpc_ticks
}
if ($summary.queue_overflow -or $summary.queue_dropped -ne 0 -or $summary.writer_failures -ne 0 -or
    $summary.wrong_identity -ne 0 -or $summary.post_rejected -ne 0 -or
    $summary.source_inserted -ne $summary.source_requested -or
    $summary.wndproc_seen -ne $summary.fifo_enqueued -or
    $summary.fifo_enqueued -ne $summary.writer_written -or
    $summary.writer_written -ne $summary.writer_flushed -or
    $summary.queue_high_water -gt $summary.queue_capacity -or $summary.queue_high_water -lt 0) {
    throw 'Diagnostic probe failed closed: inspect summary and raw event log'
}
Write-Output $summaryJson
