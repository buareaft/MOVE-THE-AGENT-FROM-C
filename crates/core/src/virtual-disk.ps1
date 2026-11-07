# Embedded in the Rust core. JSON is base64 data, never executable path text.
$ErrorActionPreference='Stop'
$s=$request.settings
$action=$request.action
$image=[string]$s.imagePath
$letter=[string]$s.driveLetter
$id=[string]$s.diskId
$marker=$image+'.move-your-agent.json'
$taskName='MoveYourAgent-Mount-'+$id
function NoLinks([string]$p) {
    for($current=$p;$current;$current=Split-Path -Parent $current){
        if(Test-Path -LiteralPath $current){if((Get-Item -LiteralPath $current -Force).Attributes-band [IO.FileAttributes]::ReparsePoint){throw 'A storage path is a reparse point.'}}
    }
}
function SaveMarker { $m | ConvertTo-Json -Compress | Set-Content -LiteralPath $marker -Encoding UTF8 }
function PrivateAcl([string]$path,[bool]$directory) {
    if($directory){$acl=New-Object Security.AccessControl.DirectorySecurity}else{$acl=New-Object Security.AccessControl.FileSecurity}
    $acl.SetAccessRuleProtection($true,$false)
    foreach($value in @($request.owner,'S-1-5-18','S-1-5-32-544')){
        $sid=New-Object Security.Principal.SecurityIdentifier($value)
        if($directory){$rule=New-Object Security.AccessControl.FileSystemAccessRule($sid,'FullControl','ContainerInherit,ObjectInherit','None','Allow')}
        else{$rule=New-Object Security.AccessControl.FileSystemAccessRule($sid,'FullControl','Allow')}
        $acl.AddAccessRule($rule)
    }
    $acl.SetOwner((New-Object Security.Principal.SecurityIdentifier($request.owner)))
    Set-Acl -LiteralPath $path -AclObject $acl
}
function Reply($status) {
    if($request.result){@{ok=$true;status=$status}|ConvertTo-Json -Compress|Set-Content -LiteralPath $request.result -Encoding UTF8}
}
function EncodedRequest {
    $argsNow=[Environment]::GetCommandLineArgs();$index=[Array]::IndexOf($argsNow,'-EncodedCommand')
    if($index-lt 0){throw 'Missing encoded helper.'}
    $source=[Text.Encoding]::Unicode.GetString([Convert]::FromBase64String($argsNow[$index+1]))
    $body=$source.Substring($source.IndexOf("`n")+1)
    $payload=[Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes(($request|ConvertTo-Json -Compress)))
    $command="`$request = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('$payload')) | ConvertFrom-Json`n"+$body
    return [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command))
}
try {
    if($action-notin @('status','create','mount','detach','enable-auto','disable-auto')){throw 'Invalid action.'}
    if($letter-notmatch '^[D-Z]$' -or $image-notmatch '^[A-Za-z]:\\.+\.vhdx$' -or $image-match '["\r\n]' -or $id-notmatch '^[a-f0-9-]{36}$'){throw 'Invalid disk plan.'}
    NoLinks $image;NoLinks $marker
    $isAdmin=([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    $currentSid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    if(-not $request.owner){$request|Add-Member -NotePropertyName owner -NotePropertyValue $currentSid}
    if($action-ne 'status' -and $currentSid-ne $request.owner){throw 'Administrator elevation must use the same Windows account.'}
    if($action-ne 'status' -and -not $isAdmin){
        $child=Start-Process -FilePath "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" -ArgumentList @('-NoProfile','-NonInteractive','-EncodedCommand',(EncodedRequest)) -Verb RunAs -WindowStyle Hidden -PassThru -Wait
        exit $child.ExitCode
    }
    $exists=Test-Path -LiteralPath $image
    $m=$null
    if(Test-Path -LiteralPath $marker){
        $m=Get-Content -LiteralPath $marker -Raw -Encoding UTF8|ConvertFrom-Json
        if($m.id-ne $id -or $m.image-ne $image){throw 'Image ownership mismatch.'}
    }
    if($exists -and -not $m){throw 'Existing image is not owned by this application.'}
    if($action-eq 'create'){
        $backing=Get-Volume -DriveLetter $image.Substring(0,1)
        if($backing.FileSystem-ne 'NTFS' -or $backing.DriveType-ne 'Fixed'){throw 'Backing image requires a local fixed NTFS volume.'}
        if($backing.SizeRemaining-lt 512MB){throw 'At least 512 MB free backing space is required.'}
        if(-not $exists){
            if($m -and $m.phase-ne 'reserved'){throw 'Image is missing; ownership record retained.'}
            if(Get-Volume -DriveLetter $letter -ErrorAction SilentlyContinue){throw 'Requested drive letter is occupied.'}
            [IO.Directory]::CreateDirectory((Split-Path -Parent $image))|Out-Null
            NoLinks $image
            $newReservation=-not $m
            if($newReservation){$m=[pscustomobject]@{id=$id;image=$image;disk='';partition='';volume='';phase='reserved';taskUser=''}}
            # Reserve exclusively. An existing sidecar is never overwritten.
            if($newReservation){$file=[IO.File]::Open($marker,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None);$file.Dispose();PrivateAcl $marker $false;SaveMarker}
            Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices;
public static class MYADisk {
 [StructLayout(LayoutKind.Sequential)] public struct Storage {public uint Device; public Guid Vendor;}
 [StructLayout(LayoutKind.Explicit,Size=160)] public struct Params {
  [FieldOffset(0)] public uint Version; [FieldOffset(8)] public Guid Id;
  [FieldOffset(24)] public ulong Size; [FieldOffset(32)] public uint Block;
  [FieldOffset(36)] public uint Sector; [FieldOffset(40)] public uint Physical;
 }
 [DllImport("virtdisk.dll",CharSet=CharSet.Unicode)] static extern uint CreateVirtualDisk(ref Storage t,string path,uint access,IntPtr sd,uint flags,uint provider,ref Params p,IntPtr ov,out IntPtr h);
 [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
 public static void Create(string path,ulong size,Guid id){
  var t=new Storage{Device=3,Vendor=new Guid("EC984AEC-A0F9-47E9-901F-71415A66345B")};
  var p=new Params{Version=2,Id=id,Size=size,Sector=512,Physical=4096};IntPtr h;
  uint e=CreateVirtualDisk(ref t,path,0,IntPtr.Zero,0,0,ref p,IntPtr.Zero,out h);
  if(e!=0)throw new System.ComponentModel.Win32Exception((int)e);CloseHandle(h);
 }
}
'@
            [MYADisk]::Create($image,([uint64]$s.capacityGb*1GB),[Guid]$id)
            PrivateAcl $image $false
            $exists=$true
            $m.phase='created';SaveMarker
        }
    }
    if(-not $exists){Reply @{state=$(if($m){'incomplete'}else{'absent'});message=$(if($m){'创建未完成，可重试'}else{'尚未创建'});autoMount=$false};exit}
    if($action-in @('create','mount','detach','enable-auto','disable-auto')){
        $di=Get-DiskImage -ImagePath $image
        if($action-ne 'detach' -and -not $di.Attached){Mount-DiskImage -ImagePath $image -NoDriveLetter|Out-Null}
        $di=Get-DiskImage -ImagePath $image
        if($di.Attached){
            $disk=$di|Get-Disk
            if($disk.IsBoot -or $disk.IsSystem){throw 'System disk rejected.'}
            if(-not $m.disk){
                if($action-ne 'create' -or $m.phase-ne 'created' -or $disk.PartitionStyle-ne 'RAW'){throw 'Unverified disk; no formatting allowed.'}
                $disk|Initialize-Disk -PartitionStyle GPT|Out-Null
                $disk=$di|Get-Disk;$m.disk=[string]$disk.UniqueId;$m.phase='initialized';SaveMarker
            }
            if([string]$disk.UniqueId-ne $m.disk){throw 'Disk identity mismatch.'}
            $parts=@($disk|Get-Partition|Where-Object Type -ne 'Reserved')
            if(-not $m.partition){
                if($action-ne 'create' -or $m.phase-ne 'initialized' -or $parts.Count-ne 0){throw 'Unexpected partitions; refusing.'}
                $part=$disk|New-Partition -UseMaximumSize
                $m.partition=[string]$part.Guid;$m.phase='partitioned';SaveMarker
            }else{
                $part=$parts|Where-Object { [string]$_.Guid-eq $m.partition }
                if(-not $part -or $parts.Count-ne 1){throw 'Partition identity mismatch.'}
            }
            $vol=$part|Get-Volume
            if($m.phase-eq 'partitioned'){
                if($action-ne 'create'){throw 'Creation interrupted; use Create to resume.'}
                if($vol.FileSystemType-eq 'Unknown' -or -not $vol.FileSystem){$vol=$part|Format-Volume -FileSystem NTFS -NewFileSystemLabel 'MoveYourAgent' -Confirm:$false}
                if($vol.FileSystem-ne 'NTFS' -or $vol.FileSystemLabel-ne 'MoveYourAgent'){throw 'Unexpected filesystem; image preserved.'}
                $m.volume=[string]$vol.UniqueId;$m.phase='ready';SaveMarker
            }
            if($m.phase-ne 'ready' -or $vol.FileSystem-ne 'NTFS' -or [string]$vol.UniqueId-ne $m.volume){throw 'Volume identity mismatch.'}
            if($action-ne 'detach'){
                $occupied=Get-Volume -DriveLetter $letter -ErrorAction SilentlyContinue
                if($occupied -and [string]$occupied.UniqueId-ne $m.volume){throw 'Requested drive letter is occupied; no volume changed.'}
                if([string]$part.DriveLetter-ne $letter){$part|Set-Partition -NewDriveLetter $letter}
                $target=$letter+':\AgentData';NoLinks $target
                [IO.Directory]::CreateDirectory($target)|Out-Null
                if(-not $m.dataAcl){PrivateAcl $target $true;$m|Add-Member -NotePropertyName dataAcl -NotePropertyValue 1 -Force;SaveMarker}
                $proof=$letter+':\.move-your-agent-volume.json';NoLinks $proof
                if(Test-Path -LiteralPath $proof){if((Get-Content -LiteralPath $proof -Raw|ConvertFrom-Json).id-ne $id){throw 'Volume marker mismatch.'}}
                else {@{id=$id;volume=$m.volume}|ConvertTo-Json|Set-Content -LiteralPath $proof -Encoding UTF8}
            }
        }
        if($action-eq 'detach'){Dismount-DiskImage -ImagePath $image|Out-Null}
        if($action-eq 'enable-auto'){
            $savedResult=$request.result
            $existing=Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
            if($existing -and $existing.Description-ne ('Move Your Agent '+$id)){throw 'Unowned scheduled task.'}
            $sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
            # Store only the mount operation in the task. It cannot initialize or format.
            $request.action='mount';$request.result=$null
            $encoded=EncodedRequest
            $a=New-ScheduledTaskAction -Execute "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" -Argument ('-NoProfile -NonInteractive -WindowStyle Hidden -EncodedCommand '+$encoded)
            $t=New-ScheduledTaskTrigger -AtLogOn -User $sid
            $p=New-ScheduledTaskPrincipal -UserId $sid -LogonType Interactive -RunLevel Highest
            Register-ScheduledTask -TaskName $taskName -Action $a -Trigger $t -Principal $p -Description ('Move Your Agent '+$id) -Force|Out-Null
            $m.taskUser=$sid;SaveMarker
            $request.action='enable-auto';$request.result=$savedResult
        }
        if($action-eq 'disable-auto'){
            $task=Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
            if($task){if($task.Description-ne ('Move Your Agent '+$id)){throw 'Unowned scheduled task.'};Unregister-ScheduledTask -TaskName $taskName -Confirm:$false}
        }
    }
    $auto=$false
    try {$task=Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue;$auto=!!($task -and $task.Description-eq ('Move Your Agent '+$id) -and $task.State-ne 'Disabled')} catch {}
    $volume=Get-Volume -DriveLetter $letter -ErrorAction SilentlyContinue
    if($volume -and $m.volume -and [string]$volume.UniqueId-eq $m.volume){
        $proof=$letter+':\.move-your-agent-volume.json';NoLinks $proof
        if(-not(Test-Path -LiteralPath $proof) -or (Get-Content -LiteralPath $proof -Raw|ConvertFrom-Json).id-ne $id){throw 'Volume marker missing or mismatched.'}
        $backing=Get-Volume -DriveLetter $image.Substring(0,1)
        Reply @{state='mounted';message='已挂载';target=($letter+':\AgentData');size=[long]$volume.Size;free=[long]$volume.SizeRemaining;backingFree=[long]$backing.SizeRemaining;imageBytes=[long](Get-Item -LiteralPath $image).Length;autoMount=$auto}
    }else{Reply @{state= $(if($m.phase-eq 'ready'){'detached'}else{'incomplete'});message=$(if($m.phase-eq 'ready'){'未挂载'}else{'创建中断，镜像已保留'});imageBytes=[long](Get-Item -LiteralPath $image).Length;autoMount=$auto}}
} catch {
    if($request.result){@{ok=$false;error=$_.Exception.Message}|ConvertTo-Json -Compress|Set-Content -LiteralPath $request.result -Encoding UTF8}
    exit 1
}
