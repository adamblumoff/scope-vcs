param(
    [Parameter(Position = 0)]
    [ValidateSet('doctor', 'up', 'down', 'status', 'reset')]
    [string]$Command = 'status'
)

$ErrorActionPreference = 'Stop'

$Root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$StateDir = Join-Path $Root '.tmp\local-dev'
$DataDir = Join-Path $Root '.scope\dev'
$ObjectDir = Join-Path $DataDir 'objects'
$KeyFile = Join-Path $DataDir 'object-key.txt'
$ApiPidFile = Join-Path $StateDir 'api.pid'
$WebPidFile = Join-Path $StateDir 'web.pid'
$ApiOutLog = Join-Path $StateDir 'api.out.log'
$ApiErrLog = Join-Path $StateDir 'api.err.log'
$WebOutLog = Join-Path $StateDir 'web.out.log'
$WebErrLog = Join-Path $StateDir 'web.err.log'

function Ensure-Dir([string]$Path) {
    New-Item -ItemType Directory -Force -Path $Path | Out-Null
}

function Quote-PS([string]$Value) {
    return "'" + ($Value -replace "'", "''") + "'"
}

function Read-DotEnv([string]$Path) {
    $values = @{}
    if (!(Test-Path -LiteralPath $Path)) {
        return $values
    }
    Get-Content -LiteralPath $Path | ForEach-Object {
        if ($_ -match '^\s*([^#][^=]+)=\s*(.*)$') {
            $name = $matches[1].Trim()
            $value = $matches[2].Trim().Trim('"').Trim("'")
            $values[$name] = $value
        }
    }
    return $values
}

function Get-ClerkIssuer {
    $webEnv = Read-DotEnv (Join-Path $Root 'web\.env.local')
    $publishableKey = $webEnv['VITE_CLERK_PUBLISHABLE_KEY']
    if (!$publishableKey) {
        throw 'web\.env.local must define VITE_CLERK_PUBLISHABLE_KEY'
    }
    if (!$publishableKey.StartsWith('pk_test_')) {
        throw 'local dev requires a Clerk development publishable key (pk_test_)'
    }
    $payload = $publishableKey -replace '^pk_test_', ''
    $payload = $payload.Replace('_', '/').Replace('-', '+')
    switch ($payload.Length % 4) {
        2 { $payload += '==' }
        3 { $payload += '=' }
    }
    $clerkHost = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($payload)).TrimEnd('$')
    if (!$clerkHost.EndsWith('.clerk.accounts.dev')) {
        throw "local dev Clerk issuer must end with .clerk.accounts.dev, got $clerkHost"
    }
    return "https://$clerkHost"
}

function Get-DevUser {
    $rootEnv = Read-DotEnv (Join-Path $Root '.env.local')
    $email = $rootEnv['SCOPE_DEV_USER_EMAIL']
    if (!$email) {
        throw 'root .env.local must define SCOPE_DEV_USER_EMAIL for seeded local repos'
    }
    $handle = $rootEnv['SCOPE_DEV_USER_HANDLE']
    return @{
        Email = $email
        Handle = $handle
    }
}

function Get-ObjectKey {
    Ensure-Dir $DataDir
    if (!(Test-Path -LiteralPath $KeyFile)) {
        $bytes = [byte[]]::new(32)
        $rng = [Security.Cryptography.RandomNumberGenerator]::Create()
        try {
            $rng.GetBytes($bytes)
        } finally {
            $rng.Dispose()
        }
        [Convert]::ToBase64String($bytes) | Set-Content -NoNewline -LiteralPath $KeyFile
    }
    return (Get-Content -Raw -LiteralPath $KeyFile).Trim()
}

function Get-PortOwner([int]$Port) {
    Get-NetTCPConnection -LocalPort $Port -ErrorAction SilentlyContinue |
        Where-Object State -eq Listen |
        Select-Object -First 1 -ExpandProperty OwningProcess
}

function Stop-ProcessTree([int]$ProcessId) {
    $children = Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
        Where-Object { $_.ParentProcessId -eq $ProcessId } |
        Select-Object -ExpandProperty ProcessId
    foreach ($child in $children) {
        Stop-ProcessTree ([int]$child)
    }
    Stop-Process -Id $ProcessId -Force -ErrorAction SilentlyContinue
}

function Stop-OwnedProcesses {
    foreach ($pidFile in @($ApiPidFile, $WebPidFile)) {
        if (Test-Path -LiteralPath $pidFile) {
            $pidValue = (Get-Content -Raw -LiteralPath $pidFile).Trim()
            if ($pidValue -match '^\d+$') {
                Stop-ProcessTree ([int]$pidValue)
            }
            Remove-Item -LiteralPath $pidFile -Force -ErrorAction SilentlyContinue
        }
    }
}

function Assert-Command([string]$Name) {
    if (!(Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "$Name is required for local dev"
    }
}

function Assert-PortFree([int]$Port, [string]$Name) {
    $owner = Get-PortOwner $Port
    if ($owner) {
        throw "$Name port $Port is already in use by PID $owner"
    }
}

function Invoke-Doctor {
    Assert-Command cargo
    Assert-Command git
    if (!(Get-Command pnpm.cmd -ErrorAction SilentlyContinue) -and !(Get-Command pnpm -ErrorAction SilentlyContinue)) {
        throw 'pnpm is required for local web dev'
    }
    $issuer = Get-ClerkIssuer
    $devUser = Get-DevUser
    $webEnv = Read-DotEnv (Join-Path $Root 'web\.env.local')
    if (!$webEnv['CLERK_SECRET_KEY'] -or !$webEnv['CLERK_SECRET_KEY'].StartsWith('sk_test_')) {
        throw 'web\.env.local must define a Clerk development secret key (sk_test_)'
    }
    Write-Host "doctor ok"
    Write-Host "clerk issuer: $issuer"
    Write-Host "seed user: $($devUser.Email)"
    Write-Host "api: http://localhost:8080"
    Write-Host "web: http://localhost:3000"
}

function Start-Api {
    $issuer = Get-ClerkIssuer
    $devUser = Get-DevUser
    $objectKey = Get-ObjectKey
    $cargo = (Get-Command cargo -ErrorAction Stop).Source
    $apiDir = Join-Path $Root 'api'
    $scriptPath = Join-Path $StateDir 'api.ps1'
    $devHandleLine = if ($devUser.Handle) {
        "`$env:SCOPE_DEV_USER_HANDLE = $(Quote-PS $devUser.Handle)"
    } else {
        "Remove-Item Env:\SCOPE_DEV_USER_HANDLE -ErrorAction SilentlyContinue"
    }
    $script = @"
`$ErrorActionPreference = 'Stop'
Set-Location $(Quote-PS $Root)
Get-ChildItem Env:RAILWAY_* -ErrorAction SilentlyContinue | Remove-Item
Remove-Item Env:\DATABASE_URL -ErrorAction SilentlyContinue
Get-ChildItem Env:SCOPE_BUCKET_* -ErrorAction SilentlyContinue | Remove-Item
`$env:SCOPE_ENV = 'local'
`$env:SCOPE_METADATA_STORE = 'memory'
`$env:SCOPE_OBJECT_STORE = 'filesystem'
`$env:SCOPE_DATA_DIR = $(Quote-PS $DataDir)
`$env:SCOPE_OBJECT_STORE_DIR = $(Quote-PS $ObjectDir)
`$env:SCOPE_OBJECT_ENCRYPTION_KEY = $(Quote-PS $objectKey)
`$env:PORT = '8080'
`$env:SCOPE_APP_ORIGIN = 'http://localhost:3000'
`$env:SCOPE_API_PUBLIC_URL = 'http://localhost:8080'
`$env:CLERK_ISSUER = $(Quote-PS $issuer)
`$env:CLERK_JWKS_URL = $(Quote-PS "$issuer/.well-known/jwks.json")
`$env:CLERK_AUTHORIZED_PARTIES = 'http://localhost:3000'
`$env:SCOPE_DEV_USER_EMAIL = $(Quote-PS $devUser.Email)
$devHandleLine
`$apiProcess = Start-Process -FilePath $(Quote-PS $cargo) -ArgumentList @('run', '--features', 'local-dev') -WorkingDirectory $(Quote-PS $apiDir) -RedirectStandardOutput $(Quote-PS $ApiOutLog) -RedirectStandardError $(Quote-PS $ApiErrLog) -PassThru -Wait
exit `$apiProcess.ExitCode
"@
    $script | Set-Content -LiteralPath $scriptPath
    $process = Start-Process -FilePath powershell.exe -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $scriptPath) -WorkingDirectory $Root -WindowStyle Hidden -PassThru
    $process.Id | Set-Content -NoNewline -LiteralPath $ApiPidFile
}

function Start-Web {
    $webDir = Join-Path $Root 'web'
    $pnpm = (Get-Command pnpm.cmd -ErrorAction SilentlyContinue)
    if (!$pnpm) {
        $pnpm = Get-Command pnpm -ErrorAction Stop
    }
    $scriptPath = Join-Path $StateDir 'web.ps1'
    $script = @"
`$ErrorActionPreference = 'Stop'
Set-Location $(Quote-PS $Root)
Get-ChildItem Env:RAILWAY_* -ErrorAction SilentlyContinue | Remove-Item
`$env:SCOPE_API_INTERNAL_URL = 'http://localhost:8080'
`$env:SCOPE_API_PUBLIC_URL = 'http://localhost:8080'
`$webProcess = Start-Process -FilePath $(Quote-PS $pnpm.Source) -ArgumentList @('dev') -WorkingDirectory $(Quote-PS $webDir) -RedirectStandardOutput $(Quote-PS $WebOutLog) -RedirectStandardError $(Quote-PS $WebErrLog) -PassThru -Wait
exit `$webProcess.ExitCode
"@
    $script | Set-Content -LiteralPath $scriptPath
    $process = Start-Process -FilePath powershell.exe -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $scriptPath) -WorkingDirectory $Root -WindowStyle Hidden -PassThru
    $process.Id | Set-Content -NoNewline -LiteralPath $WebPidFile
}

function Wait-Http([string]$Url, [int]$Seconds) {
    $deadline = (Get-Date).AddSeconds($Seconds)
    do {
        try {
            $response = Invoke-WebRequest -UseBasicParsing -Uri $Url -TimeoutSec 2
            if ($response.StatusCode -ge 200 -and $response.StatusCode -lt 500) {
                return $true
            }
        } catch {
            Start-Sleep -Milliseconds 500
        }
    } while ((Get-Date) -lt $deadline)
    return $false
}

function Invoke-Up {
    Invoke-Doctor | Out-Null
    Stop-OwnedProcesses
    Assert-PortFree 8080 'api'
    Assert-PortFree 3000 'web'
    Ensure-Dir $StateDir
    Ensure-Dir $DataDir
    Remove-Item -LiteralPath $ApiOutLog, $ApiErrLog, $WebOutLog, $WebErrLog -Force -ErrorAction SilentlyContinue
    Start-Api
    if (!(Wait-Http 'http://localhost:8080/readyz' 45)) {
        throw "api did not become ready; see $ApiErrLog"
    }
    Start-Web
    if (!(Wait-Http 'http://localhost:3000' 45)) {
        throw "web did not become ready; see $WebErrLog"
    }
    Invoke-Status
}

function Get-PidStatus([string]$Path) {
    if (!(Test-Path -LiteralPath $Path)) {
        return 'stopped'
    }
    $pidValue = (Get-Content -Raw -LiteralPath $Path).Trim()
    if ($pidValue -match '^\d+$' -and (Get-Process -Id ([int]$pidValue) -ErrorAction SilentlyContinue)) {
        return "running pid $pidValue"
    }
    return 'stale'
}

function Invoke-Status {
    Write-Host "api process: $(Get-PidStatus $ApiPidFile)"
    Write-Host "web process: $(Get-PidStatus $WebPidFile)"
    try {
        $ready = Invoke-WebRequest -UseBasicParsing -Uri 'http://localhost:8080/readyz' -TimeoutSec 3
        Write-Host "api readyz: $($ready.Content)"
    } catch {
        Write-Host "api readyz: unavailable"
    }
    try {
        $web = Invoke-WebRequest -UseBasicParsing -Uri 'http://localhost:3000' -TimeoutSec 3
        Write-Host "web: HTTP $($web.StatusCode)"
    } catch {
        Write-Host "web: unavailable"
    }
}

function Invoke-Reset {
    Stop-OwnedProcesses
    $resolvedData = [IO.Path]::GetFullPath($DataDir)
    $resolvedRoot = [IO.Path]::GetFullPath($Root)
    if (!$resolvedData.StartsWith($resolvedRoot)) {
        throw "refusing to remove data outside repo: $resolvedData"
    }
    Remove-Item -LiteralPath $DataDir -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $StateDir -Recurse -Force -ErrorAction SilentlyContinue
    Write-Host 'local dev state reset'
}

switch ($Command) {
    'doctor' { Invoke-Doctor }
    'up' { Invoke-Up }
    'down' { Stop-OwnedProcesses; Write-Host 'local dev stopped' }
    'status' { Invoke-Status }
    'reset' { Invoke-Reset }
}
