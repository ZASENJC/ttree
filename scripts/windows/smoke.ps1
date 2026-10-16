param([Parameter(Mandatory=$true)][string]$Exe, [Parameter(Mandatory=$true)][string]$Output)
$ErrorActionPreference = 'Stop'
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class TTreeSmoke {
    public delegate bool EnumProc(IntPtr h, IntPtr p);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc proc, IntPtr data);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, System.Text.StringBuilder text, int max);
    [DllImport("user32.dll", EntryPoint="GetClassLongPtrW")] public static extern IntPtr GetClassLongPtr(IntPtr h, int index);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out Rect rect);
    [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr h, int id);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
    public struct Rect { public int Left, Top, Right, Bottom; }
    public static IntPtr Find(uint target) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h,p) => { uint pid; GetWindowThreadProcessId(h, out pid);
            var name = new System.Text.StringBuilder(256); GetClassName(h, name, 256);
            if (pid == target && name.ToString() == "TTreePureRustWin32MainClass") { found = h; return false; } return true;
        }, IntPtr.Zero);
        return found;
    }
}
'@
Add-Type -AssemblyName System.Drawing
New-Item -ItemType Directory -Force $Output | Out-Null
$ExePath = (Resolve-Path $Exe).Path
$Children = [System.Collections.Generic.List[System.Diagnostics.Process]]::new()
function Start-App([string[]]$LaunchArgs = @()) {
    $Options = @{FilePath=$ExePath; PassThru=$true; WindowStyle='Hidden'}
    if ($LaunchArgs.Count) { $Options.ArgumentList = $LaunchArgs }
    $Child = Start-Process @Options
    $Children.Add($Child)
    return $Child
}
function Wait-Window($Child) {
    for ($i=0; $i -lt 100; $i++) {
        if ($Child.HasExited) { throw "App exited with $($Child.ExitCode)" }
        $Found = [TTreeSmoke]::Find($Child.Id)
        if ($Found -ne [IntPtr]::Zero) { return $Found }
        Start-Sleep -Milliseconds 100
    }
    throw 'Native window was not created'
}
function Assert-Visible([IntPtr]$Handle, [bool]$Expected, [string]$Message) {
    for ($i=0; $i -lt 50; $i++) {
        if ([TTreeSmoke]::IsWindowVisible($Handle) -eq $Expected) { return }
        Start-Sleep -Milliseconds 100
    }
    throw $Message
}
function Assert-Forwarded($Child) {
    if (-not $Child.WaitForExit(10000)) { throw 'Repeated launch created another resident instance' }
    if ($Child.ExitCode -ne 0) { throw "Repeated launch failed with $($Child.ExitCode)" }
}
function Stop-App($Child, [IntPtr]$Handle) {
    [TTreeSmoke]::SendMessage($Handle, 0x111, [IntPtr]905, [IntPtr]::Zero) | Out-Null
    if (-not $Child.WaitForExit(10000)) { throw 'App did not exit through its tray action' }
}
try {
    $Process = Start-App
    $Window = Wait-Window $Process
    Assert-Visible $Window $true 'Double-click launch did not show the window'
    if ([TTreeSmoke]::GetClassLongPtr($Window, -14) -eq [IntPtr]::Zero) { throw 'Embedded TTREE icon did not load' }
    # Esc is sent to the real source edit, exercising its keyboard handler.
    $SourceEdit = [TTreeSmoke]::GetDlgItem($Window, 201)
    if ($SourceEdit -eq [IntPtr]::Zero) { throw 'Source input was not created' }
    [TTreeSmoke]::SendMessage($SourceEdit, 0x100, [IntPtr]0x1b, [IntPtr]::Zero) | Out-Null
    Assert-Visible $Window $false 'Escape did not hide the window'
    Assert-Forwarded (Start-App)
    Assert-Visible $Window $true 'Repeated double-click did not reopen the existing window'
    # Relaunching while visible must keep the window open, rather than toggle it.
    Assert-Forwarded (Start-App)
    Assert-Visible $Window $true 'Repeated launch hid the visible window'
    [TTreeSmoke]::SendMessage($Window, 0x10, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    Assert-Visible $Window $false 'Close did not hide the window'
    [TTreeSmoke]::SendMessage($Window, 0x111, [IntPtr]901, [IntPtr]::Zero) | Out-Null
    [TTreeSmoke]::SetForegroundWindow($Window) | Out-Null
    Start-Sleep -Seconds 1
    if (-not [TTreeSmoke]::IsWindowVisible($Window)) { throw 'Translation window did not show' }
    $Rect = New-Object TTreeSmoke+Rect
    [TTreeSmoke]::GetWindowRect($Window, [ref]$Rect) | Out-Null
    $Width = $Rect.Right - $Rect.Left
    $Height = $Rect.Bottom - $Rect.Top
    if ($Width -lt 300 -or $Height -lt 200) { throw 'Unexpected native window dimensions' }
    $Bitmap = New-Object System.Drawing.Bitmap($Width, $Height)
    $Graphics = [System.Drawing.Graphics]::FromImage($Bitmap)
    try {
        # Capture this app even when Windows keeps the user's current app in front.
        $Dc = $Graphics.GetHdc()
        try {
            if (-not [TTreeSmoke]::PrintWindow($Window, $Dc, 2)) { throw 'Native window capture failed' }
        } finally { $Graphics.ReleaseHdc($Dc) }
        $Bitmap.Save((Join-Path (Resolve-Path $Output) 'windows.png'))
    } finally { $Graphics.Dispose(); $Bitmap.Dispose() }
    Stop-App $Process $Window

    $Background = Start-App @('--hidden')
    $HiddenWindow = Wait-Window $Background
    Start-Sleep -Milliseconds 500
    Assert-Visible $HiddenWindow $false 'Background launch unexpectedly showed a window'
    Assert-Forwarded (Start-App @('--hidden'))
    Assert-Visible $HiddenWindow $false 'Repeated background launch showed a window'
    Assert-Forwarded (Start-App @('--show'))
    Assert-Visible $HiddenWindow $true '--show did not activate the background instance'
    Stop-App $Background $HiddenWindow

    # Two cold launches in quick succession must still leave exactly one resident process.
    $First = Start-App
    $Second = Start-App
    for ($i=0; $i -lt 100; $i++) {
        if ($First.HasExited -or $Second.HasExited) { break }
        Start-Sleep -Milliseconds 100
    }
    if ($First.HasExited -eq $Second.HasExited) { throw 'Simultaneous launches did not leave exactly one instance' }
    $Primary = if ($First.HasExited) { $Second } else { $First }
    $Forwarded = if ($First.HasExited) { $First } else { $Second }
    Assert-Forwarded $Forwarded
    $RaceWindow = Wait-Window $Primary
    Assert-Visible $RaceWindow $true 'Simultaneous launches did not show the window'
    Stop-App $Primary $RaceWindow

    @{ result='passed'; pid=$Process.Id; width=$Width; height=$Height; checks=@('double-click shows window', 'embedded TTREE icon loaded', 'Escape hides window', 'repeated launch reopens same instance', 'visible relaunch stays visible', 'tray action opens translation', 'background launch stays hidden', 'background relaunch stays hidden', '--show activates existing instance', 'simultaneous launch keeps one instance') } |
        ConvertTo-Json | Set-Content (Join-Path $Output 'result.json')
} finally {
    foreach ($Child in $Children) {
        if (-not $Child.HasExited) { Stop-Process -Id $Child.Id -Force }
    }
}
