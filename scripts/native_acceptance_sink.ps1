param(
    [string]$ReadyFile,
    [string]$EventLog,
    [double]$DurationSeconds = 0
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

$title = "Sky Auto Player — Native Acceptance Sink"
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
$label.Text = "$title`r`n`r`nThis project-owned window is the only permitted real-input sink.`r`nKeep it as the intended target during SendInput runs.`r`n`r`nObserved key presses: 0 | releases: 0"
$form.Controls.Add($label)

$counts = @{ key_press = 0; key_release = 0 }
$eventWriter = $null
if ($EventLog) {
    $eventWriter = [System.IO.StreamWriter]::new($EventLog, $true, [System.Text.Encoding]::UTF8)
}

$record = {
    param([string]$Kind, [System.Windows.Forms.KeyEventArgs]$Event)
    $counts[$Kind]++
    $label.Text = "$title`r`n`r`nThis project-owned window is the only permitted real-input sink.`r`nKeep it as the intended target during SendInput runs.`r`n`r`nObserved key presses: $($counts.key_press) | releases: $($counts.key_release)"
    if ($null -ne $eventWriter) {
        $payload = @{
            kind = $Kind
            key_code = [int]$Event.KeyCode
            observed_utc = [DateTime]::UtcNow.ToString("O")
        } | ConvertTo-Json -Compress
        $eventWriter.WriteLine($payload)
        $eventWriter.Flush()
    }
}

$form.Add_KeyDown({ param($Sender, $Event) & $record "key_press" $Event })
$form.Add_KeyUp({ param($Sender, $Event) & $record "key_release" $Event })
$form.Add_FormClosed({
    if ($null -ne $eventWriter) {
        $eventWriter.Dispose()
    }
})

$form.CreateControl()
$hwnd = $form.Handle.ToInt64()
$ready = @{
    pid = $PID
    hwnd = $hwnd
    title = $title
    process = "native_acceptance_sink.ps1"
    input_policy = "receives_only; benchmark must use SendInput"
} | ConvertTo-Json
Write-Output $ready
if ($ReadyFile) {
    $parent = Split-Path -Parent $ReadyFile
    if ($parent) {
        New-Item -ItemType Directory -Force -Path $parent | Out-Null
    }
    Set-Content -Path $ReadyFile -Value $ready -Encoding UTF8
}

if ($DurationSeconds -gt 0) {
    $timer = New-Object System.Windows.Forms.Timer
    $timer.Interval = [Math]::Max(1, [int]($DurationSeconds * 1000))
    $timer.Add_Tick({ $timer.Stop(); $form.Close() })
    $timer.Start()
}

[System.Windows.Forms.Application]::Run($form)
