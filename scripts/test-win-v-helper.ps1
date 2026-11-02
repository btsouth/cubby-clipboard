param(
    [int]$TimeoutSeconds = 8,
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$tauriRoot = Join-Path $repoRoot "src-tauri"
$helperPath = Join-Path $tauriRoot "target\debug\win_v_helper.exe"

if (-not $SkipBuild) {
    Push-Location $tauriRoot
    try {
        cargo build --features dev-harness --bin win_v_helper
        if ($LASTEXITCODE -ne 0) {
            throw "win_v_helper build failed"
        }
    }
    finally {
        Pop-Location
    }
}

if (-not (Test-Path -LiteralPath $helperPath)) {
    throw "Helper binary not found at $helperPath"
}

$nativeSource = @'
using System;
using System.Runtime.InteropServices;

public static class CubbyShortcutTestInput
{
    [DllImport("user32.dll")]
    public static extern void keybd_event(byte virtualKey, byte scanCode, uint flags, UIntPtr extraInfo);

    [DllImport("user32.dll")]
    public static extern short GetAsyncKeyState(int virtualKey);
}
'@

if (-not ("CubbyShortcutTestInput" -as [type])) {
    Add-Type -TypeDefinition $nativeSource
}

$keyUp = 0x0002
$testMarker = New-Object UIntPtr ([uint64]0x43554254)
$keys = @{
    Ctrl = 0xA2
    Shift = 0xA0
    Win = 0x5B
    V = 0x56
}

function Send-KeyDown([byte]$key) {
    [CubbyShortcutTestInput]::keybd_event($key, 0, 0, $testMarker)
    Start-Sleep -Milliseconds 35
}

function Send-KeyUp([byte]$key) {
    [CubbyShortcutTestInput]::keybd_event($key, 0, $keyUp, $testMarker)
    Start-Sleep -Milliseconds 35
}

function Send-KeyTap([byte]$key) {
    Send-KeyDown $key
    Send-KeyUp $key
}

function Send-WinV {
    Send-KeyDown $keys.Win
    Send-KeyTap $keys.V
    Send-KeyUp $keys.Win
}

function Release-TestKeys {
    foreach ($key in @($keys.V, $keys.Shift, $keys.Ctrl, $keys.Win, 0x5C, 0xA1, 0xA3, 0xA4, 0xA5)) {
        [CubbyShortcutTestInput]::keybd_event([byte]$key, 0, $keyUp, $testMarker)
    }
}

# Stands in for an app that starts after Cubby and breaks the hook chain: it
# returns without calling the next hook, so older hooks never see the key while
# Windows still acts on it (#376).
$chainBreakingHook = @'
param([int]$Milliseconds, [string]$ReadyPath)
Add-Type -TypeDefinition @"
using System;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Threading;
public static class ChainBreakingHook {
    delegate IntPtr Proc(int code, IntPtr wParam, IntPtr lParam);
    [StructLayout(LayoutKind.Sequential)]
    struct Msg { public IntPtr h; public uint m; public IntPtr w; public IntPtr l; public uint t; public int x; public int y; }
    [DllImport("user32.dll", SetLastError = true)]
    static extern IntPtr SetWindowsHookEx(int id, Proc proc, IntPtr module, uint thread);
    [DllImport("user32.dll")]
    static extern bool PeekMessage(out Msg message, IntPtr window, uint min, uint max, uint remove);
    static Proc keep = (code, wParam, lParam) => IntPtr.Zero;
    public static void Run(int milliseconds, string readyPath) {
        if (SetWindowsHookEx(13, keep, IntPtr.Zero, 0) == IntPtr.Zero) { throw new Exception("hook failed"); }
        System.IO.File.WriteAllText(readyPath, "ready");
        var clock = Stopwatch.StartNew();
        Msg message;
        while (clock.ElapsedMilliseconds < milliseconds) {
            while (PeekMessage(out message, IntPtr.Zero, 0, 0, 1)) { }
            Thread.Sleep(2);
        }
    }
}
"@
[ChainBreakingHook]::Run($Milliseconds, $ReadyPath)
'@

function Wait-Until([scriptblock]$condition, [int]$seconds, [string]$failure) {
    $deadline = (Get-Date).AddSeconds($seconds)
    while (-not (& $condition)) {
        if ((Get-Date) -gt $deadline) {
            throw $failure
        }
        Start-Sleep -Milliseconds 50
    }
}

# A helper started before a chain-breaking hook must take back the front of the
# chain. With no reassert window it must not, which also proves the stand-in
# really does hide keys from the helper.
function Get-ActivationsBehindChainBreakingHook([int]$ReassertWindowSeconds) {
    $tag = "cubby-win-v-chain-$(Get-Date -Format 'yyyyMMdd-HHmmss-fff')"
    $out = Join-Path $env:TEMP "$tag.out.log"
    $err = Join-Path $env:TEMP "$tag.err.log"
    $ready = Join-Path $env:TEMP "$tag.ready"
    $script = Join-Path $env:TEMP "$tag.ps1"
    Set-Content -LiteralPath $script -Value $chainBreakingHook
    $helper = $null
    $hook = $null
    try {
        $helper = Start-Process -FilePath $helperPath -WindowStyle Hidden -PassThru `
            -ArgumentList @("--timeout-seconds", 30, "--accept-injected-test-events",
                "--hook-reassert-window-seconds", $ReassertWindowSeconds) `
            -RedirectStandardOutput $out -RedirectStandardError $err
        Wait-Until { (Test-Path -LiteralPath $out) -and ((Get-Content -LiteralPath $out -Raw) -match '"event":"ready"') } 20 "Helper did not become ready"
        $hook = Start-Process -FilePath "powershell.exe" -WindowStyle Hidden -PassThru `
            -ArgumentList @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $script, "-Milliseconds", 30000, "-ReadyPath", $ready)
        Wait-Until { Test-Path -LiteralPath $ready } 20 "Chain-breaking hook did not become ready"
        # Longer than the helper's reassert interval.
        Start-Sleep -Seconds 3
        Send-WinV
        Start-Sleep -Milliseconds 500
        # If the chord reached Windows, close Clipboard History again.
        Send-KeyTap 0x1B
        return @(Get-Content -LiteralPath $out | Where-Object { $_ -match '"event":"win_v"' }).Count
    }
    finally {
        foreach ($started in @($helper, $hook)) {
            if ($started -and -not $started.HasExited) {
                Stop-Process -Id $started.Id -Force -ErrorAction SilentlyContinue
            }
        }
        Release-TestKeys
        Remove-Item -LiteralPath $script, $ready -ErrorAction SilentlyContinue
    }
}

$stamp = Get-Date -Format "yyyyMMdd-HHmmss-fff"
$stdoutPath = Join-Path $env:TEMP "cubby-win-v-test-$stamp.out.log"
$stderrPath = Join-Path $env:TEMP "cubby-win-v-test-$stamp.err.log"
$process = $null

try {
    Release-TestKeys
    $process = Start-Process `
        -FilePath $helperPath `
        -ArgumentList @("--timeout-seconds", $TimeoutSeconds, "--accept-injected-test-events") `
        -WindowStyle Hidden `
        -RedirectStandardOutput $stdoutPath `
        -RedirectStandardError $stderrPath `
        -PassThru

    $deadline = (Get-Date).AddSeconds(3)
    do {
        Start-Sleep -Milliseconds 50
        $output = if (Test-Path -LiteralPath $stdoutPath) {
            Get-Content -LiteralPath $stdoutPath -Raw
        } else {
            ""
        }
    } while ($output -notmatch '"event":"ready"' -and (Get-Date) -lt $deadline)

    if ($output -notmatch '"event":"ready"') {
        throw "Helper did not become ready. stderr: $(Get-Content -LiteralPath $stderrPath -Raw)"
    }

    # Plain V must pass through without activating Cubby.
    Send-KeyTap $keys.V

    # Two complete chords must each activate.
    Send-WinV
    Send-WinV

    # Holding Win and tapping V twice must activate twice. Pressing Shift next
    # exercises restoration of the still-physical Windows key.
    Send-KeyDown $keys.Win
    Send-KeyTap $keys.V
    Send-KeyTap $keys.V
    Send-KeyTap $keys.Shift
    Send-KeyUp $keys.Win

    # Extra modifiers make this a non-exact chord and must not activate Cubby.
    Send-KeyDown $keys.Ctrl
    Send-WinV
    Send-KeyUp $keys.Ctrl

    $process.WaitForExit(($TimeoutSeconds + 3) * 1000) | Out-Null
    if (-not $process.HasExited) {
        throw "Helper did not stop after its timeout"
    }

    $events = Get-Content -LiteralPath $stdoutPath |
        Where-Object { $_.Trim() } |
        ForEach-Object { $_ | ConvertFrom-Json }

    $activationCount = @($events | Where-Object event -eq "win_v").Count
    $restoreCount = @($events | Where-Object event -eq "win_restored").Count
    $failures = @($events | Where-Object event -in @("error", "injection_failed", "restore_failed"))

    if ($activationCount -ne 4) {
        throw "Expected 4 Win+V activations, observed $activationCount"
    }
    if ($restoreCount -ne 1) {
        throw "Expected 1 Windows-key restoration, observed $restoreCount"
    }
    if ($failures.Count -ne 0) {
        throw "Helper reported failures: $($failures | ConvertTo-Json -Compress)"
    }

    Release-TestKeys
    Start-Sleep -Milliseconds 100
    $keysStillDown = @(
        $keys.Values + @(0x5C, 0xA1, 0xA3, 0xA4, 0xA5) |
            Sort-Object -Unique |
            Where-Object { [CubbyShortcutTestInput]::GetAsyncKeyState($_) -lt 0 }
    )
    if ($keysStillDown.Count -ne 0) {
        throw "Test keys remain logically down: $($keysStillDown -join ', ')"
    }

    $withoutReassert = Get-ActivationsBehindChainBreakingHook -ReassertWindowSeconds 0
    if ($withoutReassert -ne 0) {
        throw "Expected the chain-breaking hook to hide Win+V without reassert, observed $withoutReassert activations"
    }
    $withReassert = Get-ActivationsBehindChainBreakingHook -ReassertWindowSeconds 60
    if ($withReassert -ne 1) {
        throw "Expected 1 Win+V activation after reassert, observed $withReassert"
    }

    [pscustomobject]@{
        Passed = $true
        Activations = $activationCount
        ActivationsBehindChainBreakingHook = $withReassert
        WinRestorations = $restoreCount
        KeysStillDown = 0
        Log = $stdoutPath
    } | Format-List
}
finally {
    Release-TestKeys
    if ($process -and -not $process.HasExited) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
    }
}
