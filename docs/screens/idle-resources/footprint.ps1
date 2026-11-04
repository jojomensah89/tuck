param([Parameter(Mandatory)][int]$RootId,[int]$Seconds=180,[int]$IntervalSec=5,[Parameter(Mandatory)][string]$Output)
$previousCpu=@{}; $previousSwitch=@{}; $rows=@(); $watch=[Diagnostics.Stopwatch]::StartNew(); $lastTime=0
while ($watch.Elapsed.TotalSeconds -lt $Seconds) {
  $tree=Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId
  $ids=[Collections.Generic.HashSet[int]]::new(); [void]$ids.Add($RootId)
  do { $grew=$false; foreach($p in $tree) { if($ids.Contains([int]$p.ParentProcessId) -and $ids.Add([int]$p.ProcessId)){$grew=$true} } } while($grew)
  $family=@(Get-Process -Id @($ids) -ErrorAction SilentlyContinue)
  if(-not ($family.Id -contains $RootId)){throw 'Isolated root exited'}
  $cpu=0.0
  foreach($p in $family){ $value=$p.TotalProcessorTime.TotalMilliseconds; if($previousCpu.ContainsKey($p.Id)){$cpu += [Math]::Max(0,$value-$previousCpu[$p.Id])}; $previousCpu[$p.Id]=$value }
  $switches=0.0
  $threads=Get-CimInstance Win32_PerfRawData_PerfProc_Thread | Where-Object { $ids.Contains([int]$_.IDProcess) }
  foreach($t in $threads){$key="$($t.IDProcess):$($t.IDThread)"; $value=[double]$t.ContextSwitchesPersec; if($previousSwitch.ContainsKey($key)){$switches += [Math]::Max(0,$value-$previousSwitch[$key])}; $previousSwitch[$key]=$value}
  $now=$watch.Elapsed.TotalSeconds; $elapsed=$now-$lastTime; $lastTime=$now
  $rows += [pscustomobject]@{Seconds=$now;Processes=$family.Count;WorkingSetMB=(($family|Measure-Object WorkingSet64 -Sum).Sum/1MB);PrivateMB=(($family|Measure-Object PrivateMemorySize64 -Sum).Sum/1MB);CpuPctOfOneCore=($cpu/($elapsed*1000)*100);ContextSwitchesPerSecond=($switches/$elapsed)}
  Start-Sleep -Seconds $IntervalSec
}
$rows=@($rows | Select-Object -Skip 1)
function Median($values){$ordered=@($values|Sort-Object);$ordered[[int][Math]::Floor(($ordered.Count-1)/2)]}
$summary=[pscustomobject]@{RootId=$RootId;DurationSeconds=$watch.Elapsed.TotalSeconds;Samples=$rows.Count;WorkingSetMB=(Median $rows.WorkingSetMB);PrivateMB=(Median $rows.PrivateMB);CpuPctOfOneCore=(Median $rows.CpuPctOfOneCore);CpuMin=($rows.CpuPctOfOneCore|Measure-Object -Minimum).Minimum;CpuMax=($rows.CpuPctOfOneCore|Measure-Object -Maximum).Maximum;ContextSwitchesPerSecond=(Median $rows.ContextSwitchesPerSecond)}
@{Summary=$summary;Rows=$rows}|ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $Output
$summary | Format-List
