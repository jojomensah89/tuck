param([Parameter(Mandatory)][int]$ProcessId,[int]$Seconds=45,[string]$OutputPath)
$ErrorActionPreference='Stop'
$rootProcess=Get-Process -Id $ProcessId
if($rootProcess.Path -ne 'F:\tuck-agent-target\debug\tuck.exe'){throw 'Refusing a non-probe process'}
function Sample-Tree {
    $all=Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId
    $ids=[Collections.Generic.HashSet[int]]::new(); [void]$ids.Add($ProcessId)
    do { $added=$false; foreach($entry in $all){if($ids.Contains([int]$entry.ParentProcessId) -and $ids.Add([int]$entry.ProcessId)){$added=$true}} } while($added)
    $cpu=@{}; foreach($id in $ids){$p=Get-Process -Id $id -ErrorAction SilentlyContinue;if($p){$cpu[$id]=$p.TotalProcessorTime.TotalSeconds}}
    return $cpu
}
$before=Sample-Tree
$clock=[Diagnostics.Stopwatch]::StartNew()
Start-Sleep -Seconds $Seconds
$after=Sample-Tree
$elapsed=$clock.Elapsed.TotalSeconds
if(-not $after.ContainsKey($ProcessId)){throw 'Probe exited during measurement'}
$tree=0.0
foreach($id in $before.Keys){if($after.ContainsKey($id)){$tree += [Math]::Max([double]0,[double]($after[$id]-$before[$id]))}}
$main=$after[$ProcessId]-$before[$ProcessId]
$result=[ordered]@{processId=$ProcessId;elapsedSeconds=[Math]::Round($elapsed,3);mainCpuSeconds=$main;treeCpuSeconds=$tree;mainPercentOneCore=[Math]::Round(100*$main/$elapsed,4);treePercentOneCore=[Math]::Round(100*$tree/$elapsed,4);logicalProcessors=[Environment]::ProcessorCount;processesAtStart=$before.Count;processesAtEnd=$after.Count}
$json=$result|ConvertTo-Json
if($OutputPath){Set-Content -LiteralPath $OutputPath $json}
Write-Output $json
