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
    [double]$DurationSeconds = 0
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$nativeAcceptanceReferences = @(
    [System.Windows.Forms.Form].Assembly.Location,
    [System.Windows.Forms.Message].Assembly.Location,
    [System.Drawing.Font].Assembly.Location,
    [System.ComponentModel.Component].Assembly.Location
) | Select-Object -Unique
Add-Type -ReferencedAssemblies $nativeAcceptanceReferences -TypeDefinition @"
using System;
using System.Windows.Forms;

public sealed class NativeKeyMessageData : EventArgs {
    public string Kind { get; private set; }
    public int ScanCode { get; private set; }
    public bool Extended { get; private set; }
    public int VirtualKey { get; private set; }
    public int Message { get; private set; }

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

    protected override void WndProc(ref Message message) {
        NativeKeyMessageData decoded = NativeKeyMessageData.DecodeKeyMessage(
            message.Msg, message.WParam, message.LParam);
        if (decoded != null && NativeKeyMessage != null) {
            NativeKeyMessage(this, decoded);
        }
        base.WndProc(ref message);
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

    Write-Output "native acceptance sink decoder self-test: PASS"
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

Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class NativeAcceptanceSinkFocus {
    [DllImport("user32.dll")]
    public static extern bool SetForegroundWindow(IntPtr hWnd);
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

$eventWriter = [System.IO.StreamWriter]::new(
    $EventLog,
    $true,
    [System.Text.UTF8Encoding]::new($false)
)
$state = @{
    key_press = 0
    key_release = 0
    sequence = 0L
    event_log_id = [guid]::NewGuid().ToString("N")
}

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

# Both roles observe native keyboard messages. InertFocusProbe is inert because
# this script never emits input; the WndProc observer remains enabled so a
# wrong-window delivery cannot disappear as an unobserved event.
$form.Add_NativeKeyMessage({ param($Sender, $Event) & $record $Sender $Event })
$form.Add_Shown({
    $form.Activate()
    $form.Focus()
    [void][NativeAcceptanceSinkFocus]::SetForegroundWindow($form.Handle)
})
$form.Add_FormClosed({
    if ($null -ne $eventWriter) {
        $eventWriter.Dispose()
    }
})

$form.CreateControl()
$hwnd = $form.Handle.ToInt64()
$processStartTimeFiletime = [long](Get-Process -Id $PID).StartTime.ToUniversalTime().ToFileTimeUtc()
$ready = [ordered]@{
    schema_version = 3
    event_schema_version = 3
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
Write-Output $ready
Set-Content -Path $ReadyFile -Value $ready -Encoding UTF8

if ($DurationSeconds -gt 0) {
    $timer = New-Object System.Windows.Forms.Timer
    $timer.Interval = [Math]::Max(1, [int]($DurationSeconds * 1000))
    $timer.Add_Tick({ $timer.Stop(); $form.Close() })
    $timer.Start()
}

[System.Windows.Forms.Application]::Run($form)
