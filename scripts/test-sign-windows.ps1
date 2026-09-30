# Exercise the production signCommand with a credential-free module and real
# child processes. Captured stdout/stderr must close after failure, even when
# the fake signtool spawned a descendant that inherited those handles.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if (-not $IsWindows) {
    throw 'The signing process regression requires Windows and PowerShell 7.'
}

$testDirectory = Join-Path ([System.IO.Path]::GetTempPath()) "cubby-signing-$([guid]::NewGuid().ToString('N'))"
$moduleRoot = Join-Path $testDirectory 'modules'
$moduleDirectory = Join-Path $moduleRoot 'ArtifactSigning/0.1.8'
$signerPath = Join-Path $testDirectory 'signtool.exe'
$signScript = Join-Path $PSScriptRoot 'sign-windows.ps1'
$ownedTestProcesses = [System.Collections.Generic.List[System.Diagnostics.Process]]::new()

function Wait-ForSignerIds {
    param([string]$Prefix)

    $deadline = [datetime]::UtcNow.AddSeconds(10)
    do {
        try {
            $rootId = [int][System.IO.File]::ReadAllText("$Prefix.root")
            $childId = [int][System.IO.File]::ReadAllText("$Prefix.child")
            return @($rootId, $childId)
        } catch [System.IO.IOException] {
            Start-Sleep -Milliseconds 20
        }
    } while ([datetime]::UtcNow -lt $deadline)
    throw "The fake signer and descendant did not start for $Prefix."
}

function Open-TestProcess {
    param([int]$ProcessId)

    $process = [System.Diagnostics.Process]::GetProcessById($ProcessId)
    # Pin its handle now, so subsequent assertions do not refer to a reused PID.
    $null = $process.Handle
    $ownedTestProcesses.Add($process)
    return $process
}

try {
    New-Item -ItemType Directory -Path $moduleDirectory -Force | Out-Null
    $sourcePath = Join-Path $testDirectory 'FakeSigner.cs'
    @'
using System;
using System.Diagnostics;
using System.IO;
using System.Reflection;
using System.Threading;

public static class FakeSigner
{
    public static void Main(string[] args)
    {
        string role = args[0];
        string prefix = args[1];
        string record = prefix + "." + role;
        File.WriteAllText(record + ".tmp", Process.GetCurrentProcess().Id.ToString());
        File.Move(record + ".tmp", record);
        if (role == "root")
        {
            Process.Start(new ProcessStartInfo {
                FileName = Assembly.GetExecutingAssembly().Location,
                Arguments = "child \"" + prefix + "\"",
                UseShellExecute = false,
                CreateNoWindow = true
            });
        }
        Console.WriteLine("Fake signer " + role + " holds its inherited output handle.");
        Thread.Sleep(300000);
    }
}
'@ | Set-Content -LiteralPath $sourcePath -Encoding utf8

    $compiler = Join-Path $env:WINDIR 'Microsoft.NET/Framework64/v4.0.30319/csc.exe'
    if (-not (Test-Path -LiteralPath $compiler)) {
        throw "Windows C# compiler not found: $compiler"
    }
    & $compiler /nologo /target:exe "/out:$signerPath" $sourcePath
    if ($LASTEXITCODE -ne 0) { throw 'Compiling the fake signer failed.' }

    @'
@{
    RootModule = 'ArtifactSigning.psm1'
    ModuleVersion = '0.1.8'
    FunctionsToExport = @('Invoke-ArtifactSigning')
}
'@ | Set-Content -LiteralPath (Join-Path $moduleDirectory 'ArtifactSigning.psd1') -Encoding utf8
    @'
function Invoke-ArtifactSigning {
    [CmdletBinding()]
    param(
        [string]$Endpoint,
        [string]$CodeSigningAccountName,
        [string]$CertificateProfileName,
        [string]$Files,
        [string]$FileDigest,
        [string]$TimestampRfc3161,
        [string]$TimestampDigest,
        [string]$Description,
        [string]$DescriptionUrl,
        [int]$Timeout = 300,
        [switch]$ExcludeEnvironmentCredential,
        [switch]$ExcludeWorkloadIdentityCredential,
        [switch]$ExcludeManagedIdentityCredential,
        [switch]$ExcludeSharedTokenCacheCredential,
        [switch]$ExcludeVisualStudioCredential,
        [switch]$ExcludeVisualStudioCodeCredential,
        [switch]$ExcludeAzureCliCredential,
        [switch]$ExcludeAzurePowerShellCredential,
        [switch]$ExcludeAzureDeveloperCliCredential,
        [switch]$ExcludeInteractiveBrowserCredential
    )
    $bound = @{}
    foreach ($entry in $PSBoundParameters.GetEnumerator()) {
        $bound[$entry.Key] = if ($entry.Value -is [System.Management.Automation.SwitchParameter]) {
            [bool]$entry.Value
        } else { $entry.Value }
    }
    $bound | ConvertTo-Json | Set-Content -LiteralPath $env:CUBBY_TEST_BINDINGS -Encoding utf8
    if ($env:CUBBY_TEST_SIGNING_MODE -eq 'unsigned') { return }

    $process = Start-Process -FilePath $env:CUBBY_TEST_SIGNTOOL `
        -ArgumentList @('root', ('"{0}"' -f $env:CUBBY_TEST_PID_PREFIX)) -NoNewWindow -PassThru
    $deadline = [datetime]::UtcNow.AddSeconds(10)
    while (-not (Test-Path -LiteralPath "$env:CUBBY_TEST_PID_PREFIX.child")) {
        if ([datetime]::UtcNow -ge $deadline) { throw 'Fake descendant did not start.' }
        Start-Sleep -Milliseconds 20
    }
    if ($env:CUBBY_TEST_SIGNING_MODE -eq 'failure') { throw 'Fake signer failed after spawning.' }
    # Deliberately reproduce the pinned module's lack of timeout cleanup.
    Wait-Process -InputObject $process -Timeout $Timeout -ErrorAction Stop
}
'@ | Set-Content -LiteralPath (Join-Path $moduleDirectory 'ArtifactSigning.psm1') -Encoding utf8

    $unrelatedPrefix = Join-Path $testDirectory 'unrelated'
    $unrelated = Start-Process -FilePath $signerPath `
        -ArgumentList @('root', ('"{0}"' -f $unrelatedPrefix)) -NoNewWindow -PassThru
    $ownedTestProcesses.Add($unrelated)
    $unrelatedIds = @(Wait-ForSignerIds -Prefix $unrelatedPrefix)
    $unrelatedChild = Open-TestProcess -ProcessId $unrelatedIds[1]
    $unsignedFile = Join-Path $testDirectory 'unsigned.exe'
    Copy-Item -LiteralPath $signerPath -Destination $unsignedFile

    foreach ($mode in @('timeout', 'failure', 'unsigned')) {
        $prefix = Join-Path $testDirectory $mode
        $bindingsPath = "$prefix.bindings.json"
        $logPath = "$prefix.log"
        $startInfo = [System.Diagnostics.ProcessStartInfo]::new((Get-Process -Id $PID).Path)
        $startInfo.UseShellExecute = $false
        $startInfo.CreateNoWindow = $true
        $startInfo.RedirectStandardOutput = $true
        $startInfo.RedirectStandardError = $true
        foreach ($argument in @('-NoProfile', '-File', $signScript, '-Path', $unsignedFile, '-SigningTimeoutSeconds', '1')) {
            $startInfo.ArgumentList.Add($argument)
        }
        # Override only the child environment; never use signing configuration,
        # credentials, or PowerShell modules from the test caller.
        $startInfo.Environment['PSModulePath'] = "$moduleRoot$([System.IO.Path]::PathSeparator)$env:PSModulePath"
        $startInfo.Environment['ARTIFACT_SIGNING_ENDPOINT'] = 'https://example.invalid'
        $startInfo.Environment['ARTIFACT_SIGNING_ACCOUNT_NAME'] = 'fake-account'
        $startInfo.Environment['ARTIFACT_SIGNING_CERTIFICATE_PROFILE_NAME'] = 'fake-profile'
        $startInfo.Environment['CUBBY_REQUIRE_SIGNING'] = '1'
        $startInfo.Environment['CUBBY_SIGN_LOG'] = $logPath
        $startInfo.Environment['CUBBY_TEST_SIGNTOOL'] = $signerPath
        $startInfo.Environment['CUBBY_TEST_PID_PREFIX'] = $prefix
        $startInfo.Environment['CUBBY_TEST_BINDINGS'] = $bindingsPath
        $startInfo.Environment['CUBBY_TEST_SIGNING_MODE'] = $mode
        $worker = [System.Diagnostics.Process]::Start($startInfo)
        $ownedTestProcesses.Add($worker)
        $stdout = $worker.StandardOutput.ReadToEndAsync()
        $stderr = $worker.StandardError.ReadToEndAsync()
        if ($mode -ne 'unsigned') {
            $signerIds = @(Wait-ForSignerIds -Prefix $prefix)
            # They may already be terminated by the immediate-failure cleanup;
            # PID-based checks below also handle that case.
        }
        if (-not $worker.WaitForExit(20000)) { throw "${mode}: the signing worker did not fail promptly." }
        if (-not $stdout.Wait(5000) -or -not $stderr.Wait(5000)) {
            throw "${mode}: inherited output pipes remained open after the worker exited."
        }
        if ($worker.ExitCode -eq 0) { throw "${mode}: failed/unsigned signing returned success." }

        $bindings = Get-Content -LiteralPath $bindingsPath -Raw | ConvertFrom-Json -AsHashtable
        foreach ($excluded in @('WorkloadIdentity', 'ManagedIdentity', 'SharedTokenCache', 'VisualStudio',
                'VisualStudioCode', 'AzurePowerShell', 'AzureDeveloperCli', 'InteractiveBrowser')) {
            if ($bindings["Exclude${excluded}Credential"] -ne $true) {
                throw "The pinned action's $excluded exclusion was not forwarded."
            }
        }
        foreach ($enabled in @('Environment', 'AzureCli')) {
            if ($bindings["Exclude${enabled}Credential"]) { throw "$enabled must remain enabled." }
        }
        if ($bindings['Timeout'] -ne 1) { throw 'The signer timeout was not forwarded.' }
        $log = Get-Content -LiteralPath $logPath -Raw
        if ($mode -eq 'unsigned') {
            if ($log -notmatch "after signing") { throw 'Unsigned artifact validation was not reached.' }
        } else {
            if ($log -notmatch 'terminating owned signtool PID') { throw "${mode}: owned cleanup was not logged." }
            foreach ($signerId in $signerIds) {
                $remaining = Get-Process -Id $signerId -ErrorAction SilentlyContinue
                if ($null -ne $remaining -and -not $remaining.HasExited) {
                    $ownedTestProcesses.Add($remaining)
                    throw "${mode}: owned process $signerId is still running."
                }
            }
        }
        if ($unrelated.HasExited -or $unrelatedChild.HasExited) {
            throw "${mode}: cleanup killed an unrelated signer or its descendant."
        }
        Write-Host "Signing $mode regression passed; unrelated signer survived."
    }
} finally {
    # These are test-owned handles, never a name-wide process sweep. Also clean
    # fixture PIDs if an assertion failed before a handle could be recorded.
    foreach ($fixture in Get-ChildItem -LiteralPath $testDirectory -File -ErrorAction SilentlyContinue |
            Where-Object { $_.Extension -in @('.root', '.child') }) {
        try {
            $process = [System.Diagnostics.Process]::GetProcessById([int][System.IO.File]::ReadAllText($fixture.FullName))
            if (-not $process.HasExited -and $process.MainModule.FileName -eq $signerPath) { $process.Kill($true) }
            $process.Dispose()
        } catch [System.ArgumentException] { }
    }
    foreach ($process in $ownedTestProcesses) {
        try { if (-not $process.HasExited) { $process.Kill($true) } } finally { $process.Dispose() }
    }
    Remove-Item -LiteralPath $testDirectory -Recurse -Force -ErrorAction SilentlyContinue
}
