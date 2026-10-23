param([string]$Executable='target/desktop/leetdesk.exe', [string]$CaseName='')
$ErrorActionPreference='Stop'
$taskWorkspace=(Get-Location).Path
$taskBinary=(Resolve-Path -LiteralPath $Executable).Path
$taskCases=@(
    @{name='reading-first-native';theme='beige';mode='study';english=$false;width=1488;height=1057},
    @{name='reading-narrow-native';theme='beige';mode='study';english=$false;width=1120;height=800},
    @{name='reading-white-english-native';theme='white';mode='study';english=$true;width=1488;height=1057},
    @{name='coding-dark-english-native';theme='dark';mode='coding';english=$true;width=1488;height=1057}
)
if ($CaseName) { $taskCases=@($taskCases | Where-Object { $_.name -eq $CaseName }) }
foreach ($taskCase in $taskCases) {
    $taskRoot=Join-Path $taskWorkspace ('target/review-'+$taskCase.name+'-'+[guid]::NewGuid().ToString('N'))
    $taskArguments=@('scripts/design_fixture.py',$taskRoot)
    if ($taskCase.english) {$taskArguments+='--english'}
    & python @taskArguments | Out-Null
    if ($LASTEXITCODE -ne 0) {throw 'Fixture creation failed'}
    foreach ($taskEntry in @(@{file='appearance';value=$taskCase.theme},@{file='workspace-layout';value=$taskCase.mode})) {
        [IO.File]::WriteAllText((Join-Path $taskRoot ($taskEntry.file+'.json')),('"'+$taskEntry.value+'"'))
    }
    $taskOutput=Join-Path $taskWorkspace ('design/'+$taskCase.name+'.png')
    $taskProcess=Start-Process -FilePath $taskBinary -WorkingDirectory $taskWorkspace -ArgumentList @('--design-capture',$taskOutput,'--data-dir',$taskRoot,'--width',$taskCase.width,'--height',$taskCase.height) -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $taskRoot 'capture.log') -RedirectStandardError (Join-Path $taskRoot 'capture-error.log')
    if (-not $taskProcess.WaitForExit(30000)) {throw "Capture timed out: $taskRoot"}
    $taskProcess.Refresh()
    if ($taskProcess.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $taskOutput)) {throw "Capture failed: $taskRoot"}
    Get-Content -LiteralPath (Join-Path $taskRoot 'capture.log')
}
