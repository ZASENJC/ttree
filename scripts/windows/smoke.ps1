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
$Process = Start-Process (Resolve-Path $Exe) -PassThru
try {
    $Window = [IntPtr]::Zero
    for ($i=0; $i -lt 100; $i++) {
        if ($Process.HasExited) { throw "App exited with $($Process.ExitCode)" }
        $Window = [TTreeSmoke]::Find($Process.Id)
        if ($Window -ne [IntPtr]::Zero) { break }
        Start-Sleep -Milliseconds 100
    }
    if ($Window -eq [IntPtr]::Zero) { throw 'Native window was not created' }
    if ([TTreeSmoke]::GetClassLongPtr($Window, -14) -eq [IntPtr]::Zero) { throw 'Embedded TTREE icon did not load' }
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
        $Graphics.CopyFromScreen($Rect.Left, $Rect.Top, 0, 0, $Bitmap.Size)
        $Bitmap.Save((Join-Path (Resolve-Path $Output) 'windows.png'))
    } finally { $Graphics.Dispose(); $Bitmap.Dispose() }
    @{ result='passed'; pid=$Process.Id; width=$Width; height=$Height; checks=@('process alive', 'native window created', 'embedded TTREE icon loaded', 'tray action opens translation') } |
        ConvertTo-Json | Set-Content (Join-Path $Output 'result.json')
} finally {
    if (-not $Process.HasExited) { Stop-Process -Id $Process.Id -Force }
}
