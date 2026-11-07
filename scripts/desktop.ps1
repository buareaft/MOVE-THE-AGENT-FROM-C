param([switch]$Build)
. "$PSScriptRoot\env.ps1"
if ($Build) { & npm.cmd exec tauri build -- --no-bundle } else { & npm.cmd exec tauri dev -- -- --fixtures }
exit $LASTEXITCODE
