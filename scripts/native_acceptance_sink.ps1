param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("ReceiveOnly", "InertFocusProbe")]
    [string]$Mode,
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$RunId,
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$ReadyFile,
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$EventLog,
    [double]$DurationSeconds = 0
)

$ErrorActionPreference = "Stop"
if ([string]::IsNullOrWhiteSpace($RunId) -or $RunId.Length -gt 128) {
    throw "RunId must contain 1..128 non-whitespace characters"
}

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
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

$form = New-Object System.Windows.Forms.Form
$form.Text = $title
$form.Width = 620
$form.Height = 240
$form.StartPosition = [System.Windows.Forms.FormStartPosition]::CenterScreen
$form.KeyPreview = $true

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
}

$record = {
    param([string]$Kind, [System.Windows.Forms.KeyEventArgs]$Event)
    $state[$Kind]++
    $state.sequence = [long]$state.sequence + 1L
    $label.Text = if ($isProbe) {
        "$title`r`n`r`nThis project-owned window observes accidental input only.`r`nIt never emits gameplay input.`r`n`r`nObserved key presses: $($state.key_press) | releases: $($state.key_release)"
    } else {
        "$title`r`n`r`nThis project-owned window is the only permitted real-input sink.`r`nKeep it as the intended target during SendInput runs.`r`n`r`nObserved key presses: $($state.key_press) | releases: $($state.key_release)"
    }
    $payload = [ordered]@{
        run_id = $RunId
        sequence = [long]$state.sequence
        kind = $Kind
        key_code = [int]$Event.KeyCode
        observed_utc = [DateTime]::UtcNow.ToString("O")
    } | ConvertTo-Json -Compress
    $eventWriter.WriteLine($payload)
    $eventWriter.Flush()
}

# Both roles observe KeyDown/KeyUp. InertFocusProbe is inert because this
# script never emits input; the handlers intentionally remain enabled so a
# wrong-window delivery cannot disappear as an unobserved event.
$form.Add_KeyDown({ param($Sender, $Event) & $record "key_press" $Event })
$form.Add_KeyUp({ param($Sender, $Event) & $record "key_release" $Event })
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
    schema_version = 2
    run_id = $RunId
    role = $role
    sink_kind = $sinkKind
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
