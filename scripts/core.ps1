param([ValidateSet('test','run','check')][string]$Action = 'test', [Parameter(ValueFromRemainingArguments=$true)][string[]]$CliArgs)
. "$PSScriptRoot\env.ps1"
if ($Action -eq 'test') { cargo test -p agent-space-core }
elseif ($Action -eq 'check') { cargo check --workspace }
else { cargo run -p agent-space-cli -- --fixtures @CliArgs }
exit $LASTEXITCODE
