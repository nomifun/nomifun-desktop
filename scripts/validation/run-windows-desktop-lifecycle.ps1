param(
    [Parameter(Mandatory = $true)][string]$DesktopExe,
    [ValidateSet('quit', 'parent-death')][string]$Mode = 'quit',
    [switch]$KeepArtifacts,
    [switch]$WithAgentResources,
    [switch]$StopAgentBeforeQuit,
    [string]$NodeExe = 'node'
)
$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'A real Windows Desktop host is required.' }
if ($StopAgentBeforeQuit -and (-not $WithAgentResources -or $Mode -ne 'quit')) {
    throw 'StopAgentBeforeQuit requires WithAgentResources and the normal Quit mode.'
}
$executable = (Resolve-Path -LiteralPath $DesktopExe).ProviderPath
if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) { throw 'Desktop executable is missing.' }
$executableFile = Get-Item -LiteralPath $executable
$executableIdentity = @{path=$executable; sha256=(Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash.ToLowerInvariant();
    modified_at_utc=$executableFile.LastWriteTimeUtc.ToString('o'); length_bytes=[long]$executableFile.Length}
$agentSidecarScript = $null
$nodeExecutable = $null
$sidecarIdentity = $null
$driverIdentity = @{path=$PSCommandPath; sha256=(Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant()}
if ($WithAgentResources) {
    $agentSidecarScript = Join-Path $PSScriptRoot 'desktop-agent-resources.mjs'
    if (-not (Test-Path -LiteralPath $agentSidecarScript -PathType Leaf)) { throw 'The dedicated Agent resource sidecar is missing.' }
    $agentSidecarScript = (Resolve-Path -LiteralPath $agentSidecarScript).ProviderPath
    $nodeExecutable = (Get-Command -Name $NodeExe -CommandType Application -ErrorAction Stop).Source
    $nodeExecutable = (Resolve-Path -LiteralPath $nodeExecutable).ProviderPath
    if ($agentSidecarScript.Contains('"') -or $nodeExecutable.Contains('"')) { throw 'Native fixture executable paths cannot contain a quote.' }
    $sidecarIdentity = @{path=$agentSidecarScript; sha256=(Get-FileHash -LiteralPath $agentSidecarScript -Algorithm SHA256).Hash.ToLowerInvariant();
        node_path=$nodeExecutable; node_sha256=(Get-FileHash -LiteralPath $nodeExecutable -Algorithm SHA256).Hash.ToLowerInvariant()}
}

# Random UUIDv7 is a fixture admission token, never a product Session identity.
$random = New-Object byte[] 16
$generator = [Security.Cryptography.RandomNumberGenerator]::Create()
try { $generator.GetBytes($random) } finally { $generator.Dispose() }
$milliseconds = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
for ($offset = 5; $offset -ge 0; $offset--) {
    $random[$offset] = [byte]($milliseconds -band 255)
    $milliseconds = $milliseconds -shr 8
}
$random[6] = [byte](($random[6] -band 15) -bor 112)
$random[8] = [byte](($random[8] -band 63) -bor 128)
$hex = [BitConverter]::ToString($random).Replace('-', '').ToLowerInvariant()
$token = '{0}-{1}-{2}-{3}-{4}' -f $hex.Substring(0,8), $hex.Substring(8,4), $hex.Substring(12,4), $hex.Substring(16,4), $hex.Substring(20,12)
$temporaryParent = (Resolve-Path -LiteralPath ([IO.Path]::GetTempPath())).ProviderPath.TrimEnd('\')
$root = Join-Path $temporaryParent "nomifun-lifecycle-$token"
[void][IO.Directory]::CreateDirectory($root)
$data = Join-Path $root 'data'
$work = Join-Path $root 'work'
[void][IO.Directory]::CreateDirectory($data)
[void][IO.Directory]::CreateDirectory($work)
[IO.File]::WriteAllText((Join-Path $root 'fixture-sentinel'), $token)
[IO.File]::WriteAllText((Join-Path $root 'binary-identity.json'), ($executableIdentity | ConvertTo-Json -Compress))
[IO.File]::WriteAllText((Join-Path $root 'driver-identity.json'), (@{driver=$driverIdentity; agent_sidecar=$sidecarIdentity} | ConvertTo-Json -Depth 4 -Compress))
if ($WithAgentResources) {
    [IO.File]::WriteAllText((Join-Path $root 'agent-resources.request'), $token)
}

if (-not ('NomiLifecycleProcessWitness' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class NomiLifecycleProcessWitness {
    [DllImport("kernel32.dll", SetLastError=true)]
    public static extern IntPtr OpenProcess(uint rights, bool inherit, uint processId);
    [DllImport("kernel32.dll", SetLastError=true)]
    public static extern uint WaitForSingleObject(IntPtr handle, uint milliseconds);
    [DllImport("kernel32.dll", SetLastError=true)]
    public static extern bool CloseHandle(IntPtr handle);
}
'@
}
$hostProcess = $null
$externalOwnerProcess = $null
$externalOwnerHandle = [IntPtr]::Zero
$externalOwnerAliveBeforeExit = $false
$externalOwnerAliveAfterExit = $false
$browserHandles = @()
$agentHandles = @()
$agentWitnesses = @()
$agentReceipt = $null
$agentStopReceipt = $null
$agentStopObservation = $null
$agentSidecar = $null
$agentStdout = $null
$agentStderr = $null
$passed = $false
$passReceipt = $null
$stdout = $null
$stderr = $null
function Canonical-FixturePath([string]$path) {
    if ($path.StartsWith('\\?\UNC\')) { $path = '\\' + $path.Substring(8) }
    elseif ($path.StartsWith('\\?\')) { $path = $path.Substring(4) }
    return [IO.Path]::GetFullPath($path)
}
function Assert-OriginalHostAncestor($process, $originalHost) {
    $visited = @{}
    $child = $process
    $hostCreated = $originalHost.StartTime.ToUniversalTime()
    while ($child.ParentProcessId -ne $originalHost.Id) {
        $parentId = [uint32]$child.ParentProcessId
        if ($parentId -eq 0 -or $visited.ContainsKey($parentId) -or $visited.Count -ge 64) {
            throw 'The Agent resource has no verified original Desktop ancestry.'
        }
        $visited[$parentId] = $true
        $parent = Get-CimInstance Win32_Process -Filter ('ProcessId=' + $parentId)
        if (-not $parent -or $parent.CreationDate.ToUniversalTime() -lt $hostCreated -or
            $parent.CreationDate -gt $child.CreationDate) {
            throw 'An intermediate Agent process owner is missing or its identity changed.'
        }
        $child = $parent
    }
    if ($originalHost.HasExited -or $child.CreationDate.ToUniversalTime() -lt $hostCreated) {
        throw 'The original Desktop no longer owns the observed Agent resource.'
    }
}
try {
    # This one independent helper belongs to the validation driver, outside
    # the Desktop process tree and its JobObjects. It is deliberately not an
    # installed OS service; survival proves only this controlled shared-owner
    # boundary. The driver retains its exact Process object for final cleanup.
    $externalReady = Join-Path $root 'external-owner-ready.json'
    $externalReadyLiteral = $externalReady.Replace("'", "''")
    $externalScript = @'
$ErrorActionPreference = 'Stop'
$readyPath = '{{READY}}'
$receipt = @{token='{{TOKEN}}'; pid=$PID} | ConvertTo-Json -Compress
[IO.File]::WriteAllText(($readyPath + '.tmp'), $receipt)
[IO.File]::Move(($readyPath + '.tmp'), $readyPath)
$held = New-Object System.Threading.ManualResetEvent($false)
try { [void]$held.WaitOne() } finally { $held.Dispose() }
'@
    $externalScript = $externalScript.Replace('{{READY}}', $externalReadyLiteral).Replace('{{TOKEN}}', $token)
    $externalStart = New-Object Diagnostics.ProcessStartInfo
    $externalStart.FileName = [Diagnostics.Process]::GetCurrentProcess().MainModule.FileName
    $externalStart.Arguments = '-NoLogo -NoProfile -NonInteractive -EncodedCommand ' + [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($externalScript))
    $externalStart.WorkingDirectory = $root
    $externalStart.UseShellExecute = $false
    $externalStart.CreateNoWindow = $true
    $externalStart.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
    $externalOwnerProcess = [Diagnostics.Process]::Start($externalStart)
    $exactExternalProcessHandle = $externalOwnerProcess.Handle
    $externalReadyDeadline = [DateTime]::UtcNow.AddSeconds(15)
    while (-not (Test-Path -LiteralPath $externalReady)) {
        if ($externalOwnerProcess.HasExited) { throw 'The independent validation owner exited before readiness.' }
        if ([DateTime]::UtcNow -ge $externalReadyDeadline) { throw 'The independent validation owner did not publish readiness.' }
        Start-Sleep -Milliseconds 25
    }
    $externalReceipt = [IO.File]::ReadAllText($externalReady) | ConvertFrom-Json
    if ($externalReceipt.token -ne $token -or $externalReceipt.pid -ne $externalOwnerProcess.Id) { throw 'Independent owner readiness has the wrong exact identity.' }
    $externalOwnerHandle = [NomiLifecycleProcessWitness]::OpenProcess(0x00100000, $false, [uint32]$externalOwnerProcess.Id)
    if ($externalOwnerHandle -eq [IntPtr]::Zero -or [NomiLifecycleProcessWitness]::WaitForSingleObject($externalOwnerHandle, 0) -ne 258) {
        throw 'The original independent validation owner was not observed alive.'
    }
    $start = New-Object Diagnostics.ProcessStartInfo
    $start.FileName = $executable
    $start.WorkingDirectory = $root
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.EnvironmentVariables['NOMIFUN_DATA_DIR'] = $data
    $start.EnvironmentVariables['NOMIFUN_WORK_DIR'] = $work
    $start.EnvironmentVariables['NOMIFUN_LIFECYCLE_FIXTURE_DIR'] = $root
    $start.EnvironmentVariables['NOMIFUN_LIFECYCLE_FIXTURE_TOKEN'] = $token
    $start.EnvironmentVariables['NOMIFUN_LIFECYCLE_FIXTURE_MODE'] = $Mode
    $start.EnvironmentVariables['NOMI_LOG_LEVEL'] = 'warn'
    foreach ($name in @('WEBVIEW2_USER_DATA_FOLDER', 'WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS', 'WEBVIEW2_BROWSER_EXECUTABLE_FOLDER',
        'WEBVIEW2_CHANNEL_SEARCH_KIND', 'WEBVIEW2_RELEASE_CHANNELS', 'WEBVIEW2_RELEASE_CHANNEL_PREFERENCE', 'NOMIFUN_RELIABILITY_DROP_TURN_ACK_KEY')) {
        $start.EnvironmentVariables.Remove($name)
    }
    if ($WithAgentResources) {
        foreach ($name in @('NOMIFUN_ADMIN_PASSWORD', 'NOMIFUN_ADMIN_USERNAME', 'JWT_SECRET', 'NOMIFUN_JWT_SECRET')) {
            $start.EnvironmentVariables.Remove($name)
        }
    }
    $hostProcess = [Diagnostics.Process]::Start($start)
    # Bind to the newly created exact OS process before any Quit/crash action.
    $exactHostHandle = $hostProcess.Handle
    $stdout = $hostProcess.StandardOutput.ReadToEndAsync()
    $stderr = $hostProcess.StandardError.ReadToEndAsync()
    $readyPath = Join-Path $root 'ready.json'
    $deadline = [DateTime]::UtcNow.AddSeconds(120)
    while (-not (Test-Path -LiteralPath $readyPath)) {
        if ($hostProcess.HasExited) { throw "Isolated Desktop exited before native readiness ($($hostProcess.ExitCode))." }
        if ([DateTime]::UtcNow -ge $deadline) { throw 'Isolated Desktop did not publish native readiness.' }
        Start-Sleep -Milliseconds 50
    }
    $ready = [IO.File]::ReadAllText($readyPath) | ConvertFrom-Json
    if ($ready.token -ne $token -or $ready.mode -ne $Mode -or $ready.host_pid -ne $hostProcess.Id -or
        $ready.identifier -ne "fun.nomi.lifecycle.fixture.run$($token.Replace('-', ''))" -or
        (Canonical-FixturePath $ready.data_root) -ne (Canonical-FixturePath $data)) {
        throw 'Native fixture readiness does not belong to this exact isolated host.'
    }
    if ($WithAgentResources -and $ready.agent_resources -ne $true) { throw 'The original host did not prepare the explicitly requested Agent resource fixture owner.' }
    # SYNCHRONIZE only: no termination/write authority over the native browser.
    if (@($ready.browser_pids).Count -lt 1 -or @($ready.browser_pids) -notcontains $ready.browser_pid) { throw 'Exact shell/managed native process inventory is missing.' }
    $environments = @($ready.native_environments)
    if ($environments.Count -ne 2 -or @($environments.role) -notcontains 'shell' -or @($environments.role) -notcontains 'managed') {
        throw 'The two original WebView2 Environment ProcessInfos receipts are missing.'
    }
    $fixtureAbsolute = (Canonical-FixturePath $root).TrimEnd('\') + '\'
    $observedNativeIds = @()
    foreach ($environment in $environments) {
        $udf = Canonical-FixturePath $environment.user_data_folder
        if (-not $udf.StartsWith($fixtureAbsolute, [StringComparison]::OrdinalIgnoreCase) -or
            @($environment.processes.kind) -notcontains 'renderer' -or
            @($environment.processes | Where-Object { $_.kind -eq 'browser' -and $_.pid -eq $environment.browser_pid }).Count -ne 1) {
            throw 'A native inventory does not belong to its isolated UDF/browser owner or has no renderer witness.'
        }
        $observedNativeIds += @($environment.processes | ForEach-Object { [uint32]$_.pid })
    }
    $nativeProcessIds = @($ready.native_process_pids | ForEach-Object { [uint32]$_ } | Sort-Object -Unique)
    if ($nativeProcessIds.Count -le @($ready.browser_pids).Count -or $nativeProcessIds -contains [uint32]$hostProcess.Id -or
        $nativeProcessIds -contains [uint32]$externalOwnerProcess.Id -or $hostProcess.Id -eq $externalOwnerProcess.Id -or
        @(Compare-Object $nativeProcessIds @($observedNativeIds | Sort-Object -Unique)).Count -ne 0) {
        throw 'Exact native ProcessInfos inventory omitted or changed its browser/child identities.'
    }
    foreach ($nativeProcessId in $nativeProcessIds) {
        $handle = [NomiLifecycleProcessWitness]::OpenProcess(0x00100000, $false, [uint32]$nativeProcessId)
        if ($handle -ne [IntPtr]::Zero) { $browserHandles += $handle }
        if ($handle -eq [IntPtr]::Zero -or [NomiLifecycleProcessWitness]::WaitForSingleObject($handle, 0) -ne 258) {
            throw "Exact native ProcessInfo $nativeProcessId was not observed alive before the exit action."
        }
    }
    if ($WithAgentResources) {
        $agentReadyPath = Join-Path $root 'agent-resources-ready.json'
        $agentStart = New-Object Diagnostics.ProcessStartInfo
        $agentStart.FileName = $nodeExecutable
        # ProcessStartInfo invokes Node directly, without a shell. These two
        # known absolute file paths cannot contain quotes or end in a slash.
        $agentStart.Arguments = '"{0}" --root "{1}"' -f $agentSidecarScript, $root
        $agentStart.WorkingDirectory = $root
        $agentStart.UseShellExecute = $false
        $agentStart.CreateNoWindow = $true
        $agentStart.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
        $agentStart.RedirectStandardOutput = $true
        $agentStart.RedirectStandardError = $true
        $agentSidecar = [Diagnostics.Process]::Start($agentStart)
        $exactAgentSidecarHandle = $agentSidecar.Handle
        $agentStdout = $agentSidecar.StandardOutput.ReadToEndAsync()
        $agentStderr = $agentSidecar.StandardError.ReadToEndAsync()
        # The original debug host has its own 90s request window. This leaves
        # time for identity checks and Quit; it is a fixture observation bound,
        # not a deadline added to any Agent or its background programs.
        $agentReadyDeadline = (Get-Item -LiteralPath $readyPath).LastWriteTimeUtc.AddSeconds(75)
        while (-not (Test-Path -LiteralPath $agentReadyPath)) {
            if ($hostProcess.HasExited) { throw 'The original Desktop exited before Agent resource readiness.' }
            if ($agentSidecar.HasExited) { throw "Agent resource sidecar exited before readiness ($($agentSidecar.ExitCode))." }
            if ([DateTime]::UtcNow -ge $agentReadyDeadline) { throw 'Agent resources did not become ready within the original debug fixture window.' }
            Start-Sleep -Milliseconds 25
        }
        $agentReceipt = [IO.File]::ReadAllText($agentReadyPath) | ConvertFrom-Json
        $resources = @($agentReceipt.processes)
        if ($agentReceipt.token -ne $token -or $agentReceipt.host_pid -ne $hostProcess.Id -or
            $agentReceipt.passed -ne $true -or [string]::IsNullOrWhiteSpace($agentReceipt.session_id) -or
            [string]::IsNullOrWhiteSpace($agentReceipt.operation_id) -or $resources.Count -ne 3 -or
            $agentReceipt.canonical_operation_id -ne $agentReceipt.operation_id -or
            $agentReceipt.original_launch_count -ne 2 -or $agentReceipt.original_mcp_call_count -ne 1 -or
            @(Compare-Object @('mcp','pipe','pty') @($resources.kind | Sort-Object -Unique)).Count -ne 0) {
            throw 'Agent resource readiness has the wrong exact owner, task or resource set.'
        }
        $seenAgentIds = @{}
        foreach ($resource in $resources) {
            [uint32]$resourceId = 0
            if (-not [uint32]::TryParse([string]$resource.pid, [ref]$resourceId) -or $resourceId -eq 0 -or
                $seenAgentIds.ContainsKey($resourceId) -or $nativeProcessIds -contains $resourceId -or
                @($hostProcess.Id,$agentSidecar.Id,$externalOwnerProcess.Id) -contains $resourceId) {
                throw 'Agent resource readiness contains a duplicate or unrelated process identity.'
            }
            $seenAgentIds[$resourceId] = $true
            $markerPath = Canonical-FixturePath ([string]$resource.marker_path)
            if (-not $markerPath.StartsWith($fixtureAbsolute, [StringComparison]::OrdinalIgnoreCase) -or
                -not (Test-Path -LiteralPath $markerPath -PathType Leaf)) {
                throw 'An Agent native marker is outside this exact disposable root.'
            }
            $markerInfo = Get-Item -LiteralPath $markerPath
            if ($markerInfo.Length -gt 4096) { throw 'An Agent native marker exceeds its fixture boundary.' }
            $markerParent = $markerInfo
            while ($markerParent -and $markerParent.FullName.StartsWith($fixtureAbsolute, [StringComparison]::OrdinalIgnoreCase)) {
                if (($markerParent.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'An Agent native marker traverses a reparse point.' }
                $markerParent = if ($markerParent -is [IO.FileInfo]) { $markerParent.Directory } else { $markerParent.Parent }
            }
            $marker = [IO.File]::ReadAllText($markerPath) | ConvertFrom-Json
            if ($marker.token -ne $token -or $marker.host_pid -ne $hostProcess.Id -or $marker.pid -ne $resourceId -or
                $marker.kind -ne $resource.kind) { throw 'An Agent native marker has the wrong original owner identity.' }
            $handle = [NomiLifecycleProcessWitness]::OpenProcess(0x00100000, $false, $resourceId)
            if ($handle -ne [IntPtr]::Zero) { $agentHandles += $handle }
            if ($handle -eq [IntPtr]::Zero -or [NomiLifecycleProcessWitness]::WaitForSingleObject($handle, 0) -ne 258) {
                throw "Exact Agent resource $resourceId was not observed alive before the exit action."
            }
            $native = Get-CimInstance Win32_Process -Filter ('ProcessId=' + $resourceId)
            $expectedImage = Canonical-FixturePath ([string]$resource.expected_image)
            if (-not $native -or [string]::IsNullOrWhiteSpace($native.ExecutablePath) -or
                -not (Test-Path -LiteralPath $expectedImage -PathType Leaf) -or
                -not [string]::Equals((Canonical-FixturePath $native.ExecutablePath), $expectedImage, [StringComparison]::OrdinalIgnoreCase) -or
                $native.CreationDate.ToUniversalTime() -lt $agentSidecar.StartTime.ToUniversalTime() -or
                $native.CreationDate.ToUniversalTime() -gt [DateTime]::UtcNow) {
                throw 'An Agent native image or creation-time witness does not match its exact receipt.'
            }
            Assert-OriginalHostAncestor $native $hostProcess
            if ([NomiLifecycleProcessWitness]::WaitForSingleObject($handle, 0) -ne 258) {
                throw 'An Agent native resource exited while its exact identity was being checked.'
            }
            $agentWitnesses += [pscustomobject]@{handle=$handle; pid=$resourceId; kind=$resource.kind;
                image=$expectedImage; created_at_utc=$native.CreationDate.ToUniversalTime().ToString('o'); parent_pid=$native.ParentProcessId;
                marker_path=$markerPath; process_id=$resource.process_id; call_id=$resource.call_id; mcp_request_id=$resource.mcp_request_id}
        }
        if ($agentSidecar.HasExited) { throw 'The original model sidecar ended before the Desktop exit action.' }
    }
    if ($StopAgentBeforeQuit) {
        $stopRequest = @{token=$token; host_pid=$hostProcess.Id; session_id=$agentReceipt.session_id; operation_id=$agentReceipt.operation_id}
        $stopRequestPath = Join-Path $root 'stop-agent.request'
        [IO.File]::WriteAllText(($stopRequestPath + '.tmp'), ($stopRequest | ConvertTo-Json -Compress))
        [IO.File]::Move(($stopRequestPath + '.tmp'), $stopRequestPath)
        $stopReadyPath = Join-Path $root 'stop-agent-ready.json'
        $stopReadyDeadline = (Get-Item -LiteralPath $readyPath).LastWriteTimeUtc.AddSeconds(75)
        while (-not (Test-Path -LiteralPath $stopReadyPath)) {
            if ($hostProcess.HasExited -or $agentSidecar.HasExited) { throw 'An original fixture owner exited before the exact Agent Stop proof.' }
            if ([DateTime]::UtcNow -ge $stopReadyDeadline) { throw 'The original Agent Stop did not prove Runtime retirement within the debug fixture window.' }
            Start-Sleep -Milliseconds 25
        }
        $agentStopReceipt = [IO.File]::ReadAllText($stopReadyPath) | ConvertFrom-Json
        if ($agentStopReceipt.passed -ne $true -or $agentStopReceipt.token -ne $token -or $agentStopReceipt.host_pid -ne $hostProcess.Id -or
            $agentStopReceipt.session_id -ne $agentReceipt.session_id -or $agentStopReceipt.operation_id -ne $agentReceipt.operation_id -or
            $agentStopReceipt.runtime_state -ne 'released' -or $agentStopReceipt.mcp_unknown_preserved -ne $true -or
            $agentStopReceipt.automatic_replay_authorized -ne $false -or @($agentStopReceipt.unknown_effects).Count -lt 1 -or
            @($agentStopReceipt.unknown_effects | Where-Object {$_.state -ne 'unknown'}).Count -ne 0 -or
            $agentStopReceipt.model_requests_before -ne @($agentReceipt.model_requests).Count -or
            $agentStopReceipt.model_requests_after -ne $agentStopReceipt.model_requests_before -or
            $agentStopReceipt.launch_count_before -ne 2 -or $agentStopReceipt.launch_count_after -ne 2 -or
            $agentStopReceipt.original_mcp_call_count -ne 1) {
            throw 'The exact Agent Stop did not preserve its original task, Unknown Effect or execution counts.'
        }
        $beforeTargets = @($agentStopReceipt.process_targets_before)
        $afterTargets = @($agentStopReceipt.process_targets_after)
        if ($beforeTargets.Count -ne 2 -or $afterTargets.Count -ne 2) { throw 'The original Session no longer holds exactly its two background processes.' }
        foreach ($witness in @($agentWitnesses | Where-Object {$_.kind -ne 'mcp'})) {
            $before = @($beforeTargets | Where-Object {$_.process_id -eq $witness.process_id})
            $after = @($afterTargets | Where-Object {$_.process_id -eq $witness.process_id})
            if ($before.Count -ne 1 -or $after.Count -ne 1 -or $before[0].kind -ne 'process_instance' -or
                $before[0].agent_session_id -ne $agentReceipt.session_id -or [string]::IsNullOrWhiteSpace($before[0].owner_generation)) {
                throw 'An original background process lost its exact resource target during Agent Stop.'
            }
            foreach ($field in @('kind','agent_session_id','owner_generation','process_id','creator_turn_id','creator_call_id')) {
                if ($before[0].$field -ne $after[0].$field) { throw 'Agent Stop changed an original process owner, identity or creator.' }
            }
            if ([NomiLifecycleProcessWitness]::WaitForSingleObject($witness.handle, 0) -ne 258) {
                throw 'Agent Stop killed a legitimate original Session background process.'
            }
        }
        $mcpWitness = @($agentWitnesses | Where-Object {$_.kind -eq 'mcp'})[0]
        $mcpWait = [NomiLifecycleProcessWitness]::WaitForSingleObject($mcpWitness.handle, 0)
        if ($mcpWait -ne 0 -and $mcpWait -ne 258) { throw 'The original MCP process observation failed after Agent Stop.' }
        # Closing the original request need not kill a shared MCP connection.
        # Whichever physical state is observed, Quit must join the same handle.
        $agentStopObservation = @{runtime_state='released'; pipe_and_pty_original_handles_alive=$true;
            mcp_process_alive=($mcpWait -eq 258); mcp_process_already_exited=($mcpWait -eq 0);
            model_requests=$agentStopReceipt.model_requests_after; original_launch_count=2; original_mcp_call_count=1}
        [IO.File]::WriteAllText((Join-Path $root 'agent-stop-native-observation.json'), ($agentStopObservation | ConvertTo-Json -Compress))
    }
    $externalOwnerAliveBeforeExit = [NomiLifecycleProcessWitness]::WaitForSingleObject($externalOwnerHandle, 0) -eq 258
    if (-not $externalOwnerAliveBeforeExit) { throw 'The exact independent owner was lost before the Desktop exit action.' }
    if ($Mode -eq 'quit') {
        [IO.File]::WriteAllText((Join-Path $root 'quit.request'), $token)
    } else {
        # Kill only the newly launched host Process object. Never Kill(true),
        # taskkill /T, process-name scanning, or killing a browser to pass.
        $hostProcess.Kill()
    }
    if (-not $hostProcess.WaitForExit(90000)) { throw 'The exact isolated host did not exit.' }
    $nativeExitDeadline = [DateTime]::UtcNow.AddSeconds(15)
    for ($witnessIndex = 0; $witnessIndex -lt $browserHandles.Count; $witnessIndex++) {
        $handle = $browserHandles[$witnessIndex]
        $remaining = [uint32][Math]::Max(0, ($nativeExitDeadline - [DateTime]::UtcNow).TotalMilliseconds)
        if ([NomiLifecycleProcessWitness]::WaitForSingleObject($handle, $remaining) -ne 0) {
            throw "Original Environment ProcessInfo $($nativeProcessIds[$witnessIndex]) is still alive; parent-death/quit is not proven."
        }
    }
    foreach ($witness in $agentWitnesses) {
        $remaining = [uint32][Math]::Max(0, ($nativeExitDeadline - [DateTime]::UtcNow).TotalMilliseconds)
        if ([NomiLifecycleProcessWitness]::WaitForSingleObject($witness.handle, $remaining) -ne 0) {
            throw "Original Agent $($witness.kind) process $($witness.pid) is still alive; exact native cleanup is not proven."
        }
    }
    $externalOwnerAliveAfterExit = [NomiLifecycleProcessWitness]::WaitForSingleObject($externalOwnerHandle, 0) -eq 258
    if (-not $externalOwnerAliveAfterExit) { throw 'Desktop retirement killed the independent driver-owned process; shared-owner protection is not proven.' }
    if (-not $stdout.Wait(5000) -or -not $stderr.Wait(5000)) { throw 'Isolated child output handles did not close after its native process exit.' }
    [IO.File]::WriteAllText((Join-Path $root 'stdout.log'), $stdout.GetAwaiter().GetResult())
    [IO.File]::WriteAllText((Join-Path $root 'stderr.log'), $stderr.GetAwaiter().GetResult())
    $coordinatorVerified = $false
    if ($Mode -eq 'quit') {
        $terminalPath = Join-Path $root 'terminal.json'
        if (-not (Test-Path -LiteralPath $terminalPath)) { throw 'The real ExitCoordinator terminal receipt is missing.' }
        $terminal = [IO.File]::ReadAllText($terminalPath) | ConvertFrom-Json
        $coordinatorVerified = $terminal.token -eq $token -and $terminal.pass -eq $true -and $terminal.coordinator_cleanup_verified -eq $true
        if ($hostProcess.ExitCode -ne 0 -or -not $coordinatorVerified) { throw 'The real Desktop Quit cleanup was not verified.' }
    }
    $connection = New-Object Net.Sockets.TcpClient
    try {
        $connect = $connection.ConnectAsync('127.0.0.1', [int]$ready.backend_port)
        try { [void]$connect.Wait(500) } catch { }
        if ($connection.Connected) { throw 'The original backend listener remains available after host exit.' }
    } finally { $connection.Dispose() }
    if ([IO.File]::ReadAllText((Join-Path $root 'fixture-sentinel')) -ne $token -or -not (Test-Path -LiteralPath $data -PathType Container)) {
        throw 'Native retirement unexpectedly deleted persistent fixture data.'
    }
    if ([NomiLifecycleProcessWitness]::WaitForSingleObject($externalOwnerHandle, 0) -ne 258) { throw 'The independent owner did not survive the complete Desktop shutdown observation.' }
    if ($WithAgentResources -and $agentSidecar.HasExited) { throw 'The model sidecar ended before the complete native shutdown observation.' }
    $passed = $true
    $passReceipt = (@{mode=$Mode; desktop_binary=$executableIdentity; driver=$driverIdentity; host_pid=$ready.host_pid; browser_pid=$ready.browser_pid; browser_pids=@($ready.browser_pids);
        native_process_pids=@($nativeProcessIds)+@($agentWitnesses | ForEach-Object {$_.pid}); native_environments=$environments;
        exact_native_inventory_exited=$true; native_witness_count=$browserHandles.Count+$agentHandles.Count; webview_witness_count=$browserHandles.Count;
        agent_resources=@{enabled=[bool]$WithAgentResources; native_witness_count=$agentHandles.Count;
            sidecar_identity=$sidecarIdentity;
            sidecar_pid=if($agentSidecar){$agentSidecar.Id}else{$null};
            session_id=$agentReceipt.session_id; operation_id=$agentReceipt.operation_id;
            original_launch_count=$agentReceipt.original_launch_count; original_mcp_call_count=$agentReceipt.original_mcp_call_count;
            processes=@($agentWitnesses | Select-Object pid,kind,image,created_at_utc,parent_pid,marker_path,process_id,call_id,mcp_request_id)};
        agent_stop_before_quit=@{enabled=[bool]$StopAgentBeforeQuit; observation=$agentStopObservation; receipt=$agentStopReceipt};
        exact_browser_exited=$true; coordinator_cleanup_verified=$coordinatorVerified; exit_code=$hostProcess.ExitCode;
        external_owner=@{pid=$externalOwnerProcess.Id; driver_owned_sentinel=$true; alive_before_exit=$externalOwnerAliveBeforeExit;
            alive_after_native_exit=$externalOwnerAliveAfterExit; registered_os_service=$false};
        scope=if($WithAgentResources){'exact isolated shell/managed WebView2 plus real Agent pipe, ConPTY and in-flight stdio MCP; independent driver-owned sentinel is not an OS service'}
            else{'exact isolated shell and managed WebView2 Environment ProcessInfos, real desktop host, and independent driver-owned sentinel (not an OS service)'};
        artifacts=$root} | ConvertTo-Json -Depth 8 -Compress)
    [IO.File]::WriteAllText((Join-Path $root 'native-exit-proof.json'), $passReceipt)
} finally {
    try {
        if ($hostProcess -and -not $hostProcess.HasExited) {
            # Failed fixtures may stop their own exact host; this is never a
            # pass, and native resource handles remain observation-only.
            $hostProcess.Kill()
            [void]$hostProcess.WaitForExit(10000)
        }
    } finally {
        if ($WithAgentResources) {
            $credentialPath = [IO.Path]::GetFullPath((Join-Path $root 'agent-resources-owner.jwt'))
            $exactRoot = [IO.Path]::GetFullPath($root).TrimEnd('\')
            if (-not [string]::Equals([IO.Path]::GetDirectoryName($credentialPath), $exactRoot, [StringComparison]::OrdinalIgnoreCase) -or
                [IO.File]::ReadAllText((Join-Path $root 'fixture-sentinel')) -ne $token) {
                throw 'Refusing to remove a credential outside the exact private fixture root.'
            }
            $credentialExisted = Test-Path -LiteralPath $credentialPath
            if ($credentialExisted) {
                if ((Get-Item -LiteralPath $credentialPath).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'The private fixture credential became a reparse point.' }
                Remove-Item -LiteralPath $credentialPath -Force
            }
            if (Test-Path -LiteralPath $credentialPath) { throw 'The private fixture login credential was not removed.' }
            [IO.File]::WriteAllText((Join-Path $root 'agent-credential-cleanup.json'),
                (@{token=$token; credential_existed=$credentialExisted; credential_present_after_cleanup=$false} | ConvertTo-Json -Compress))
        }
    }
    if ($stdout -and $stdout.IsCompleted) {
        try { [IO.File]::WriteAllText((Join-Path $root 'stdout.log'), $stdout.GetAwaiter().GetResult()) } catch { }
    }
    if ($stderr -and $stderr.IsCompleted) {
        try { [IO.File]::WriteAllText((Join-Path $root 'stderr.log'), $stderr.GetAwaiter().GetResult()) } catch { }
    }
    if ($agentSidecar) {
        try {
            # This exact Node process is the test's model server, outside the
            # Desktop Job. Its native Agent/MCP resources were owned by the
            # Desktop above; never kill this sidecar's tree to manufacture exit.
            if (-not $agentSidecar.HasExited) { $agentSidecar.Kill() }
            if (-not $agentSidecar.WaitForExit(10000)) { throw 'The exact Agent test sidecar did not stop during final cleanup.' }
            if ($agentStdout -and $agentStdout.Wait(5000)) {
                [IO.File]::WriteAllText((Join-Path $root 'agent-sidecar.stdout.log'), $agentStdout.GetAwaiter().GetResult())
            }
            if ($agentStderr -and $agentStderr.Wait(5000)) {
                [IO.File]::WriteAllText((Join-Path $root 'agent-sidecar.stderr.log'), $agentStderr.GetAwaiter().GetResult())
            }
        } finally { $agentSidecar.Dispose() }
    }
    foreach ($handle in $browserHandles) { [void][NomiLifecycleProcessWitness]::CloseHandle($handle) }
    foreach ($handle in $agentHandles) { [void][NomiLifecycleProcessWitness]::CloseHandle($handle) }
    if ($hostProcess) { $hostProcess.Dispose() }
    if ($externalOwnerProcess) {
        try {
            # Stop only the one independent helper created above. Never use a
            # process name, PID lookup, child-tree kill, or native browser kill.
            if (-not $externalOwnerProcess.HasExited) { $externalOwnerProcess.Kill() }
            if (-not $externalOwnerProcess.WaitForExit(10000)) { throw 'The exact driver-owned sentinel did not stop during final cleanup.' }
        } finally {
            if ($externalOwnerHandle -ne [IntPtr]::Zero) { [void][NomiLifecycleProcessWitness]::CloseHandle($externalOwnerHandle) }
            $externalOwnerProcess.Dispose()
        }
    }
    if ($passed -and -not $KeepArtifacts) {
        $absolute = [IO.Path]::GetFullPath($root).TrimEnd('\')
        if ([IO.Path]::GetDirectoryName($absolute) -ne $temporaryParent -or [IO.Path]::GetFileName($absolute) -ne "nomifun-lifecycle-$token" -or
            [IO.File]::ReadAllText((Join-Path $absolute 'fixture-sentinel')) -ne $token) { throw 'Refusing to delete a fixture directory outside the exact disposable root.' }
        Remove-Item -LiteralPath $absolute -Recurse -Force
    } else {
        Write-Output "LIFECYCLE_FIXTURE_ARTIFACTS $root"
    }
}
if ($passed) { Write-Output "LIFECYCLE_DESKTOP_PASS $passReceipt" }
