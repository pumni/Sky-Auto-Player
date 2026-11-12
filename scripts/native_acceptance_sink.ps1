param(
    [switch]$SelfTest,
    [ValidateSet("ReceiveOnly", "InertFocusProbe")]
    [string]$Mode,
    [ValidateNotNullOrEmpty()]
    [string]$RunId,
    [ValidateNotNullOrEmpty()]
    [string]$ReadyFile,
    [ValidateNotNullOrEmpty()]
    [string]$EventLog,
    [ValidateSet('LegacyInline', 'SchedulerQueuedV2')]
    [string]$ObserverMode = 'LegacyInline',
    [string]$ObserverSummaryFile,
    [string]$ActivateRequestFile,
    [double]$DurationSeconds = 0
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$nativeAcceptanceReferenceDirectory = Join-Path $PSHOME 'ref'
$nativeAcceptanceReferences = @(
    [System.Windows.Forms.Form].Assembly.Location,
    [System.Windows.Forms.Message].Assembly.Location,
    [System.Drawing.Font].Assembly.Location,
    [System.ComponentModel.Component].Assembly.Location,
    [System.Text.Json.JsonSerializer].Assembly.Location,
    (Join-Path $nativeAcceptanceReferenceDirectory 'System.Collections.dll'),
    (Join-Path $nativeAcceptanceReferenceDirectory 'System.Runtime.dll'),
    (Join-Path $nativeAcceptanceReferenceDirectory 'System.IO.dll'),
    (Join-Path $nativeAcceptanceReferenceDirectory 'System.Text.Encoding.dll'),
    (Join-Path $nativeAcceptanceReferenceDirectory 'System.Text.Encoding.Extensions.dll'),
    (Join-Path $nativeAcceptanceReferenceDirectory 'System.Threading.dll'),
    (Join-Path $nativeAcceptanceReferenceDirectory 'System.Threading.Thread.dll'),
    (Join-Path $nativeAcceptanceReferenceDirectory 'System.Diagnostics.TraceSource.dll')
) | Select-Object -Unique
Add-Type -ReferencedAssemblies $nativeAcceptanceReferences -TypeDefinition @"
using System;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Text;
using System.Text.Json;
using System.Threading;
using System.Windows.Forms;

public sealed class NativeKeyMessageData : EventArgs {
    public string Kind { get; private set; }
    public int ScanCode { get; private set; }
    public bool Extended { get; private set; }
    public int VirtualKey { get; private set; }
    public int Message { get; private set; }
    public byte Direction {
        get { return Kind == "key_press" || Kind == "sys_key_press" ? (byte)1 : (byte)2; }
    }

    private NativeKeyMessageData(string kind, int scanCode, bool extended, int virtualKey, int message) {
        Kind = kind;
        ScanCode = scanCode;
        Extended = extended;
        VirtualKey = virtualKey;
        Message = message;
    }

    public static NativeKeyMessageData DecodeKeyMessage(int message, IntPtr wParam, IntPtr lParam) {
        string kind;
        switch (message) {
            case 0x0100: kind = "key_press"; break;
            case 0x0101: kind = "key_release"; break;
            case 0x0104: kind = "sys_key_press"; break;
            case 0x0105: kind = "sys_key_release"; break;
            default: return null;
        }

        long nativeLParam = lParam.ToInt64();
        int scanCode = (int)((nativeLParam >> 16) & 0xffL);
        bool extended = ((nativeLParam >> 24) & 0x1L) != 0;
        int virtualKey = (int)(wParam.ToInt64() & 0xffffL);
        return new NativeKeyMessageData(kind, scanCode, extended, virtualKey, message);
    }
}

public sealed class NativeAcceptanceSinkForm : Form {
    public event EventHandler<NativeKeyMessageData> NativeKeyMessage;
    public bool PreventActivationOnShow { get; set; }
    public NativeAcceptanceEventQueue SchedulerObserver { get; set; }

    protected override bool ShowWithoutActivation {
        get { return PreventActivationOnShow; }
    }

    protected override void WndProc(ref Message message) {
        NativeAcceptanceEventQueue observer = SchedulerObserver;
        long handlerStart = observer == null ? 0L : Stopwatch.GetTimestamp();
        bool observed = false;
        try {
            if (observer != null) {
                observed = observer.ObserveKeyMessage(message.Msg, message.WParam.ToInt64(),
                    message.LParam.ToInt64(), message.HWnd.ToInt64(), handlerStart);
            } else {
                NativeKeyMessageData decoded = NativeKeyMessageData.DecodeKeyMessage(
                    message.Msg, message.WParam, message.LParam);
                if (decoded != null && NativeKeyMessage != null) {
                    NativeKeyMessage(this, decoded);
                }
            }
            base.WndProc(ref message);
        } finally {
            if (observed) {
                observer.RecordHandlerLatency(handlerStart, Stopwatch.GetTimestamp());
            }
        }
    }
}

public struct NativeAcceptanceQueuedKey {
    public long Sequence;
    public long QpcTicks;
    public long TargetHwnd;
    public int TargetPid;
    public long ForegroundHwnd;
    public int ForegroundPid;
    public int ScanCode;
    public bool Extended;
    public int VirtualKey;
    public int Message;
    public byte Direction;
}

public sealed class NativeAcceptanceRingSnapshot {
    public long EventsSeen;
    public long EventsEnqueued;
    public long EventsWritten;
    public long Dropped;
    public long QueueHighWater;
    public int QueueCapacity;
    public bool Overflowed;
    public long HandlerLatencyCount;
    public long HandlerLatencySampleCount;
    public long HandlerLatencySampleDropped;
    public long HandlerLatencyP50Us;
    public long HandlerLatencyP95Us;
    public long HandlerLatencyP99Us;
    public long HandlerLatencyMaxUs;
    public long WriterFailures;
    public long InternalFailures;
    public int QueueDepth;
    public bool AddingCompleted;
}

public sealed class NativeAcceptanceEventRing {
    private readonly object _gate = new object();
    private readonly NativeAcceptanceQueuedKey[] _items;
    private readonly long[] _handlerLatencyUs = new long[65536];
    private readonly AutoResetEvent _changed = new AutoResetEvent(false);
    private int _head;
    private int _tail;
    private int _count;
    private bool _completed;
    private bool _overflowed;
    private long _eventsSeen;
    private long _eventsEnqueued;
    private long _eventsWritten;
    private long _dropped;
    private long _queueHighWater;
    private long _handlerLatencyCount;
    private int _handlerLatencyStored;
    private long _handlerLatencySampleDropped;
    private long _handlerLatencyMaxUs;
    private long _writerFailures;
    private long _internalFailures;

    public NativeAcceptanceEventRing(int capacity) {
        if (capacity < 1 || capacity > 1048576) {
            throw new ArgumentOutOfRangeException("capacity");
        }
        _items = new NativeAcceptanceQueuedKey[capacity];
    }

    public bool TryEnqueue(byte direction, int scanCode, bool extended, int virtualKey,
        int message, long qpcTicks, long targetHwnd, int targetPid,
        long foregroundHwnd, int foregroundPid) {
        lock (_gate) {
            if (_completed) {
                _internalFailures++;
                return false;
            }
            if (_eventsSeen == Int64.MaxValue) {
                _overflowed = true;
                _dropped++;
                _changed.Set();
                return false;
            }
            long sequence = ++_eventsSeen;
            if (_overflowed || _count == _items.Length) {
                _overflowed = true;
                _dropped++;
                _changed.Set();
                return false;
            }
            _items[_tail] = new NativeAcceptanceQueuedKey {
                Sequence = sequence,
                QpcTicks = qpcTicks,
                TargetHwnd = targetHwnd,
                TargetPid = targetPid,
                ForegroundHwnd = foregroundHwnd,
                ForegroundPid = foregroundPid,
                ScanCode = scanCode,
                Extended = extended,
                VirtualKey = virtualKey,
                Message = message,
                Direction = direction
            };
            _tail = (_tail + 1) % _items.Length;
            _count++;
            _eventsEnqueued++;
            if (_count > _queueHighWater) _queueHighWater = _count;
        }
        _changed.Set();
        return true;
    }

    public bool TryDequeue(out NativeAcceptanceQueuedKey item, int waitMilliseconds) {
        while (true) {
            lock (_gate) {
                if (_count > 0) {
                    item = _items[_head];
                    _head = (_head + 1) % _items.Length;
                    _count--;
                    return true;
                }
                if (_completed) {
                    item = default(NativeAcceptanceQueuedKey);
                    return false;
                }
            }
            if (waitMilliseconds <= 0 || !_changed.WaitOne(waitMilliseconds)) {
                item = default(NativeAcceptanceQueuedKey);
                return false;
            }
        }
    }

    public void CompleteAdding() {
        lock (_gate) _completed = true;
        _changed.Set();
    }

    public bool IsCompletedAndEmpty {
        get { lock (_gate) return _completed && _count == 0; }
    }

    public void MarkWritten() {
        lock (_gate) _eventsWritten++;
    }

    public void MarkWriterFailure() {
        lock (_gate) _writerFailures++;
    }

    public void MarkInternalFailure() {
        lock (_gate) _internalFailures++;
    }

    public void RecordHandlerLatency(long startQpc, long endQpc, long qpcFrequency) {
        if (endQpc < startQpc || qpcFrequency <= 0) {
            MarkInternalFailure();
            return;
        }
        long elapsedUs = (long)(((decimal)(endQpc - startQpc) * 1000000m) / qpcFrequency);
        lock (_gate) {
            _handlerLatencyCount++;
            if (elapsedUs > _handlerLatencyMaxUs) _handlerLatencyMaxUs = elapsedUs;
            if (_handlerLatencyStored == _handlerLatencyUs.Length) {
                _handlerLatencySampleDropped++;
            } else {
                _handlerLatencyUs[_handlerLatencyStored++] = elapsedUs;
            }
        }
    }

    public NativeAcceptanceRingSnapshot Snapshot() {
        lock (_gate) {
            Array.Sort(_handlerLatencyUs, 0, _handlerLatencyStored);
            return new NativeAcceptanceRingSnapshot {
                EventsSeen = _eventsSeen,
                EventsEnqueued = _eventsEnqueued,
                EventsWritten = _eventsWritten,
                Dropped = _dropped,
                QueueHighWater = _queueHighWater,
                QueueCapacity = _items.Length,
                Overflowed = _overflowed,
                HandlerLatencyCount = _handlerLatencyCount,
                HandlerLatencySampleCount = _handlerLatencyStored,
                HandlerLatencySampleDropped = _handlerLatencySampleDropped,
                HandlerLatencyP50Us = Percentile(0.50),
                HandlerLatencyP95Us = Percentile(0.95),
                HandlerLatencyP99Us = Percentile(0.99),
                HandlerLatencyMaxUs = _handlerLatencyMaxUs,
                WriterFailures = _writerFailures,
                InternalFailures = _internalFailures,
                QueueDepth = _count,
                AddingCompleted = _completed
            };
        }
    }

    private long Percentile(double percentile) {
        if (_handlerLatencyStored == 0) return -1;
        int index = (int)Math.Ceiling(percentile * _handlerLatencyStored) - 1;
        return _handlerLatencyUs[Math.Max(0, index)];
    }

    public static bool RunContractSelfTest() {
        var ordered = new NativeAcceptanceEventRing(4);
        int[] messages = new int[] { 0x0100, 0x0101, 0x0104, 0x0105 };
        int[] virtualKeys = new int[] { 0x41, 0x41, 0x12, 0x12 };
        byte[] directions = new byte[] { 1, 2, 1, 2 };
        for (int index = 0; index < messages.Length; index++) {
            NativeKeyMessageData decoded = NativeKeyMessageData.DecodeKeyMessage(
                messages[index], new IntPtr(virtualKeys[index]), new IntPtr(0x00150000));
            if (decoded == null || decoded.VirtualKey != virtualKeys[index] ||
                decoded.Message != messages[index] ||
                decoded.Direction != directions[index] ||
                !ordered.TryEnqueue(decoded.Direction, decoded.ScanCode, decoded.Extended,
                    decoded.VirtualKey, decoded.Message, 100 + index, 11, 7, 11, 7)) return false;
        }
        NativeAcceptanceQueuedKey second;
        long previousQpc = Int64.MinValue;
        for (int index = 0; index < messages.Length; index++) {
            NativeAcceptanceQueuedKey item;
            if (!ordered.TryDequeue(out item, 0) || item.Sequence != index + 1 ||
                item.Direction != directions[index] || item.ScanCode != 0x15 ||
                item.VirtualKey != virtualKeys[index] || item.Message != messages[index] ||
                item.QpcTicks < previousQpc) return false;
            previousQpc = item.QpcTicks;
        }
        if (ordered.TryDequeue(out second, 0)) return false;

        var bounded = new NativeAcceptanceEventRing(1);
        if (!bounded.TryEnqueue(1, 0x1D, true, 0xA3, 0x0100, 200, 22, 8, 22, 8) ||
            bounded.TryEnqueue(2, 0x1D, true, 0xA3, 0x0101, 201, 22, 8, 22, 8) ||
            bounded.TryEnqueue(2, 0x1D, true, 0xA3, 0x0101, 202, 22, 8, 22, 8)) return false;
        NativeAcceptanceRingSnapshot snapshot = bounded.Snapshot();
        NativeAcceptanceQueuedKey retained;
        return snapshot.Overflowed && snapshot.QueueCapacity == 1 &&
            snapshot.QueueHighWater == 1 && snapshot.EventsSeen == 3 &&
            snapshot.EventsEnqueued == 1 && snapshot.Dropped == 2 &&
            bounded.TryDequeue(out retained, 0) && retained.Sequence == 1 &&
            retained.Direction == 1 && retained.ScanCode == 0x1D && retained.Extended &&
            retained.VirtualKey == 0xA3 && retained.Message == 0x0100 &&
            !bounded.TryDequeue(out retained, 0);
    }
}

public sealed class NativeAcceptanceEventQueue {
    public const string ObserverVersion = "scheduler-receiveonly-queue-v2";
    public static long QpcFrequency { get { return Stopwatch.Frequency; } }
    private readonly NativeAcceptanceEventRing _ring;
    private readonly string _eventLog;
    private readonly string _summaryFile;
    private readonly string _runId;
    private readonly string _role;
    private readonly string _eventLogId;
    private readonly long _targetHwnd;
    private readonly int _targetPid;
    private readonly Label _label;
    private readonly string _displayTemplate;
    private readonly long _qpcOrigin;
    private readonly long _utcOriginTicks;
    private readonly AutoResetEvent _ready = new AutoResetEvent(false);
    private readonly Thread _consumer;
    private volatile bool _headerFlushed;
    private volatile bool _summaryWritten;
    private volatile bool _consumerStopped;
    private volatile int _startupFailures;
    private int _completionStarted;
    private bool _writerFailed;
    private volatile bool _eventsFlushed;

    [System.Runtime.InteropServices.DllImport("user32.dll")]
    private static extern IntPtr GetForegroundWindow();

    [System.Runtime.InteropServices.DllImport("user32.dll")]
    private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);

    public NativeAcceptanceEventQueue(string eventLog, string summaryFile, string runId,
        string role, string eventLogId, long targetHwnd, int targetPid, int capacity,
        Label label, string displayTemplate) {
        if (String.IsNullOrWhiteSpace(eventLog) || String.IsNullOrWhiteSpace(summaryFile) ||
            String.IsNullOrWhiteSpace(runId) || String.IsNullOrWhiteSpace(role) ||
            String.IsNullOrWhiteSpace(eventLogId) || targetHwnd <= 0 || targetPid <= 0) {
            throw new ArgumentException("Scheduler observer identity and output paths are required");
        }
        _eventLog = eventLog;
        _summaryFile = summaryFile;
        _runId = runId;
        _role = role;
        _eventLogId = eventLogId;
        _targetHwnd = targetHwnd;
        _targetPid = targetPid;
        _ring = new NativeAcceptanceEventRing(capacity);
        _label = label;
        _displayTemplate = displayTemplate;
        _qpcOrigin = Stopwatch.GetTimestamp();
        _utcOriginTicks = DateTime.UtcNow.Ticks;
        _consumer = new Thread(Consume) { IsBackground = true, Name = "ReceiveOnly observer consumer" };
        _consumer.Start();
    }

    public bool WaitForReady(int timeoutMilliseconds) {
        return _ready.WaitOne(timeoutMilliseconds) && _headerFlushed && _startupFailures == 0;
    }

    public bool ObserveKeyMessage(int message, long wParam, long lParam,
        long messageHwnd, long qpcTicks) {
        NativeKeyMessageData decoded = NativeKeyMessageData.DecodeKeyMessage(
            message, new IntPtr(wParam), new IntPtr(lParam));
        if (decoded == null) return false;
        byte direction = decoded.Direction;
        int targetPid = 0;
        int foregroundPid = 0;
        long foregroundHwnd = 0;
        try {
            uint rawTargetPid;
            GetWindowThreadProcessId(new IntPtr(messageHwnd), out rawTargetPid);
            targetPid = (int)rawTargetPid;
            IntPtr foreground = GetForegroundWindow();
            uint rawForegroundPid;
            GetWindowThreadProcessId(foreground, out rawForegroundPid);
            foregroundHwnd = foreground.ToInt64();
            foregroundPid = (int)rawForegroundPid;
            if (targetPid <= 0 || foregroundHwnd <= 0 || foregroundPid <= 0) {
                _ring.MarkInternalFailure();
            }
        } catch {
            _ring.MarkInternalFailure();
        }
        try {
            _ring.TryEnqueue(direction, decoded.ScanCode, decoded.Extended,
                decoded.VirtualKey, decoded.Message, qpcTicks, messageHwnd,
                targetPid, foregroundHwnd, foregroundPid);
        } catch {
            _ring.MarkInternalFailure();
        }
        return true;
    }

    public void RecordHandlerLatency(long startQpc, long endQpc) {
        try { _ring.RecordHandlerLatency(startQpc, endQpc, Stopwatch.Frequency); }
        catch { _ring.MarkInternalFailure(); }
    }

    public bool Complete() {
        if (Interlocked.Exchange(ref _completionStarted, 1) == 0) _ring.CompleteAdding();
        bool joined = _consumer.Join(30000);
        return joined && _consumerStopped && _summaryWritten && IsClean();
    }

    public bool IsClean() {
        NativeAcceptanceRingSnapshot snapshot = _ring.Snapshot();
        return !_writerFailed && _startupFailures == 0 && snapshot.Dropped == 0 &&
            !snapshot.Overflowed && snapshot.WriterFailures == 0 && snapshot.InternalFailures == 0;
    }

    private void Consume() {
        long downCount = 0;
        long upCount = 0;
        int unflushed = 0;
        try {
            using (var writer = new StreamWriter(_eventLog, true, new UTF8Encoding(false), 65536)) {
                writer.WriteLine(HeaderJson());
                writer.Flush();
                _headerFlushed = true;
                _ready.Set();
                while (true) {
                    NativeAcceptanceQueuedKey item;
                    if (!_ring.TryDequeue(out item, 100)) {
                        if (_ring.IsCompletedAndEmpty) break;
                        if (unflushed > 0) {
                            writer.Flush();
                            unflushed = 0;
                            UpdateUi(downCount, upCount);
                        }
                        continue;
                    }
                    if (!_writerFailed) {
                        try {
                            writer.WriteLine(EventJson(item));
                            _ring.MarkWritten();
                            unflushed++;
                            if (unflushed >= 64) {
                                writer.Flush();
                                unflushed = 0;
                                UpdateUi(downCount, upCount);
                            }
                        } catch {
                            _writerFailed = true;
                            _ring.MarkWriterFailure();
                        }
                    } else {
                        _ring.MarkWriterFailure();
                    }
                    if (item.Direction == 1) downCount++; else upCount++;
                }
                writer.Flush();
                _eventsFlushed = true;
                UpdateUi(downCount, upCount);
            }
        } catch {
            _startupFailures++;
            _ring.MarkWriterFailure();
            _ready.Set();
        }
        _consumerStopped = true;
        try {
            NativeAcceptanceRingSnapshot snapshot = _ring.Snapshot();
            var summary = new {
                schema_version = 1,
                observer_version = ObserverVersion,
                run_id = _runId,
                role = _role,
                event_log_id = _eventLogId,
                target_hwnd = _targetHwnd,
                target_pid = _targetPid,
                qpc_frequency_hz = Stopwatch.Frequency,
                queue_capacity = snapshot.QueueCapacity,
                events_seen = snapshot.EventsSeen,
                events_enqueued = snapshot.EventsEnqueued,
                events_written = snapshot.EventsWritten,
                queue_dropped = snapshot.Dropped,
                queue_high_water = snapshot.QueueHighWater,
                queue_overflow = snapshot.Overflowed,
                queue_drained = snapshot.QueueDepth == 0 && snapshot.AddingCompleted,
                handler_latency_count = snapshot.HandlerLatencyCount,
                handler_latency_sample_count = snapshot.HandlerLatencySampleCount,
                handler_latency_sample_dropped = snapshot.HandlerLatencySampleDropped,
                handler_latency_p50_us = snapshot.HandlerLatencyP50Us < 0 ? (long?)null : snapshot.HandlerLatencyP50Us,
                handler_latency_p95_us = snapshot.HandlerLatencyP95Us < 0 ? (long?)null : snapshot.HandlerLatencyP95Us,
                handler_latency_p99_us = snapshot.HandlerLatencyP99Us < 0 ? (long?)null : snapshot.HandlerLatencyP99Us,
                handler_latency_max_us = snapshot.HandlerLatencyCount == 0 ? (long?)null : snapshot.HandlerLatencyMaxUs,
                writer_failures = snapshot.WriterFailures,
                observer_internal_failures = snapshot.InternalFailures,
                stream_header_flushed = _headerFlushed,
                events_flushed = _eventsFlushed,
                observer_startup_failures = _startupFailures,
                consumer_stopped = true
            };
            File.WriteAllText(_summaryFile,
                JsonSerializer.Serialize(summary) + Environment.NewLine, new UTF8Encoding(false));
            _summaryWritten = true;
        } catch {
            _startupFailures++;
        }
    }

    private string HeaderJson() {
        var header = new {
            schema_version = 4,
            run_id = _runId,
            role = _role,
            event_log_id = _eventLogId,
            observer_version = ObserverVersion,
            sequence = 0L,
            kind = "stream_start",
            scan_code = 0,
            extended = false,
            virtual_key = 0,
            message = 0,
            observed_utc = DateTime.UtcNow.ToString("O", CultureInfo.InvariantCulture),
            observed_qpc_ticks = _qpcOrigin,
            target_hwnd = _targetHwnd,
            target_pid = _targetPid,
            foreground_hwnd = _targetHwnd,
            foreground_pid = _targetPid
        };
        return JsonSerializer.Serialize(header);
    }

    private string EventJson(NativeAcceptanceQueuedKey item) {
        long utcTicks = _utcOriginTicks + (long)(((decimal)(item.QpcTicks - _qpcOrigin) * TimeSpan.TicksPerSecond) / Stopwatch.Frequency);
        var record = new {
            schema_version = 4,
            run_id = _runId,
            role = _role,
            event_log_id = _eventLogId,
            observer_version = ObserverVersion,
            sequence = item.Sequence,
            kind = item.Direction == 1 ? "key_press" : "key_release",
            scan_code = item.ScanCode,
            extended = item.Extended,
            virtual_key = item.VirtualKey,
            message = item.Message,
            observed_utc = new DateTime(utcTicks, DateTimeKind.Utc).ToString("O", CultureInfo.InvariantCulture),
            observed_qpc_ticks = item.QpcTicks,
            target_hwnd = item.TargetHwnd,
            target_pid = item.TargetPid,
            foreground_hwnd = item.ForegroundHwnd,
            foreground_pid = item.ForegroundPid
        };
        return JsonSerializer.Serialize(record);
    }

    private void UpdateUi(long downCount, long upCount) {
        if (_label == null || _label.IsDisposed || !_label.IsHandleCreated) return;
        try {
            _label.BeginInvoke((MethodInvoker)delegate {
                if (!_label.IsDisposed) {
                    _label.Text = String.Format(CultureInfo.InvariantCulture,
                        _displayTemplate, downCount, upCount);
                }
            });
        } catch { }
    }
}
"@

if ($SelfTest) {
    function Assert-NativeDecoder {
        param([bool]$Condition, [string]$Name)
        if (-not $Condition) {
            throw "native decoder self-test failed: $Name"
        }
    }

    $down = [NativeKeyMessageData]::DecodeKeyMessage(
        0x0100,
        [IntPtr]::new(0xE5),
        [IntPtr]::new(0x00150000))
    Assert-NativeDecoder ($null -ne $down) "WM_KEYDOWN decoded"
    Assert-NativeDecoder ($down.Kind -eq "key_press") "KeyDown direction"
    Assert-NativeDecoder ($down.ScanCode -eq 0x15) "non-extended scan 0x15"
    Assert-NativeDecoder (-not $down.Extended) "non-extended flag"
    Assert-NativeDecoder ($down.VirtualKey -eq 0xE5) "diagnostic VK_PROCESSKEY"
    Assert-NativeDecoder ($down.Message -eq 0x0100) "KeyDown message"

    $up = [NativeKeyMessageData]::DecodeKeyMessage(
        0x0101,
        [IntPtr]::new(0xE5),
        [IntPtr]::new(0x00150000))
    Assert-NativeDecoder ($null -ne $up) "WM_KEYUP decoded"
    Assert-NativeDecoder ($up.Kind -eq "key_release") "KeyUp direction"
    Assert-NativeDecoder ($up.ScanCode -eq 0x15) "KeyUp scan 0x15"

    $extended = [NativeKeyMessageData]::DecodeKeyMessage(
        0x0100,
        [IntPtr]::new(0xA3),
        [IntPtr]::new(0x011D0000))
    Assert-NativeDecoder ($null -ne $extended) "extended message decoded"
    Assert-NativeDecoder ($extended.ScanCode -eq 0x1D) "extended scan"
    Assert-NativeDecoder $extended.Extended "extended flag"
    Assert-NativeDecoder ([NativeAcceptanceEventRing]::RunContractSelfTest()) "bounded queue order, sequence, and overflow"

    Write-Output "native acceptance sink decoder and Scheduler observer self-test: PASS"
    exit 0
}

if ([string]::IsNullOrWhiteSpace($Mode) -or
    [string]::IsNullOrWhiteSpace($RunId) -or
    [string]::IsNullOrWhiteSpace($ReadyFile) -or
    [string]::IsNullOrWhiteSpace($EventLog)) {
    throw "Mode, RunId, ReadyFile, and EventLog are required unless -SelfTest is used"
}
if ($RunId.Length -gt 128) {
    throw "RunId must contain 1..128 non-whitespace characters"
}
if ($ObserverMode -eq 'SchedulerQueuedV2' -and
    ($Mode -ne 'ReceiveOnly' -or [string]::IsNullOrWhiteSpace($ObserverSummaryFile))) {
    throw 'SchedulerQueuedV2 is opt-in for ReceiveOnly and requires ObserverSummaryFile'
}
if ($ObserverMode -eq 'LegacyInline' -and -not [string]::IsNullOrWhiteSpace($ObserverSummaryFile)) {
    throw 'ObserverSummaryFile is only valid for SchedulerQueuedV2'
}

Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class NativeAcceptanceSinkFocus {
    [DllImport("user32.dll")]
    public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")]
    public static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);
    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint lpdwProcessId);
    [DllImport("user32.dll")]
    public static extern int GetClassName(IntPtr hWnd, StringBuilder lpClassName, int nMaxCount);
    [DllImport("user32.dll")]
    public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);

    public static void SuppressOverlayWindows(uint pid) {
        EnumWindows((hWnd, lParam) => {
            uint procId;
            GetWindowThreadProcessId(hWnd, out procId);
            if (procId == pid) {
                StringBuilder cls = new StringBuilder(256);
                GetClassName(hWnd, cls, 256);
                string name = cls.ToString();
                if (name.StartsWith("UAC") || name.StartsWith("Touch Tooltip")) {
                    ShowWindow(hWnd, 0);
                }
            }
            return true;
        }, IntPtr.Zero);
    }
}
"@

$isProbe = $Mode -eq "InertFocusProbe"
$title = if ($isProbe) {
    "Sky Auto Player — Native Acceptance Focus Probe"
} else {
    "Sky Auto Player — Native Acceptance Sink"
}
$role = if ($isProbe) { "InertFocusProbe" } else { "ReceiveOnly" }
$sinkKind = if ($isProbe) {
    "SkyAutoPlayer.NativeAcceptanceFocusProbe"
} else {
    "SkyAutoPlayer.NativeAcceptanceSink"
}
$inputPolicy = "receives_only; benchmark must use SendInput"

$form = New-Object NativeAcceptanceSinkForm
$form.PreventActivationOnShow = $isProbe
$form.Text = $title
$form.Width = 620
$form.Height = 240
$form.StartPosition = [System.Windows.Forms.FormStartPosition]::CenterScreen

$label = New-Object System.Windows.Forms.Label
$label.Dock = [System.Windows.Forms.DockStyle]::Fill
$label.TextAlign = [System.Drawing.ContentAlignment]::MiddleCenter
$label.Font = New-Object System.Drawing.Font("Segoe UI", 12)
$label.Text = if ($isProbe) {
    "$title`r`n`r`nThis project-owned window observes accidental input only.`r`nIt never emits gameplay input.`r`n`r`nObserved key presses: 0 | releases: 0"
} else {
    "$title`r`n`r`nThis project-owned window is the only permitted real-input sink.`r`nKeep it as the intended target during SendInput runs.`r`n`r`nObserved key presses: 0 | releases: 0"
}
$form.Controls.Add($label)

$readyParent = Split-Path -Parent $ReadyFile
if ($readyParent) {
    New-Item -ItemType Directory -Force -Path $readyParent | Out-Null
}
$eventParent = Split-Path -Parent $EventLog
if ($eventParent) {
    New-Item -ItemType Directory -Force -Path $eventParent | Out-Null
}
$observerParent = if ($ObserverSummaryFile) { Split-Path -Parent $ObserverSummaryFile } else { $null }
if ($observerParent) {
    New-Item -ItemType Directory -Force -Path $observerParent | Out-Null
}

$eventWriter = $null
$schedulerObserver = $null
$state = @{
    key_press = 0
    key_release = 0
    sequence = 0L
    event_log_id = [guid]::NewGuid().ToString("N")
}

$observerVersion = $null
if ($ObserverMode -eq 'SchedulerQueuedV2') {
    $observerVersion = [NativeAcceptanceEventQueue]::ObserverVersion
} else {
    $eventWriter = [System.IO.StreamWriter]::new(
        $EventLog,
        $true,
        [System.Text.UTF8Encoding]::new($false)
    )
    $header = [ordered]@{
        schema_version = 3
        run_id = $RunId
        role = $role
        event_log_id = $state.event_log_id
        sequence = 0L
        kind = "stream_start"
        scan_code = 0
        extended = $false
        virtual_key = 0
        message = 0
        observed_utc = [DateTime]::UtcNow.ToString("O")
    } | ConvertTo-Json -Compress
    $eventWriter.WriteLine($header)
    $eventWriter.Flush()
}

$record = {
    param([object]$Sender, [NativeKeyMessageData]$Event)
    if ($Event.Kind -eq "key_press") {
        $state.key_press++
    } elseif ($Event.Kind -eq "key_release") {
        $state.key_release++
    }
    $state.sequence = [long]$state.sequence + 1L
    $label.Text = if ($isProbe) {
        "$title`r`n`r`nThis project-owned window observes accidental input only.`r`nIt never emits gameplay input.`r`n`r`nObserved key presses: $($state.key_press) | releases: $($state.key_release)"
    } else {
        "$title`r`n`r`nThis project-owned window is the only permitted real-input sink.`r`nKeep it as the intended target during SendInput runs.`r`n`r`nObserved key presses: $($state.key_press) | releases: $($state.key_release)"
    }
    $payload = [ordered]@{
        schema_version = 3
        run_id = $RunId
        role = $role
        event_log_id = $state.event_log_id
        sequence = [long]$state.sequence
        kind = $Event.Kind
        scan_code = [int]$Event.ScanCode
        extended = [bool]$Event.Extended
        virtual_key = [int]$Event.VirtualKey
        message = [int]$Event.Message
        observed_utc = [DateTime]::UtcNow.ToString("O")
    } | ConvertTo-Json -Compress
    $eventWriter.WriteLine($payload)
    $eventWriter.Flush()
}

# Both roles observe native keyboard messages. InertFocusProbe remains on the
# legacy observer. Scheduler ReceiveOnly uses only the bounded WndProc queue.
if ($ObserverMode -eq 'LegacyInline') {
    $form.Add_NativeKeyMessage({ param($Sender, $Event) & $record $Sender $Event })
}
$form.Add_Shown({
    if (-not $isProbe) {
        $form.Activate()
        $form.Focus()
        [void][NativeAcceptanceSinkFocus]::SetForegroundWindow($form.Handle)
    }
    [NativeAcceptanceSinkFocus]::SuppressOverlayWindows([uint32]$PID)
})
$form.Add_FormClosed({
    if ($null -ne $schedulerObserver) {
        [void]$schedulerObserver.Complete()
    } elseif ($null -ne $eventWriter) {
        $eventWriter.Dispose()
    }
})

$form.CreateControl()
$hwnd = $form.Handle.ToInt64()
$displayTemplate = if ($isProbe) {
    "$title`r`n`r`nThis project-owned window observes accidental input only.`r`nIt never emits gameplay input.`r`n`r`nObserved key presses: {0} | releases: {1}"
} else {
    "$title`r`n`r`nThis project-owned window is the only permitted real-input sink.`r`nKeep it as the intended target during SendInput runs.`r`n`r`nObserved key presses: {0} | releases: {1}"
}
if ($ObserverMode -eq 'SchedulerQueuedV2') {
    $schedulerObserver = [NativeAcceptanceEventQueue]::new(
        $EventLog,
        [System.IO.Path]::GetFullPath($ObserverSummaryFile),
        $RunId,
        $role,
        $state.event_log_id,
        [long]$hwnd,
        [int]$PID,
        32768,
        $label,
        $displayTemplate
    )
    $form.SchedulerObserver = $schedulerObserver
    if (-not $schedulerObserver.WaitForReady(5000)) {
        [void]$schedulerObserver.Complete()
        throw 'Scheduler observer failed to flush its stream header before ready'
    }
}
$processStartTimeFiletime = [long](Get-Process -Id $PID).StartTime.ToUniversalTime().ToFileTimeUtc()
$ready = [ordered]@{
    schema_version = 3
    event_schema_version = if ($ObserverMode -eq 'SchedulerQueuedV2') { 4 } else { 3 }
    run_id = $RunId
    role = $role
    sink_kind = $sinkKind
    event_log_id = $state.event_log_id
    pid = [int]$PID
    hwnd = [long]$hwnd
    title = $title
    process = "native_acceptance_sink.ps1"
    input_policy = $inputPolicy
    process_start_time_filetime = $processStartTimeFiletime
} | ConvertTo-Json
if ($ObserverMode -eq 'SchedulerQueuedV2') {
    $readyObject = $ready | ConvertFrom-Json
    $readyObject | Add-Member -NotePropertyName observer_version -NotePropertyValue $observerVersion
    $readyObject | Add-Member -NotePropertyName observer_queue_capacity -NotePropertyValue 32768
    $readyObject | Add-Member -NotePropertyName observer_qpc_frequency_hz -NotePropertyValue ([NativeAcceptanceEventQueue]::QpcFrequency)
    $ready = $readyObject | ConvertTo-Json
}
Write-Output $ready
Set-Content -Path $ReadyFile -Value $ready -Encoding UTF8

$suppressTimer = New-Object System.Windows.Forms.Timer
$suppressTimer.Interval = 50
$suppressTimer.Add_Tick({
    [NativeAcceptanceSinkFocus]::SuppressOverlayWindows([uint32]$PID)
})
$suppressTimer.Start()

if ($DurationSeconds -gt 0) {
    $timer = New-Object System.Windows.Forms.Timer
    $timer.Interval = [Math]::Max(1, [int]($DurationSeconds * 1000))
    $timer.Add_Tick({ $timer.Stop(); $form.Close() })
    $timer.Start()
}

if (-not [string]::IsNullOrWhiteSpace($ActivateRequestFile)) {
    $activationTimer = New-Object System.Windows.Forms.Timer
    $activationTimer.Interval = 20
    $activationTimer.Add_Tick({
        if (Test-Path -LiteralPath $ActivateRequestFile) {
            Remove-Item -LiteralPath $ActivateRequestFile -Force -ErrorAction SilentlyContinue
            [void][NativeAcceptanceSinkFocus]::SetForegroundWindow($form.Handle)
        }
    })
    $activationTimer.Start()
}

[System.Windows.Forms.Application]::Run($form)
if ($null -ne $schedulerObserver -and -not $schedulerObserver.Complete()) {
    exit 70
}
