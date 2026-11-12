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
    [switch]$CompileOnly,
    [switch]$SelfTest,
    [switch]$PreflightOnly,
    [ValidateRange(5, 120)]
    [int]$ForegroundTimeoutSeconds = 60
)

$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($Pump) -or [string]::IsNullOrWhiteSpace($Source) -or
    [string]::IsNullOrWhiteSpace($OutputRoot)) {
    throw 'Pump, Source, and OutputRoot are required'
}
if ($PreflightOnly -and $Source -ne 'PostMessage') {
    throw 'PreflightOnly requires -Source PostMessage; it never starts a source producer'
}
$sendInputGuardMessage = 'SendInput diagnostic mode is disabled before probe startup; no SendInput call was made. A reviewed physical-key obligation and cleanup design is required first.'
if ($Source -eq 'SendInput' -and -not ($CompileOnly -or $SelfTest)) {
    throw $sendInputGuardMessage
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
    public const int WM_PAINT = 0x000F;
    public const int WM_ACTIVATE = 0x0006;
    public const int WM_DESTROY = 0x0002;
    public const int QS_KEY = 0x0001;
    public const uint PM_REMOVE = 0x0001;
    public const uint WS_OVERLAPPEDWINDOW = 0x00CF0000;
    public const int SW_SHOW = 5;

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

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern ushort RegisterClassEx(ref WndClassEx windowClass);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern IntPtr CreateWindowEx(uint exStyle, string className, string windowName,
        uint style, int x, int y, int width, int height, IntPtr parent, IntPtr menu,
        IntPtr instance, IntPtr parameter);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hwnd, int command);
    [DllImport("user32.dll")] public static extern bool UpdateWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);
    [DllImport("user32.dll")] public static extern int GetMessage(out Msg message, IntPtr hwnd, uint min, uint max);
    [DllImport("user32.dll")] public static extern bool TranslateMessage(ref Msg message);
    [DllImport("user32.dll")] public static extern IntPtr DispatchMessage(ref Msg message);
    [DllImport("user32.dll")] public static extern IntPtr DefWindowProc(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll")] public static extern void PostQuitMessage(int exitCode);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool PostThreadMessage(uint threadId, uint message, IntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll")] public static extern uint GetQueueStatus(uint flags);
    [DllImport("user32.dll")] public static extern IntPtr SetTimer(IntPtr hwnd, UIntPtr id, uint milliseconds, IntPtr callback);
    [DllImport("user32.dll")] public static extern bool KillTimer(IntPtr hwnd, UIntPtr id);
    [DllImport("user32.dll")] public static extern bool DestroyWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsDelegate callback, IntPtr parameter);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(IntPtr hwnd, StringBuilder name, int maximum);
    [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr hwnd, int command);
    [DllImport("kernel32.dll")] public static extern IntPtr GetModuleHandle(string name);
    [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
}

public struct SchedulerPumpEvent {
    public long Sequence;
    public long RetrievedQpc;
    public string RetrievalStatus;
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

public enum PumpForegroundState { Waiting, Armed, TimedOut, TargetUnavailable }
public sealed class PumpForegroundArmMachine {
    private readonly long _deadlineQpc;
    public PumpForegroundState State { get; private set; }
    public string Reason { get; private set; }
    public long MismatchChecks { get; private set; }
    public PumpForegroundArmMachine(long deadlineQpc) {
        _deadlineQpc = deadlineQpc; State = PumpForegroundState.Waiting; Reason = "awaiting_foreground";
    }
    public PumpForegroundState Observe(long nowQpc, bool targetValid, bool targetOwnedByProcess,
        bool targetVisible, bool targetEnabled, bool foregroundHwndMatches, bool foregroundPidMatches) {
        if (State != PumpForegroundState.Waiting) return State;
        if (nowQpc >= _deadlineQpc) {
            State = PumpForegroundState.TimedOut; Reason = "foreground_timeout:" + Reason; return State;
        }
        if (!targetValid || !targetOwnedByProcess || !targetVisible || !targetEnabled) {
            State = PumpForegroundState.TargetUnavailable; Reason = "target_not_valid_visible_enabled_owned"; return State;
        }
        if (!foregroundHwndMatches || !foregroundPidMatches) {
            MismatchChecks++; Reason = "foreground_identity_mismatch"; return State;
        }
        State = PumpForegroundState.Armed; Reason = "exact_foreground_identity_verified"; return State;
    }
}

public enum PumpRetrievalStatus { Unknown, Unmatched, Matched }
public sealed class PumpRetrievalMatcher {
    private bool _active, _consumed;
    private long _hwnd, _lParam, _qpc;
    private uint _message;
    private ulong _wParam;
    public void Begin(long hwnd, uint message, ulong wParam, long lParam, long qpc) {
        _active = true; _consumed = false; _hwnd = hwnd; _message = message;
        _wParam = wParam; _lParam = lParam; _qpc = qpc;
    }
    public void Clear() { _active = false; _consumed = false; _qpc = 0; }
    public PumpRetrievalStatus TryMatch(long hwnd, uint message, ulong wParam, long lParam, out long qpc) {
        qpc = 0;
        if (!_active) return PumpRetrievalStatus.Unknown;
        if (_consumed || _hwnd != hwnd || _message != message || _wParam != wParam || _lParam != lParam)
            return PumpRetrievalStatus.Unmatched;
        _consumed = true; qpc = _qpc; return PumpRetrievalStatus.Matched;
    }
}

public static class PumpProbeSourcePolicy {
    public static bool CanStart(string source, bool foregroundArmed, bool preflightOnly) {
        return source == "PostMessage" && foregroundArmed && !preflightOnly;
    }
}

public static class SchedulerPumpProbeContractTests {
    private static void Require(bool value, string message) {
        if (!value) throw new InvalidOperationException("pump probe contract failed: " + message);
    }
    public static string Run() {
        var timeout = new PumpForegroundArmMachine(20);
        Require(timeout.Observe(10, true, true, true, true, false, false) == PumpForegroundState.Waiting, "foreground mismatch waits for handoff");
        Require(timeout.Observe(20, true, true, true, true, true, true) == PumpForegroundState.TimedOut, "timeout fails closed");
        Require(timeout.MismatchChecks == 1, "mismatch counted");

        var mismatch = new PumpForegroundArmMachine(30);
        Require(mismatch.Observe(1, true, true, true, true, true, false) == PumpForegroundState.Waiting, "wrong foreground PID does not arm");
        Require(mismatch.Observe(2, true, false, true, true, true, true) == PumpForegroundState.TargetUnavailable, "wrong target ownership fails closed");

        var preflight = new PumpForegroundArmMachine(30);
        bool preflightArmed = preflight.Observe(1, true, true, true, true, true, true) == PumpForegroundState.Armed;
        int sourceStarts = 0;
        if (PumpProbeSourcePolicy.CanStart("PostMessage", preflightArmed, true)) sourceStarts++;
        Require(preflightArmed && sourceStarts == 0, "preflight arms identity but leaves source stopped");

        var success = new PumpForegroundArmMachine(30);
        bool exactArm = success.Observe(1, true, true, true, true, true, true) == PumpForegroundState.Armed;
        Require(exactArm && PumpProbeSourcePolicy.CanStart("PostMessage", exactArm, false), "exact foreground permits PostMessage source");
        int sendInputCalls = 0;
        if (PumpProbeSourcePolicy.CanStart("SendInput", exactArm, false)) sendInputCalls++;
        Require(!PumpProbeSourcePolicy.CanStart("SendInput", exactArm, false) && sendInputCalls == 0, "SendInput mode is blocked before any API call");

        var matcher = new PumpRetrievalMatcher();
        long qpc;
        Require(matcher.TryMatch(1, 0x100, 0x41, 7, out qpc) == PumpRetrievalStatus.Unknown, "sent/reentrant callback without queue retrieval is unknown");
        matcher.Begin(1, 0x100, 0x41, 7, 100);
        Require(matcher.TryMatch(1, 0x101, 0x41, 7, out qpc) == PumpRetrievalStatus.Unmatched, "wrong message does not borrow retrieval timestamp");
        Require(matcher.TryMatch(1, 0x100, 0x42, 7, out qpc) == PumpRetrievalStatus.Unmatched, "wrong wParam does not borrow retrieval timestamp");
        Require(matcher.TryMatch(1, 0x100, 0x41, 7, out qpc) == PumpRetrievalStatus.Matched && qpc == 100, "exact queue tuple matches once");
        Require(matcher.TryMatch(1, 0x100, 0x41, 7, out qpc) == PumpRetrievalStatus.Unmatched, "reentrant duplicate cannot reuse consumed timestamp");
        matcher.Clear();
        Require(matcher.TryMatch(1, 0x100, 0x41, 7, out qpc) == PumpRetrievalStatus.Unknown, "cleared retrieval becomes unknown");
        return "foreground state, source gate, SendInput block, and retrieval correlation: PASS";
    }
}

public sealed class SchedulerPumpProbeContext {
    public const string ObserverVersion = "scheduler-receiveonly-pump-prototype-v2";
    public const int WM_NONKEY = 0x8001;
    private readonly string _pump, _source, _eventPath, _runId;
    private readonly bool _load;
    private readonly bool _preflightOnly;
    private readonly int _durationSeconds;
    private readonly int _foregroundTimeoutSeconds;
    private readonly SchedulerPumpRing _ring = new SchedulerPumpRing();
    private readonly Thread _writer;
    private readonly PumpRetrievalMatcher _retrievalMatcher = new PumpRetrievalMatcher();
    private readonly long[] _dispatchLagUs = new long[32768];
    private readonly long[] _handlerUs = new long[32768];
    private readonly long[] _foregroundUs = new long[32768];
    private readonly long[] _sourceCallUs = new long[32768];
    private readonly object _latencyGate = new object();
    private int _dispatchCount, _handlerCount, _foregroundCount, _sourceCallCount;
    private long _targetHwnd, _targetPid, _startQpc, _sourceCompletionQpc;
    private long _sourceRequested, _sourceInserted, _sourceAttempts, _sourceCompletions;
    private long _postAccepted, _postRejected, _wrongIdentity, _pumpNonKey, _timerCallbacks;
    private long _foregroundChecks, _foregroundMismatchChecks, _paintMessages, _activationMessages;
    private long _retrievalMatched, _retrievalUnmatched, _retrievalUnknown, _dispatchValidSamples;
    private long _overlayTotalUs, _overlayMaxUs, _beginInvokeCount, _beginInvokeCompleted, _beginInvokeTotalUs;
    private long _focusCheckCount, _focusCheckTotalUs;
    private readonly List<string> _perSecond = new List<string>();
    private readonly object _secondGate = new object();
    private long _lastSecondQpc;
    private Form _form;
    private Label _label;
    private System.Windows.Forms.Timer _timer;
    private Thread _producer, _nonKeyPoster;
    private IntPtr _nativeHwnd;
    private uint _nativeThreadId;
    private PumpForegroundArmMachine _foregroundArm;
    private bool _targetOwnedByProcess, _targetValid, _targetVisible, _targetEnabled;
    private bool _foregroundArmed, _sourceStarted, _preflightPassed;
    private long _lastForegroundHwnd, _foregroundDeadlineQpc, _preflightCloseQpc;
    private int _lastForegroundPid;
    private string _failureReason = "awaiting_foreground";
    private int _closeRequested, _stopSource;
    private static SchedulerPumpProbeContext _nativeContext;
    private static SchedulerPumpProbeNative.WndProcDelegate _nativeWndProc;

    public SchedulerPumpProbeContext(string pump, string source, bool load, bool preflightOnly, int durationSeconds,
        int foregroundTimeoutSeconds, string eventPath, string runId) {
        _pump = pump; _source = source; _load = load; _preflightOnly = preflightOnly;
        _durationSeconds = durationSeconds; _foregroundTimeoutSeconds = foregroundTimeoutSeconds;
        _eventPath = eventPath; _runId = runId;
        _writer = new Thread(Consume) { IsBackground = true, Name = "pump prototype writer" };
    }
    public long TargetHwnd { get { return _targetHwnd; } }
    public long TargetPid { get { return _targetPid; } }
    public void CaptureFilteredMessage(ref Message message) {
        if (IsKeyMessage((uint)message.Msg)) {
            _retrievalMatcher.Begin(message.HWnd.ToInt64(), (uint)message.Msg,
                unchecked((ulong)message.WParam.ToInt64()), message.LParam.ToInt64(), Stopwatch.GetTimestamp());
        } else {
            _retrievalMatcher.Clear();
        }
    }
    public void StartWinForms() {
        _form = new ProbeForm(this); _form.Text = "Sky Auto Player — scheduler pump prototype v2 — WinForms";
        _form.Width = 640; _form.Height = 160; _form.StartPosition = FormStartPosition.CenterScreen;
        _label = new Label(); _label.Dock = DockStyle.Fill; _label.TextAlign = System.Drawing.ContentAlignment.MiddleCenter;
        _label.Font = new System.Drawing.Font("Segoe UI", 11);
        _label.Text = "Diagnostic only. Click this project-owned window or Alt+Tab to it; input remains disarmed until verified.";
        _form.Controls.Add(_label);
        _form.Shown += delegate {
            InitializeTarget(_form.Handle);
            _timer = new System.Windows.Forms.Timer(); _timer.Interval = 100;
            _timer.Tick += delegate { TimerCallback(); }; _timer.Start();
        };
        var filter = new ProbeMessageFilter(this);
        Application.AddMessageFilter(filter);
        Application.Run(_form);
        Application.RemoveMessageFilter(filter);
        if (_timer != null) _timer.Dispose();
        StopSourceThreads();
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
            "Sky Auto Player — scheduler pump prototype v2 — native Win32",
            SchedulerPumpProbeNative.WS_OVERLAPPEDWINDOW, 200, 180, 640, 160,
            IntPtr.Zero, IntPtr.Zero, wc.Instance, IntPtr.Zero);
        if (_nativeHwnd == IntPtr.Zero) throw new InvalidOperationException("CreateWindowEx failed");
        SchedulerPumpProbeNative.ShowWindow(_nativeHwnd, SchedulerPumpProbeNative.SW_SHOW);
        SchedulerPumpProbeNative.UpdateWindow(_nativeHwnd);
        InitializeTarget(_nativeHwnd);
        SchedulerPumpProbeNative.SetTimer(_nativeHwnd, new UIntPtr(1), 100U, IntPtr.Zero);
        SchedulerPumpProbeNative.Msg message;
        int result;
        while ((result = SchedulerPumpProbeNative.GetMessage(out message, IntPtr.Zero, 0, 0)) > 0) {
            SchedulerPumpProbeNative.TranslateMessage(ref message);
            if (IsKeyMessage(message.Message)) {
                _retrievalMatcher.Begin(message.HWnd.ToInt64(), message.Message, message.WParam.ToUInt64(),
                    message.LParam.ToInt64(), Stopwatch.GetTimestamp());
            } else {
                _retrievalMatcher.Clear();
            }
            try { SchedulerPumpProbeNative.DispatchMessage(ref message); }
            finally { _retrievalMatcher.Clear(); }
        }
        SchedulerPumpProbeNative.KillTimer(_nativeHwnd, new UIntPtr(1));
        if (SchedulerPumpProbeNative.IsWindow(_nativeHwnd)) SchedulerPumpProbeNative.DestroyWindow(_nativeHwnd);
        StopSourceThreads();
        StopAndDrain();
    }
    private void InitializeTarget(IntPtr hwnd) {
        _targetHwnd = hwnd.ToInt64();
        uint targetPid; SchedulerPumpProbeNative.GetWindowThreadProcessId(hwnd, out targetPid);
        _targetPid = targetPid;
        _targetOwnedByProcess = targetPid == (uint)Process.GetCurrentProcess().Id;
        _foregroundDeadlineQpc = Stopwatch.GetTimestamp() + (long)_foregroundTimeoutSeconds * Stopwatch.Frequency;
        _foregroundArm = new PumpForegroundArmMachine(_foregroundDeadlineQpc);
        _startQpc = Stopwatch.GetTimestamp(); _lastSecondQpc = _startQpc;
        _writer.Start();
    }
    private bool IsKeyMessage(uint message) {
        return message == SchedulerPumpProbeNative.WM_KEYDOWN || message == SchedulerPumpProbeNative.WM_KEYUP ||
            message == SchedulerPumpProbeNative.WM_SYSKEYDOWN || message == SchedulerPumpProbeNative.WM_SYSKEYUP;
    }
    private bool IsTargetForeground() {
        IntPtr target = new IntPtr(_targetHwnd), foreground = SchedulerPumpProbeNative.GetForegroundWindow();
        uint targetPid, foregroundPid;
        SchedulerPumpProbeNative.GetWindowThreadProcessId(target, out targetPid);
        SchedulerPumpProbeNative.GetWindowThreadProcessId(foreground, out foregroundPid);
        return SchedulerPumpProbeNative.IsWindow(target) && SchedulerPumpProbeNative.IsWindowVisible(target) &&
            SchedulerPumpProbeNative.IsWindowEnabled(target) && targetPid == (uint)_targetPid &&
            foreground.ToInt64() == _targetHwnd && foregroundPid == (uint)_targetPid;
    }
    private void StartSource() {
        if (_sourceStarted || !PumpProbeSourcePolicy.CanStart(_source, _foregroundArmed, _preflightOnly) || !IsTargetForeground()) return;
        _sourceStarted = true;
        if (_load) StartNonKeyPoster();
        _producer = new Thread(Produce) { IsBackground = true, Name = "pump prototype source" };
        _producer.Start();
    }
    private void RequestClose() {
        if (Interlocked.Exchange(ref _closeRequested, 1) != 0) return;
        if (_pump == "Native") {
            SchedulerPumpProbeNative.PostThreadMessage(_nativeThreadId, 0x0012, IntPtr.Zero, IntPtr.Zero);
        } else if (_form != null && !_form.IsDisposed && _form.IsHandleCreated) {
            try { _form.BeginInvoke((MethodInvoker)delegate { _form.Close(); }); }
            catch (InvalidOperationException) { _failureReason = "close_dispatch_failed"; }
        }
    }
    private void StartNonKeyPoster() {
        _nonKeyPoster = new Thread(delegate() {
            long end = Stopwatch.GetTimestamp() + (long)_durationSeconds * Stopwatch.Frequency;
            while (Stopwatch.GetTimestamp() < end && Volatile.Read(ref _stopSource) == 0) {
                SchedulerPumpProbeNative.PostMessage(new IntPtr(_targetHwnd), WM_NONKEY, IntPtr.Zero, IntPtr.Zero);
                Thread.Sleep(20);
            }
        }) { IsBackground = true, Name = "pump prototype non-key load" };
        _nonKeyPoster.Start();
    }
    private void Produce() {
        long deadline = Stopwatch.GetTimestamp() + (long)_durationSeconds * Stopwatch.Frequency;
        long period = Math.Max(1, Stopwatch.Frequency / 600);
        long next = Stopwatch.GetTimestamp();
        bool down = true;
        while (Stopwatch.GetTimestamp() < deadline && Volatile.Read(ref _stopSource) == 0) {
            long focusStart = Stopwatch.GetTimestamp();
            bool foreground = IsTargetForeground();
            long focusElapsed = Stopwatch.GetTimestamp() - focusStart;
            Interlocked.Increment(ref _focusCheckCount);
            Interlocked.Add(ref _focusCheckTotalUs, ToUs(focusElapsed));
            if (!foreground) {
                Interlocked.Increment(ref _wrongIdentity);
                _failureReason = "foreground_identity_lost_during_source";
                Interlocked.Exchange(ref _stopSource, 1);
                break;
            }
            long callStart = Stopwatch.GetTimestamp();
            int message = down ? SchedulerPumpProbeNative.WM_KEYDOWN : SchedulerPumpProbeNative.WM_KEYUP;
            long lParam = 1L | (0x1eL << 16);
            if (!down) lParam |= (1L << 30) | (1L << 31);
            Interlocked.Increment(ref _sourceRequested);
            Interlocked.Increment(ref _sourceAttempts);
            bool accepted = SchedulerPumpProbeNative.PostMessage(new IntPtr(_targetHwnd), (uint)message, new IntPtr(0x41), new IntPtr(lParam));
            if (accepted) { Interlocked.Increment(ref _sourceInserted); Interlocked.Increment(ref _postAccepted); }
            else { Interlocked.Increment(ref _postRejected); }
            long callEnd = Stopwatch.GetTimestamp();
            Interlocked.Increment(ref _sourceCompletions);
            AddLatency(_sourceCallUs, ref _sourceCallCount, ToUs(callEnd - callStart));
            if (!accepted) break;
            down = !down;
            next += period;
            while (Stopwatch.GetTimestamp() < next) Thread.Sleep(0);
        }
        _sourceCompletionQpc = Stopwatch.GetTimestamp();
        Interlocked.Exchange(ref _stopSource, 1);
        RequestClose();
    }
    private void AddLatency(long[] samples, ref int count, long value) {
        lock (_latencyGate) { if (count < samples.Length) samples[count++] = value; }
    }
    private long ToUs(long ticks) { return (long)(((decimal)ticks * 1000000m) / Stopwatch.Frequency); }
    public void Observe(uint message, IntPtr wParam, IntPtr lParam, long messageHwnd) {
        long start = Stopwatch.GetTimestamp();
        long retrievedQpc;
        PumpRetrievalStatus retrieval = _retrievalMatcher.TryMatch(messageHwnd, message,
            unchecked((ulong)wParam.ToInt64()), lParam.ToInt64(), out retrievedQpc);
        if (retrieval == PumpRetrievalStatus.Matched) Interlocked.Increment(ref _retrievalMatched);
        else if (retrieval == PumpRetrievalStatus.Unmatched) Interlocked.Increment(ref _retrievalUnmatched);
        else Interlocked.Increment(ref _retrievalUnknown);
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
            RetrievedQpc = retrieval == PumpRetrievalStatus.Matched ? retrievedQpc : -1,
            RetrievalStatus = retrieval.ToString().ToLowerInvariant(), HandlerStartQpc = start, ForegroundCheckUs = focusUs,
            Hwnd = messageHwnd, Pid = (int)_targetPid, ForegroundHwnd = foreground.ToInt64(), ForegroundPid = (int)fgPid,
            Message = (int)message, VirtualKey = vk, ScanCode = (int)((nativeLParam >> 16) & 0xff),
            Extended = ((nativeLParam >> 24) & 1) != 0, QueueStatus = queueStatus
        };
        if (retrieval == PumpRetrievalStatus.Matched && retrievedQpc > 0 && start >= retrievedQpc) {
            AddLatency(_dispatchLagUs, ref _dispatchCount, ToUs(start - retrievedQpc));
            Interlocked.Increment(ref _dispatchValidSamples);
        }
        AddLatency(_foregroundUs, ref _foregroundCount, focusUs);
        if (_pump == "WinForms" && _form != null) {
            if (((_ring.EventsSeen + 1) % 64) == 0) {
                long begin = Stopwatch.GetTimestamp();
                try {
                    _form.BeginInvoke((MethodInvoker)delegate {
                        Interlocked.Increment(ref _beginInvokeCompleted);
                        if (_label != null && !_label.IsDisposed) _label.Text = "Diagnostic only — scheduler-pump-prototype-v2";
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
    public void WindowMessage(uint message) {
        if (message == SchedulerPumpProbeNative.WM_PAINT) Interlocked.Increment(ref _paintMessages);
        if (message == SchedulerPumpProbeNative.WM_ACTIVATE) Interlocked.Increment(ref _activationMessages);
    }
    public void TimerCallback() {
        Interlocked.Increment(ref _timerCallbacks);
        long now = Stopwatch.GetTimestamp();
        IntPtr target = new IntPtr(_targetHwnd);
        bool targetValid = SchedulerPumpProbeNative.IsWindow(target);
        uint targetPid; SchedulerPumpProbeNative.GetWindowThreadProcessId(target, out targetPid);
        bool targetVisible = targetValid && SchedulerPumpProbeNative.IsWindowVisible(target);
        bool targetEnabled = targetValid && SchedulerPumpProbeNative.IsWindowEnabled(target);
        IntPtr foreground = SchedulerPumpProbeNative.GetForegroundWindow();
        uint foregroundPid; SchedulerPumpProbeNative.GetWindowThreadProcessId(foreground, out foregroundPid);
        _targetValid = targetValid;
        _targetVisible = targetVisible;
        _targetEnabled = targetEnabled;
        _lastForegroundHwnd = foreground.ToInt64();
        _lastForegroundPid = (int)foregroundPid;
        Interlocked.Increment(ref _foregroundChecks);
        bool foregroundHwndMatches = foreground.ToInt64() == _targetHwnd;
        bool foregroundPidMatches = foregroundPid == (uint)_targetPid;
        bool exactForeground = foregroundHwndMatches && foregroundPidMatches;
        bool targetReady = targetValid && _targetOwnedByProcess && targetPid == (uint)_targetPid && targetVisible && targetEnabled;
        if (_foregroundArm.State == PumpForegroundState.Waiting) {
            PumpForegroundState state = _foregroundArm.Observe(now, targetValid, _targetOwnedByProcess && targetPid == (uint)_targetPid,
                targetVisible, targetEnabled, foregroundHwndMatches, foregroundPidMatches);
            if (state == PumpForegroundState.Waiting) {
                if (!exactForeground) Interlocked.Increment(ref _foregroundMismatchChecks);
            } else if (state == PumpForegroundState.Armed) {
                _foregroundArmed = true;
                _failureReason = _foregroundArm.Reason;
                if (_preflightOnly) {
                    _preflightPassed = true;
                    _preflightCloseQpc = now + Stopwatch.Frequency / 2;
                } else if (PumpProbeSourcePolicy.CanStart(_source, true, false)) {
                    StartSource();
                    if (!_sourceStarted) {
                        _failureReason = "source_start_guard_failed";
                        RequestClose();
                        return;
                    }
                    if (_timer != null) _timer.Interval = _load ? 250 : 1000;
                    if (_nativeHwnd != IntPtr.Zero)
                        SchedulerPumpProbeNative.SetTimer(_nativeHwnd, new UIntPtr(1), _load ? 250U : 1000U, IntPtr.Zero);
                } else {
                    _failureReason = "source_mode_blocked_before_start";
                    RequestClose();
                }
            } else {
                _failureReason = _foregroundArm.Reason;
                RequestClose();
            }
        } else if (_foregroundArmed && (!targetReady || !exactForeground)) {
            Interlocked.Increment(ref _foregroundMismatchChecks);
            Interlocked.Increment(ref _wrongIdentity);
            _failureReason = "foreground_identity_lost_after_arm";
            Interlocked.Exchange(ref _stopSource, 1);
            RequestClose();
            return;
        }
        if (_preflightOnly && _preflightPassed && now >= _preflightCloseQpc) {
            RequestClose();
            return;
        }
        long start = Stopwatch.GetTimestamp();
        if (_load && _sourceStarted && targetReady && exactForeground)
            SchedulerPumpProbeContext.SuppressOverlayWindows((uint)_targetPid);
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
            "{{\"elapsed_seconds\":{0},\"source_requested\":{1},\"source_inserted\":{2},\"source_attempts\":{3},\"source_completions\":{4},\"wndproc_seen\":{5},\"fifo_enqueued\":{6},\"writer_written\":{7},\"writer_flushed\":{8},\"queue_status\":{9},\"queue_nonempty_sample\":{10},\"retrieval_matched\":{11},\"retrieval_unmatched\":{12},\"retrieval_unknown\":{13},\"dispatch_valid_samples\":{14}}}",
            _perSecond.Count + 1, Interlocked.Read(ref _sourceRequested), Interlocked.Read(ref _sourceInserted),
            Interlocked.Read(ref _sourceAttempts), Interlocked.Read(ref _sourceCompletions), seen,
            GetLong(ring, "enqueued"), written, flushed,
            (queueStatus >> 16), ((queueStatus >> 16) & 0xffffU) != 0 ? "true" : "false",
            Interlocked.Read(ref _retrievalMatched), Interlocked.Read(ref _retrievalUnmatched),
            Interlocked.Read(ref _retrievalUnknown), Interlocked.Read(ref _dispatchValidSamples)));
    }
    private void Consume() {
        int unflushed = 0;
        try {
            using (var writer = new StreamWriter(_eventPath, false, new UTF8Encoding(false), 65536)) {
                writer.WriteLine("{\"schema_version\":2,\"observer_version\":\"" + ObserverVersion + "\",\"run_id\":\"" + _runId + "\",\"qualification\":false,\"kind\":\"stream_start\",\"pump\":\"" + _pump + "\",\"source\":\"" + _source + "\",\"diagnostic_load\":" + (_load ? "true" : "false") + ",\"qpc_frequency_hz\":" + Stopwatch.Frequency.ToString(CultureInfo.InvariantCulture) + ",\"target_hwnd\":" + _targetHwnd.ToString(CultureInfo.InvariantCulture) + ",\"target_pid\":" + _targetPid.ToString(CultureInfo.InvariantCulture) + "}"); writer.Flush();
                SchedulerPumpEvent item;
                while (_ring.TryDequeue(out item)) {
                    string retrievalQpc = item.RetrievalStatus == "matched"
                        ? item.RetrievedQpc.ToString(CultureInfo.InvariantCulture) : "null";
                    writer.WriteLine(String.Format(CultureInfo.InvariantCulture,
                        "{{\"schema_version\":2,\"observer_version\":\"{0}\",\"sequence\":{1},\"kind\":\"{2}\",\"message\":{3},\"virtual_key\":{4},\"scan_code\":{5},\"extended\":{6},\"observed_qpc_ticks\":{7},\"retrieval_status\":\"{8}\",\"queue_retrieved_qpc_ticks\":{9},\"wndproc_start_qpc_ticks\":{10},\"wndproc_end_qpc_ticks\":{11},\"foreground_check_us\":{12},\"target_hwnd\":{13},\"target_pid\":{14},\"foreground_hwnd\":{15},\"foreground_pid\":{16},\"queue_status\":{17},\"run_id\":\"{18}\",\"qualification\":false}}",
                        ObserverVersion, item.Sequence, (item.Message == SchedulerPumpProbeNative.WM_KEYDOWN || item.Message == SchedulerPumpProbeNative.WM_SYSKEYDOWN) ? "key_down" : "key_up",
                        item.Message, item.VirtualKey, item.ScanCode, item.Extended ? "true" : "false", item.HandlerStartQpc,
                        item.RetrievalStatus, retrievalQpc, item.HandlerStartQpc, item.HandlerEndQpc, item.ForegroundCheckUs,
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
    private void StopSourceThreads() {
        Interlocked.Exchange(ref _stopSource, 1);
        if (_producer != null && _producer.IsAlive && !_producer.Join(5000))
            throw new TimeoutException("diagnostic source producer did not stop after the window pump closed");
        if (_nonKeyPoster != null && _nonKeyPoster.IsAlive && !_nonKeyPoster.Join(1000))
            throw new TimeoutException("diagnostic non-key producer did not stop after the window pump closed");
    }
    private static IntPtr NativeWindowProc(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam) {
        SchedulerPumpProbeContext context = _nativeContext;
        if (context == null) return SchedulerPumpProbeNative.DefWindowProc(hwnd, message, wParam, lParam);
        if (message == SchedulerPumpProbeNative.WM_TIMER) { context.TimerCallback(); return IntPtr.Zero; }
        if (message == WM_NONKEY) { context.NonKeyCallback(); return IntPtr.Zero; }
        context.WindowMessage(message);
        if (context.IsKeyMessage(message)) {
            context.Observe(message, wParam, lParam, hwnd.ToInt64());
            return IntPtr.Zero;
        }
        if (message == 0x0002) { SchedulerPumpProbeNative.PostQuitMessage(0); return IntPtr.Zero; }
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
        string foregroundState = _foregroundArm == null ? "not_started" : _foregroundArm.State.ToString().ToLowerInvariant();
        return String.Format(CultureInfo.InvariantCulture,
            "{{\"schema_version\":2,\"observer_version\":\"{0}\",\"qualification\":false,\"pump\":\"{1}\",\"source\":\"{2}\",\"diagnostic_load\":{3},\"duration_seconds\":{4},\"qpc_frequency_hz\":{5},\"target_hwnd\":{6},\"target_pid\":{7},\"source_requested\":{8},\"source_inserted\":{9},\"source_attempts\":{10},\"source_completions\":{11},\"post_accepted\":{12},\"post_rejected\":{13},\"source_completion_qpc_ticks\":{14},\"wndproc_seen\":{15},\"fifo_enqueued\":{16},\"writer_written\":{17},\"writer_flushed\":{18},\"queue_dropped\":{19},\"queue_high_water\":{20},\"queue_capacity\":{21},\"queue_overflow\":{22},\"writer_failures\":{23},\"wrong_identity\":{24},\"pump_nonkey_callbacks\":{25},\"timer_callbacks\":{26},\"overlay_total_us\":{27},\"overlay_max_us\":{28},\"begin_invoke_calls\":{29},\"begin_invoke_completed\":{30},\"begin_invoke_total_us\":{31},\"foreground_check_count\":{32},\"foreground_check_total_us\":{33},\"dispatch_lag\":{34},\"wndproc_handler\":{35},\"foreground_check\":{36},\"source_call\":{37},\"per_second\":[{38}],\"raw_event_log\":\"{39}\",\"run_id\":\"{40}\",\"source_started\":{41},\"foreground_armed\":{42},\"preflight_passed\":{43},\"foreground_state\":\"{44}\",\"foreground_reason\":\"{45}\",\"target_owned_by_process\":{46},\"target_valid\":{47},\"target_visible\":{48},\"target_enabled\":{49},\"last_foreground_hwnd\":{50},\"last_foreground_pid\":{51},\"foreground_checks\":{52},\"foreground_mismatch_checks\":{53},\"window_paint_messages\":{54},\"window_activation_messages\":{55},\"retrieval_matched\":{56},\"retrieval_unmatched\":{57},\"retrieval_unknown\":{58},\"dispatch_valid_sample_count\":{59}}}",
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
            perSecond.ToString(), _eventPath.Replace("\\", "\\\\").Replace("\"", "\\\""), _runId,
            _sourceStarted ? "true" : "false", _foregroundArmed ? "true" : "false", _preflightPassed ? "true" : "false",
            foregroundState, _failureReason, _targetOwnedByProcess ? "true" : "false", _targetValid ? "true" : "false",
            _targetVisible ? "true" : "false", _targetEnabled ? "true" : "false", _lastForegroundHwnd, _lastForegroundPid,
            Interlocked.Read(ref _foregroundChecks), Interlocked.Read(ref _foregroundMismatchChecks),
            Interlocked.Read(ref _paintMessages), Interlocked.Read(ref _activationMessages),
            Interlocked.Read(ref _retrievalMatched), Interlocked.Read(ref _retrievalUnmatched),
            Interlocked.Read(ref _retrievalUnknown), Interlocked.Read(ref _dispatchValidSamples));
    }
    private sealed class ProbeMessageFilter : IMessageFilter {
        private readonly SchedulerPumpProbeContext _context;
        public ProbeMessageFilter(SchedulerPumpProbeContext context) { _context = context; }
        public bool PreFilterMessage(ref Message message) {
            _context.CaptureFilteredMessage(ref message); return false;
        }
    }
    private static long GetLong(object value, string name) { return Convert.ToInt64(value.GetType().GetProperty(name).GetValue(value, null), CultureInfo.InvariantCulture); }
    private static int GetInt(object value, string name) { return Convert.ToInt32(value.GetType().GetProperty(name).GetValue(value, null), CultureInfo.InvariantCulture); }
    private static bool GetBool(object value, string name) { return Convert.ToBoolean(value.GetType().GetProperty(name).GetValue(value, null), CultureInfo.InvariantCulture); }
    public sealed class ProbeForm : Form {
        private readonly SchedulerPumpProbeContext _context;
        public ProbeForm(SchedulerPumpProbeContext context) { _context = context; }
        protected override void WndProc(ref Message message) {
            _context.WindowMessage((uint)message.Msg);
            if (_context.IsKeyMessage((uint)message.Msg)) {
                _context.Observe((uint)message.Msg, message.WParam, message.LParam, message.HWnd.ToInt64());
            } else if (message.Msg == WM_NONKEY) _context.NonKeyCallback();
            base.WndProc(ref message);
        }
    }
}

public static class SchedulerPumpProbe {
    public static string SelfTest() { return SchedulerPumpProbeContractTests.Run(); }
    public static string Run(string pump, string source, bool load, bool preflightOnly, int durationSeconds,
        int foregroundTimeoutSeconds, string eventPath, string runId) {
        if (source == "SendInput") throw new InvalidOperationException("SendInput diagnostic mode is blocked before startup");
        var context = new SchedulerPumpProbeContext(pump, source, load, preflightOnly, durationSeconds,
            foregroundTimeoutSeconds, eventPath, runId);
        if (pump == "Native") context.StartNative(); else context.StartWinForms();
        return context.SummaryJson();
    }
}

"@

if ($CompileOnly) {
    Write-Output 'scheduler-receiveonly-pump-prototype-v2 C# support: COMPILE PASS'
    exit 0
}
if ($SelfTest) {
    Write-Output ([SchedulerPumpProbe]::SelfTest())
    exit 0
}
if (-not [Environment]::UserInteractive) { throw 'Pump probe runtime requires an interactive Windows desktop' }
$runMode = if ($PreflightOnly) { 'preflight' } else { $Source.ToLowerInvariant() }
$runId = 'scheduler-pump-prototype-v2-' + $Pump.ToLowerInvariant() + '-' + $runMode + '-' + [guid]::NewGuid().ToString('N').Substring(0, 8)
$eventsPath = Join-Path $probeRoot ($runId + '.jsonl')
$summaryPath = Join-Path $probeRoot ($runId + '.summary.json')
Write-Host 'Foreground gate: click the visible project-owned probe or Alt+Tab to it. Its UI pump remains active while it polls; source stays disarmed until target and foreground HWND/PID match exactly.'
try {
    $summaryJson = [SchedulerPumpProbe]::Run($Pump, $Source, [bool]$DiagnosticLoad, [bool]$PreflightOnly,
        $DurationSeconds, $ForegroundTimeoutSeconds, $eventsPath, $runId)
} catch {
    $failureMessage = $_.Exception.Message
    $foregroundMatch = [regex]::Match($failureMessage, 'target HWND/PID=(\d+)/(\d+), foreground HWND/PID=(\d+)/(\d+)')
    $failureRecord = [ordered]@{
        schema_version = 2
        observer_version = 'scheduler-receiveonly-pump-prototype-v2'
        qualification = $false
        run_id = $runId
        pump = $Pump
        source = $Source
        diagnostic_load = [bool]$DiagnosticLoad
        duration_seconds = $DurationSeconds
        failure_phase = 'probe_startup_or_runtime'
        source_started = $false
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
        probe_script_sha256 = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    $failureRecord | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $summaryPath -Encoding utf8
    throw
}
$summary = $summaryJson | ConvertFrom-Json
$summary | Add-Member -NotePropertyName source_revision -NotePropertyValue ((& git -C $repositoryRoot rev-parse HEAD).Trim())
$summary | Add-Member -NotePropertyName source_tree_sha -NotePropertyValue ((& git -C $repositoryRoot show -s --format=%T HEAD).Trim())
$summary | Add-Member -NotePropertyName source_branch -NotePropertyValue ((& git -C $repositoryRoot branch --show-current).Trim())
$summary | Add-Member -NotePropertyName worktree_clean_at_run -NotePropertyValue ([string]::IsNullOrEmpty(((& git -C $repositoryRoot status --porcelain=v1) -join "`n")))
$summary | Add-Member -NotePropertyName probe_script_sha256 -NotePropertyValue ((Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant())
$contractPath = Join-Path $PSScriptRoot 'test_scheduler_receiveonly_pump_probe_contracts.ps1'
if (Test-Path -LiteralPath $contractPath) {
    $summary | Add-Member -NotePropertyName contract_script_sha256 -NotePropertyValue ((Get-FileHash -LiteralPath $contractPath -Algorithm SHA256).Hash.ToLowerInvariant())
}
$summaryJson = $summary | ConvertTo-Json -Depth 32 -Compress
Set-Content -LiteralPath $summaryPath -Value $summaryJson -Encoding utf8
$records = @(Get-Content -LiteralPath $eventsPath | ForEach-Object { $_ | ConvertFrom-Json })
if ($records.Count -lt 1 -or $records[0].kind -ne 'stream_start' -or
    $records[0].schema_version -ne 2 -or $records[0].observer_version -ne 'scheduler-receiveonly-pump-prototype-v2' -or
    $records[0].run_id -ne $runId -or [bool]$records[0].qualification) {
    throw 'Diagnostic event stream is missing distinct prototype provenance'
}
$previousQpc = -1L
for ($index = 1; $index -lt $records.Count; $index++) {
    $event = $records[$index]
    $expectedKind = if ([int]$event.message -in @(0x0100, 0x0104)) { 'key_down' } else { 'key_up' }
    if ($event.schema_version -ne 2 -or $event.observer_version -ne 'scheduler-receiveonly-pump-prototype-v2' -or
        $event.run_id -ne $runId -or $event.sequence -ne $index -or $event.kind -ne $expectedKind -or
        $event.virtual_key -le 0 -or $event.message -notin @(0x0100, 0x0101, 0x0104, 0x0105) -or
        $event.retrieval_status -notin @('matched', 'unmatched', 'unknown') -or
        (($event.retrieval_status -eq 'matched') -and ($null -eq $event.queue_retrieved_qpc_ticks -or $event.queue_retrieved_qpc_ticks -le 0)) -or
        (($event.retrieval_status -ne 'matched') -and $null -ne $event.queue_retrieved_qpc_ticks) -or
        $event.target_hwnd -ne $summary.target_hwnd -or $event.target_pid -ne $summary.target_pid -or
        $event.foreground_hwnd -ne $summary.target_hwnd -or $event.foreground_pid -ne $summary.target_pid -or
        $event.observed_qpc_ticks -lt $previousQpc) {
        throw ('Diagnostic event stream identity, order, or metadata failed at sequence ' + $index)
    }
    $previousQpc = [long]$event.observed_qpc_ticks
}
if (-not $summary.foreground_armed) {
    if ($summary.source_started -or $summary.source_requested -ne 0 -or $summary.source_inserted -ne 0 -or
        $summary.source_attempts -ne 0 -or $summary.wndproc_seen -ne 0 -or $records.Count -ne 1) {
        throw 'Foreground preflight failed after source activity; evidence is invalid and must be inspected'
    }
    Write-Output $summaryJson
    exit 3
}
if ($PreflightOnly -and ($summary.source_started -or $summary.source_requested -ne 0 -or $records.Count -ne 1)) {
    throw 'Preflight-only run started a source or recorded source events'
}
if ($PreflightOnly -and (-not $summary.preflight_passed -or $summary.timer_callbacks -lt 1 -or
    -not $summary.target_valid -or -not $summary.target_visible -or -not $summary.target_enabled -or
    $summary.last_foreground_hwnd -ne $summary.target_hwnd -or $summary.last_foreground_pid -ne $summary.target_pid)) {
    throw 'Preflight did not prove a live message pump and exact foreground target identity'
}
if (-not $PreflightOnly -and -not $summary.source_started) {
    throw 'Foreground armed but diagnostic source did not start'
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
