<#
.SYNOPSIS
    Authenticode-signs a single file with Azure Trusted Signing.

.DESCRIPTION
    Invoked by Tauri's bundle.windows.signCommand for every executable it
    bundles, so the app binary and the NSIS uninstaller are signed *before*
    they are packed into the installer. Signing only the finished installer
    (which is what CI did previously) leaves an unsigned cubby.exe on disk,
    and an unsigned binary in %LOCALAPPDATA% that writes a Run key is what
    trips Defender's Behavior:Win32/Persistence.A!ml heuristic.

    Signing goes through Invoke-ArtifactSigning from the PSGallery
    ArtifactSigning module -- the same cmdlet Azure/artifact-signing-action
    wraps, so the in-build signing here and the post-build installer signing in
    the release workflow use one mechanism. Credentials are resolved by that
    module's DefaultAzureCredential chain, which picks up the `az login` that
    azure/login performs with OIDC in CI. Do not swap this for
    trusted-signing-cli: that tool requires an AZURE_CLIENT_SECRET and runs
    `az login --service-principal` internally, which both reintroduces a
    long-lived credential and clobbers the OIDC session the later signing
    steps depend on.

    Configuration comes from the environment so no tenant details live in the
    repo:

        ARTIFACT_SIGNING_ENDPOINT
        ARTIFACT_SIGNING_ACCOUNT_NAME
        ARTIFACT_SIGNING_CERTIFICATE_PROFILE_NAME

    When those variables are absent the script exits successfully without
    signing, so ordinary local builds keep working for contributors who have
    no access to the signing account. Set CUBBY_REQUIRE_SIGNING=1 (CI does) to
    turn a missing configuration into a hard failure instead -- otherwise a
    typo in a workflow variable would silently ship unsigned binaries again.

    Everything this script prints is also appended to CUBBY_SIGN_LOG when that
    is set. Tauri runs signCommand with captured stdio and discards the output
    on a non-zero exit, so without the log file a failure in here surfaces as
    nothing but `failed to run pwsh`.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Path,

    [ValidateRange(1, 3600)]
    [int]$SigningTimeoutSeconds = 300
)

$ErrorActionPreference = "Stop"

# Pinned so a module release cannot change signing behaviour mid-release. This
# is the version Azure/artifact-signing-action@v2 installs.
$moduleVersion = "0.1.8"

$logPath = $env:CUBBY_SIGN_LOG

function Write-SignLog {
    param([string]$Message)

    $line = "sign-windows: $Message"
    Write-Host $line
    if (-not [string]::IsNullOrWhiteSpace($logPath)) {
        Add-Content -LiteralPath $logPath -Value $line
    }
}

# Records the reason in the log before the exception unwinds, so a failure is
# legible even though Tauri throws away our stdout.
function Stop-WithError {
    param([string]$Message)

    Write-SignLog "ERROR: $Message"
    throw "sign-windows: $Message"
}

# ArtifactSigning 0.1.8's timeout ends Wait-Process but leaves signtool running.
# Its inherited stdout/stderr handles can then keep Tauri's captured-output
# wait open after this pwsh process fails. Kill only new signtool children of
# this invocation, including their descendants; never sweep by process name.
function Stop-OwnedSignToolTrees {
    param([datetime]$StartedAt)

    $roots = @(Get-CimInstance -ClassName Win32_Process `
        -Filter "ParentProcessId = $PID AND Name = 'signtool.exe'" -ErrorAction Stop)
    foreach ($root in $roots) {
        if ($null -eq $root.CreationDate) {
            throw "Cannot establish creation time for owned signtool PID $($root.ProcessId)."
        }
        $createdAt = ([datetime]$root.CreationDate).ToUniversalTime()
        if ($createdAt -lt $StartedAt) {
            continue
        }

        $process = $null
        try {
            try {
                $process = [System.Diagnostics.Process]::GetProcessById([int]$root.ProcessId)
                $null = $process.Handle
            } catch [System.ArgumentException] {
                continue # The signer exited between enumeration and opening it.
            }
            if ($process.HasExited) {
                continue
            }
            # Hold the opened process handle and verify its start time, so a
            # reused PID cannot turn a stale CIM record into somebody else's kill.
            $openedAt = $process.StartTime.ToUniversalTime()
            if ($process.ProcessName -ne 'signtool' -or
                [Math]::Abs(($openedAt - $createdAt).TotalMilliseconds) -gt 1) {
                throw "Identity changed for owned signtool PID $($root.ProcessId); refusing cleanup."
            }
            Write-SignLog "terminating owned signtool PID $($root.ProcessId) and its descendants"
            try {
                $process.Kill($true)
            } catch [System.InvalidOperationException] {
                if (-not $process.HasExited) { throw }
            }
            if (-not $process.WaitForExit(5000)) {
                throw "Owned signtool PID $($root.ProcessId) did not exit after termination."
            }
        } finally {
            if ($null -ne $process) { $process.Dispose() }
        }
    }
}

$endpoint = $env:ARTIFACT_SIGNING_ENDPOINT
$account = $env:ARTIFACT_SIGNING_ACCOUNT_NAME
$profileName = $env:ARTIFACT_SIGNING_CERTIFICATE_PROFILE_NAME
$required = $env:CUBBY_REQUIRE_SIGNING -eq "1"

$missing = @()
if ([string]::IsNullOrWhiteSpace($endpoint)) { $missing += "ARTIFACT_SIGNING_ENDPOINT" }
if ([string]::IsNullOrWhiteSpace($account)) { $missing += "ARTIFACT_SIGNING_ACCOUNT_NAME" }
if ([string]::IsNullOrWhiteSpace($profileName)) { $missing += "ARTIFACT_SIGNING_CERTIFICATE_PROFILE_NAME" }

if ($missing.Count -gt 0) {
    $joined = $missing -join ", "
    if ($required) {
        Stop-WithError "CUBBY_REQUIRE_SIGNING is set but the signing configuration is incomplete: $joined."
    }
    Write-SignLog "skipping $Path (unset: $joined)."
    exit 0
}

if (-not (Test-Path -LiteralPath $Path)) {
    Stop-WithError "nothing to sign at $Path."
}

# Import the pinned version rather than only checking that it is installed:
# an unqualified Invoke-ArtifactSigning auto-loads the *highest* available
# version, so a second copy on the runner would quietly sign with something
# other than the version this script claims to pin. Each signCommand call gets
# a fresh pwsh process, so nothing else can already be loaded here and this
# import fully determines which version signs. It also fails loudly when the
# module is missing, which is what the installation check was for.
try {
    Import-Module -Name ArtifactSigning -RequiredVersion $moduleVersion -Force -ErrorAction Stop
}
catch {
    Stop-WithError "could not load the ArtifactSigning module $moduleVersion. Install it with 'Install-Module -Name ArtifactSigning -RequiredVersion $moduleVersion -Force -Repository PSGallery'. $($_.Exception.Message)"
}

$resolved = (Resolve-Path -LiteralPath $Path).Path
Write-SignLog "signing $resolved"

# Description/DescriptionUrl populate the text and link shown in the UAC and
# SmartScreen prompts. Without them Windows falls back to the bare file name.
# Digest and timestamp settings mirror the post-build signing step in
# release.yml so every shipped artifact carries the same signature shape.
$signingStartedAt = [datetime]::UtcNow
try {
    # Match Azure/artifact-signing-action v2's defaults. CI authenticates through
    # azure/login's Azure CLI session; unrelated developer/VM/browser credentials
    # must not delay it or open an interactive login on the build runner.
    Invoke-ArtifactSigning `
        -Endpoint $endpoint `
        -CodeSigningAccountName $account `
        -CertificateProfileName $profileName `
        -Files $resolved `
        -FileDigest "SHA256" `
        -TimestampRfc3161 "http://timestamp.acs.microsoft.com" `
        -TimestampDigest "SHA256" `
        -Description "Cubby Clipboard" `
        -DescriptionUrl "https://cubbyclipboard.com" `
        -Timeout $SigningTimeoutSeconds `
        -ExcludeWorkloadIdentityCredential `
        -ExcludeManagedIdentityCredential `
        -ExcludeSharedTokenCacheCredential `
        -ExcludeVisualStudioCredential `
        -ExcludeVisualStudioCodeCredential `
        -ExcludeAzurePowerShellCredential `
        -ExcludeAzureDeveloperCliCredential `
        -ExcludeInteractiveBrowserCredential

    $signature = Get-AuthenticodeSignature -LiteralPath $resolved
    if ($signature.Status -ne "Valid") {
        throw "$resolved is '$($signature.Status)' after signing: $($signature.StatusMessage)"
    }
}
catch {
    $signingFailure = $_.Exception.Message
    try {
        Stop-OwnedSignToolTrees -StartedAt $signingStartedAt
    } catch {
        Stop-WithError "signing failed for ${resolved}: $signingFailure Cleanup also failed: $($_.Exception.Message)"
    }
    Stop-WithError "signing failed for ${resolved}: $signingFailure"
}

Write-SignLog "signed $resolved ($($signature.SignerCertificate.Subject))"
