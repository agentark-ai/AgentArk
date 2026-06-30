$ErrorActionPreference = "Stop"

$scriptPath = Join-Path $PSScriptRoot "install.ps1"
$scriptText = Get-Content -Raw -Path $scriptPath
$tokens = $null
$errors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseInput($scriptText, [ref]$tokens, [ref]$errors)
if ($errors.Count -gt 0) {
    throw "install.ps1 has parse errors: $($errors[0].Message)"
}

$dockerEngineFunctionAst = $ast.Find({
    param($node)
    $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
        $node.Name -eq "Test-AgentArkDockerEngine"
}, $true)
if (-not $dockerEngineFunctionAst) {
    throw "Test-AgentArkDockerEngine was not found"
}

$composeProjectFunctionAst = $ast.Find({
    param($node)
    $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
        $node.Name -eq "Get-AgentArkComposeProjectName"
}, $true)
if (-not $composeProjectFunctionAst) {
    throw "Get-AgentArkComposeProjectName was not found"
}

$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("agentark-install-test-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $tmp | Out-Null
$oldPath = $env:PATH
$oldComposeProjectName = $env:COMPOSE_PROJECT_NAME
try {
    Set-Content -Path (Join-Path $tmp "docker.cmd") -Encoding ASCII -Value @"
@echo off
if "%1"=="inspect" (
echo {^"com.docker.compose.project^":^"custom-agentark^"}
exit /b 0
)
if "%1"=="ps" (
echo failed to parse template: template: :1: function "com" not defined 1^>^&2
exit /b 1
)
echo fake docker daemon error 1>&2
exit /b 1
"@
    $env:PATH = "$tmp;$env:PATH"
    Remove-Item Env:\COMPOSE_PROJECT_NAME -ErrorAction SilentlyContinue
    Invoke-Expression $dockerEngineFunctionAst.Extent.Text
    Invoke-Expression $composeProjectFunctionAst.Extent.Text

    $result = Test-AgentArkDockerEngine
    if ($result -ne $false) {
        throw "Expected Test-AgentArkDockerEngine to return false for a failing docker daemon"
    }

    $projectName = Get-AgentArkComposeProjectName
    if ($projectName -ne "custom-agentark") {
        throw "Expected Get-AgentArkComposeProjectName to read the compose project label"
    }
} finally {
    $env:PATH = $oldPath
    if ($null -eq $oldComposeProjectName) {
        Remove-Item Env:\COMPOSE_PROJECT_NAME -ErrorAction SilentlyContinue
    } else {
        $env:COMPOSE_PROJECT_NAME = $oldComposeProjectName
    }
    Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host "install.ps1 regression tests passed"
