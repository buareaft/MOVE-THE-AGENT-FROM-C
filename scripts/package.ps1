param([switch]$SkipBuild)
. "$PSScriptRoot\env.ps1"
if (-not $SkipBuild) {
    & "$PSScriptRoot\desktop.ps1" -Build
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}
& cargo build --release -p agent-space-cli
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$outputPath = Join-Path $projectRoot 'artifacts\AgentSpace-Lab'
New-Item -ItemType Directory -Force -Path $outputPath | Out-Null
Copy-Item -LiteralPath (Join-Path $projectRoot 'target\release\agent-space.exe') -Destination $outputPath
Copy-Item -LiteralPath (Join-Path $projectRoot 'target\release\agent-space-cli.exe') -Destination $outputPath
Copy-Item -LiteralPath (Join-Path $projectRoot '.agent-space-workspace') -Destination $outputPath
Copy-Item -LiteralPath (Join-Path $projectRoot 'docs\PORTABLE.txt') -Destination (Join-Path $outputPath 'README.txt')
Copy-Item -LiteralPath (Join-Path $projectRoot 'docs\THIRD-PARTY-NOTICES.txt') -Destination $outputPath
$portableFiles = @('agent-space.exe','agent-space-cli.exe','.agent-space-workspace','README.txt','THIRD-PARTY-NOTICES.txt') | ForEach-Object { Join-Path $outputPath $_ }
Compress-Archive -LiteralPath $portableFiles -DestinationPath (Join-Path $projectRoot 'artifacts\AgentSpace-Lab-win-x64.zip') -Force
$version=(Get-Content -LiteralPath (Join-Path $projectRoot 'package.json') -Raw | ConvertFrom-Json).version
Copy-Item -LiteralPath (Join-Path $projectRoot 'artifacts\AgentSpace-Lab-win-x64.zip') -Destination (Join-Path $projectRoot ("artifacts\AgentSpace-Windows-$version-x64.zip"))
Copy-Item -LiteralPath (Join-Path $projectRoot 'artifacts\AgentSpace-Lab-win-x64.zip') -Destination (Join-Path $projectRoot ("artifacts\MoveYourAgent-Windows-$version-x64.zip"))
Get-FileHash -LiteralPath (Join-Path $outputPath 'agent-space.exe') -Algorithm SHA256 | Format-List
Write-Output "Portable lab: $outputPath"
