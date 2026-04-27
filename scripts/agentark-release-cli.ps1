param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$RemainingArgs
)

$ErrorActionPreference = "Stop"

$SourceDir = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$InstallDir = Split-Path $SourceDir -Parent
$ReleaseRepo = if ([string]::IsNullOrWhiteSpace($env:AGENTARK_RELEASE_REPO)) { "agentark-ai/AgentArk" } else { $env:AGENTARK_RELEASE_REPO.Trim() }
$RepoUrl = "https://github.com/$ReleaseRepo.git"
$ImageRepository = if ([string]::IsNullOrWhiteSpace($env:AGENTARK_IMAGE_REPOSITORY)) { "ghcr.io/agentark-ai/agentark" } else { $env:AGENTARK_IMAGE_REPOSITORY.Trim() }
$LocalSourceImage = "agentark:dev"
$UpdateCacheFile = Join-Path $InstallDir ".agentark-update-check.json"

function Invoke-AgentArkGitInInstall {
    param(
        [Parameter(Mandatory = $true)]
        [string[]]$Args
    )

    & docker run --rm -v "${InstallDir}:/work" -w /work alpine/git @Args
    if ($LASTEXITCODE -ne 0) {
        throw "AgentArk git helper failed."
    }
}

function Get-AgentArkLatestReleaseTag {
    $refs = & docker run --rm alpine/git ls-remote --tags --refs $RepoUrl "v*" 2>$null
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($refs)) {
        return $null
    }

    $tags = $refs |
        ForEach-Object {
            $parts = ($_ -split '\s+')
            if ($parts.Length -gt 1) { $parts[-1] -replace '^refs/tags/', '' } else { $null }
        } |
        Where-Object { $_ -match '^v\d+\.\d+\.\d+$' }

    if (-not $tags) {
        return $null
    }

    return $tags |
        Sort-Object { [version]($_.Substring(1)) } |
        Select-Object -Last 1
}

function Get-AgentArkReleaseVersionFromTag {
    param([string]$Tag)
    if ([string]::IsNullOrWhiteSpace($Tag)) {
        return ""
    }
    return $Tag.TrimStart("v", "V")
}

function Ensure-AgentArkScriptEnvFile {
    # Script-managed Compose variables live here. Do not create a root .env.
    $envPath = Join-Path $SourceDir ".agentark\local.env"
    $envDir = Split-Path $envPath
    if (-not (Test-Path $envDir)) {
        New-Item -ItemType Directory -Path $envDir -Force | Out-Null
    }
    if (-not (Test-Path $envPath)) {
        New-Item -ItemType File -Path $envPath -Force | Out-Null
    }
    return $envPath
}

function Set-AgentArkEnvValue {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Key,
        [Parameter(Mandatory = $true)]
        [string]$Value
    )

    $envPath = Ensure-AgentArkScriptEnvFile
    $lines = if (Test-Path $envPath) { [System.Collections.Generic.List[string]](Get-Content $envPath) } else { [System.Collections.Generic.List[string]]::new() }
    $updated = $false
    for ($i = 0; $i -lt $lines.Count; $i++) {
        if ($lines[$i] -like "$Key=*") {
            $lines[$i] = "$Key=$Value"
            $updated = $true
        }
    }
    if (-not $updated) {
        $lines.Add("$Key=$Value")
    }
    Set-Content -Path $envPath -Value $lines -Encoding ASCII
}

function Get-AgentArkEnvValue {
    param([Parameter(Mandatory = $true)][string]$Key)

    $envPath = Join-Path $SourceDir ".agentark\local.env"
    if (-not (Test-Path $envPath)) {
        return $null
    }
    foreach ($line in Get-Content $envPath) {
        if ($line -match ('^' + [regex]::Escape($Key) + '=(.*)$')) {
            return $matches[1].Trim()
        }
    }
    return $null
}

function Set-AgentArkPinnedRelease {
    param([Parameter(Mandatory = $true)][string]$Tag)

    $version = Get-AgentArkReleaseVersionFromTag $Tag
    Set-AgentArkEnvValue -Key "AGENTARK_IMAGE" -Value "${ImageRepository}:$version"
    Set-AgentArkEnvValue -Key "AGENTARK_RELEASE_REPO" -Value $ReleaseRepo
    Set-AgentArkEnvValue -Key "AGENTARK_RELEASE_TAG" -Value $Tag
    Set-AgentArkEnvValue -Key "AGENTARK_INSTALL_SOURCE" -Value "image"
}

function Set-AgentArkSourceBuildRelease {
    param([Parameter(Mandatory = $true)][string]$Tag)

    Set-AgentArkEnvValue -Key "AGENTARK_IMAGE" -Value $LocalSourceImage
    Set-AgentArkEnvValue -Key "AGENTARK_RELEASE_REPO" -Value $ReleaseRepo
    Set-AgentArkEnvValue -Key "AGENTARK_RELEASE_TAG" -Value $Tag
    Set-AgentArkEnvValue -Key "AGENTARK_INSTALL_SOURCE" -Value "source"
}

function Test-AgentArkSourceInstall {
    return (Get-AgentArkEnvValue -Key "AGENTARK_INSTALL_SOURCE") -eq "source"
}

function Get-AgentArkCurrentReleaseTag {
    $tag = Get-AgentArkEnvValue -Key "AGENTARK_RELEASE_TAG"
    if (-not [string]::IsNullOrWhiteSpace($tag)) {
        return $tag
    }

    try {
        $tag = & docker run --rm -v "${InstallDir}:/work" -w /work alpine/git git -C /work/source describe --tags --exact-match 2>$null
        if ($LASTEXITCODE -eq 0) {
            return ($tag | Select-Object -First 1).Trim()
        }
    } catch {}

    return $null
}

function Assert-AgentArkCleanCheckout {
    $status = & docker run --rm -v "${InstallDir}:/work" -w /work alpine/git git -C /work/source status --porcelain --untracked-files=no 2>$null
    if ($LASTEXITCODE -ne 0) {
        throw "Unable to inspect the AgentArk source checkout."
    }
    if (-not [string]::IsNullOrWhiteSpace(($status | Out-String))) {
        throw "Tracked local changes were found in $SourceDir. Resolve them before updating."
    }
}

function Update-AgentArkCheckoutToTag {
    param([Parameter(Mandatory = $true)][string]$Tag)

    $useSourceBuild = Test-AgentArkSourceInstall
    Assert-AgentArkCleanCheckout
    Invoke-AgentArkGitInInstall -Args @("git", "-C", "/work/source", "fetch", "--tags", "--force", "origin")
    Invoke-AgentArkGitInInstall -Args @("git", "-C", "/work/source", "checkout", "--force", $Tag)
    if ($useSourceBuild) {
        Set-AgentArkSourceBuildRelease -Tag $Tag
    } else {
        Set-AgentArkPinnedRelease -Tag $Tag
    }
}

function Get-AgentArkCachedLatestReleaseTag {
    $now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
    if (Test-Path $UpdateCacheFile) {
        try {
            $cache = Get-Content $UpdateCacheFile -Raw | ConvertFrom-Json
            if ($cache.checked_at -and $cache.latest_tag) {
                $age = $now - [int64]$cache.checked_at
                if ($age -lt 86400) {
                    return [string]$cache.latest_tag
                }
            }
        } catch {}
    }

    $latest = Get-AgentArkLatestReleaseTag
    if (-not [string]::IsNullOrWhiteSpace($latest)) {
        @{ checked_at = $now; latest_tag = $latest } | ConvertTo-Json | Set-Content -Path $UpdateCacheFile -Encoding ASCII
    }
    return $latest
}

function Show-AgentArkUpdateNotice {
    param([string]$CommandName)

    if ($CommandName -in @("help", "update", "uninstall")) {
        return
    }

    $current = Get-AgentArkCurrentReleaseTag
    $latest = Get-AgentArkCachedLatestReleaseTag
    if (-not [string]::IsNullOrWhiteSpace($current) -and -not [string]::IsNullOrWhiteSpace($latest) -and $current -ne $latest) {
        Write-Host "Update available: $current -> $latest. Run 'agentark update'." -ForegroundColor Yellow
    }
}

function Invoke-AgentArkStartScript {
    param([Parameter(Mandatory = $true)][string[]]$Args)

    Push-Location $SourceDir
    try {
        & "$SourceDir\scripts\start.bat" @Args
    } finally {
        Pop-Location
    }
}

function Show-AgentArkHelp {
    Write-Host "AgentArk CLI"
    Write-Host ""
    Write-Host "Usage: agentark <command>"
    Write-Host ""
    Write-Host "  chat       Interactive CLI chat with your agent"
    Write-Host "  pulse      Run ArkPulse health check"
    Write-Host "  start      Start AgentArk"
    Write-Host "  tunnel     Start with remote access"
    Write-Host "  stop       Stop AgentArk"
    Write-Host "  restart    Restart AgentArk"
    Write-Host "  logs       View live logs"
    Write-Host "  status     Show running containers"
    Write-Host "  update     Install the latest tagged release and restart"
    Write-Host "  setup      Run setup wizard"
    Write-Host "  uninstall  Stop and remove containers"
}

if (-not (Test-Path (Join-Path $SourceDir "docker-compose.yml"))) {
    throw "AgentArk source checkout is missing at $SourceDir."
}

$CommandName = if ($RemainingArgs.Count -gt 0 -and -not [string]::IsNullOrWhiteSpace($RemainingArgs[0])) { $RemainingArgs[0].ToLowerInvariant() } else { "help" }
Show-AgentArkUpdateNotice -CommandName $CommandName

switch ($CommandName) {
    "chat" {
        & docker exec -it agentark-control /app/agentark --chat
        break
    }
    "pulse" {
        Write-Host "Running ArkPulse health check..." -ForegroundColor Cyan
        & docker exec agentark-control /app/agentark --pulse
        break
    }
    "start" {
        Invoke-AgentArkStartScript -Args @("start")
        break
    }
    "tunnel" {
        $mode = if ($RemainingArgs.Count -gt 1) { $RemainingArgs[1] } else { "" }
        Invoke-AgentArkStartScript -Args @("tunnel", $mode)
        break
    }
    "stop" {
        Invoke-AgentArkStartScript -Args @("stop")
        break
    }
    "restart" {
        Invoke-AgentArkStartScript -Args @("restart")
        break
    }
    "logs" {
        Invoke-AgentArkStartScript -Args @("logs")
        break
    }
    "status" {
        Invoke-AgentArkStartScript -Args @("status")
        break
    }
    "update" {
        $targetTag = if ([string]::IsNullOrWhiteSpace($env:AGENTARK_RELEASE_TAG)) { Get-AgentArkLatestReleaseTag } else { $env:AGENTARK_RELEASE_TAG.Trim() }
        if ([string]::IsNullOrWhiteSpace($targetTag)) {
            throw "Unable to resolve the latest tagged AgentArk release."
        }
        Write-Host "Updating AgentArk to $targetTag..." -ForegroundColor Cyan
        Update-AgentArkCheckoutToTag -Tag $targetTag
        Invoke-AgentArkStartScript -Args @("update")
        break
    }
    "setup" {
        & docker exec -it agentark-control /app/agentark --setup
        break
    }
    "uninstall" {
        Write-Host "This will stop AgentArk and remove containers." -ForegroundColor Yellow
        Write-Host "Your data volumes and source checkout will be preserved." -ForegroundColor White
        $confirm = Read-Host "Continue? [y/N]"
        if ($confirm -eq "y" -or $confirm -eq "Y") {
            Push-Location $SourceDir
            try {
                & docker compose down
            } finally {
                Pop-Location
            }
            Write-Host "Removed. Data volumes kept. Source remains in $SourceDir." -ForegroundColor Green
        }
        break
    }
    default {
        Show-AgentArkHelp
        break
    }
}
