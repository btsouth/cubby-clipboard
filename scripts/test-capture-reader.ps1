# Synthetic regression fixtures for the real production capture listener.
# Build capture_probe with --features dev-harness, then run in an isolated Windows
# interactive STA session. This replaces that session's clipboard.
# -HungOwner instead checks that a copy made while a live OLE owner is frozen is
# still captured within Windows' 30 s delayed-render limit.
param([Parameter(Mandatory=$true)][string]$ProbePath, [string]$OutputDirectory=(Join-Path $env:TEMP 'cubby-reader-regression'), [switch]$HungOwner)
$ErrorActionPreference='Stop'
$dir=[IO.Path]::GetFullPath($OutputDirectory)
[void](New-Item -ItemType Directory -Force $dir)
Add-Type -AssemblyName System.Windows.Forms,System.Drawing
Add-Type -ReferencedAssemblies System.Windows.Forms,System.Drawing -TypeDefinition @'
using System;
using System.IO;
using System.Text;
using System.Drawing;
using System.Drawing.Imaging;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Windows.Forms;
public static class CaptureFixtures {
 [DllImport("user32.dll")] static extern bool OpenClipboard(IntPtr h);
 [DllImport("user32.dll")] static extern bool CloseClipboard();
 [DllImport("user32.dll")] static extern bool EmptyClipboard();
 [DllImport("user32.dll")] static extern IntPtr SetClipboardData(uint f,IntPtr h);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern uint RegisterClipboardFormat(string s);
 [DllImport("kernel32.dll")] static extern IntPtr GlobalAlloc(uint flags,UIntPtr bytes);
 [DllImport("kernel32.dll")] static extern IntPtr GlobalLock(IntPtr h);
 [DllImport("kernel32.dll")] static extern bool GlobalUnlock(IntPtr h);
 [DllImport("kernel32.dll")] static extern IntPtr GlobalFree(IntPtr h);
 static Form owner=new Form();
 public static byte[] Unicode(string value){return Encoding.Unicode.GetBytes(value+"\0");}
 public static byte[] Utf8(string value){return Encoding.UTF8.GetBytes(value);}
 public static void Publish(uint[] formats,byte[][] buffers){
  if(!OpenClipboard(owner.Handle))throw new Exception("Fixture OpenClipboard failed");
  try {
   if(!EmptyClipboard())throw new Exception("Fixture EmptyClipboard failed");
   for(int i=0;i<formats.Length;i++){
    IntPtr h=GlobalAlloc(0x42,(UIntPtr)buffers[i].Length);
    if(h==IntPtr.Zero)throw new Exception("Fixture allocation failed");
    try {
     IntPtr p=GlobalLock(h);if(p==IntPtr.Zero)throw new Exception("Fixture GlobalLock failed");
     Marshal.Copy(buffers[i],0,p,buffers[i].Length);GlobalUnlock(h);
     if(SetClipboardData(formats[i],h)==IntPtr.Zero)throw new Exception("Fixture SetClipboardData failed");h=IntPtr.Zero;
    } finally {if(h!=IntPtr.Zero)GlobalFree(h);}
   }
  }finally{CloseClipboard();}
 }
 public static byte[] Png(int red){
  using(var bitmap=new Bitmap(2,2,PixelFormat.Format32bppArgb)){
   bitmap.SetPixel(0,0,Color.FromArgb(255,red,0,0));bitmap.SetPixel(1,0,Color.FromArgb(128,0,255,0));
   bitmap.SetPixel(0,1,Color.FromArgb(64,0,0,255));bitmap.SetPixel(1,1,Color.FromArgb(0,255,255,255));
   using(var s=new MemoryStream()){bitmap.Save(s,ImageFormat.Png);return s.ToArray();}
  }
 }
 public static byte[] DibV5(){
  var b=new byte[140]; Action<int,int> put=(offset,value)=>Array.Copy(BitConverter.GetBytes(value),0,b,offset,4);
  put(0,124);put(4,2);put(8,-2);b[12]=1;b[14]=32;put(16,3);put(20,16);
  put(40,0x00ff0000);put(44,0x0000ff00);put(48,0x000000ff);put(52,unchecked((int)0xff000000));put(56,0x73524742);
  Array.Copy(new byte[]{0,0,201,255,0,255,0,128,255,0,0,64,255,255,255,0},0,b,124,16);return b;
 }
 public static byte[] Dib32(){
  var b=new byte[56]; Action<int,int> put=(offset,value)=>Array.Copy(BitConverter.GetBytes(value),0,b,offset,4);
  put(0,40);put(4,2);put(8,2);b[12]=1;b[14]=32;put(20,16);
  for(int i=0;i<4;i++)put(40+4*i,unchecked((int)0xff3366cc));return b;
 }
 public static byte[] DropFiles(string path){
  var names=Encoding.Unicode.GetBytes(path+"\0\0");var b=new byte[20+names.Length];b[0]=20;b[16]=1;Array.Copy(names,0,b,20,names.Length);return b;
 }
 public static byte[] Html(string fragment){
  string body="<html><body><!--StartFragment-->"+fragment+"<!--EndFragment--></body></html>";
  string template="Version:0.9\r\nStartHTML:{0:D10}\r\nEndHTML:{1:D10}\r\nStartFragment:{2:D10}\r\nEndFragment:{3:D10}\r\n";
  int start=String.Format(template,0,0,0,0).Length;
  return Encoding.UTF8.GetBytes(String.Format(template,start,start+Encoding.UTF8.GetByteCount(body),start+Encoding.UTF8.GetByteCount("<html><body><!--StartFragment-->"),start+Encoding.UTF8.GetByteCount("<html><body><!--StartFragment-->")+Encoding.UTF8.GetByteCount(fragment))+body);
 }
}
'@
$rtf=[CaptureFixtures]::RegisterClipboardFormat('Rich Text Format')
$html=[CaptureFixtures]::RegisterClipboardFormat('HTML Format')
$png=[CaptureFixtures]::RegisterClipboardFormat('PNG')
$history=[CaptureFixtures]::RegisterClipboardFormat('CanIncludeInClipboardHistory')
$viewer=[CaptureFixtures]::RegisterClipboardFormat('Clipboard Viewer Ignore')
$exclude=[CaptureFixtures]::RegisterClipboardFormat('ExcludeClipboardContentFromMonitorProcessing')
$fixtures=New-Object System.Collections.Generic.List[object]
function Add-Fixture($formats,$buffers,$expected){if($formats.Count -eq 1){$buffers=,[byte[]]$buffers};if($formats.Count -ne $buffers.Count){throw "Fixture format/buffer count mismatch"};$fixtures.Add(@{formats=[uint32[]]$formats;buffers=[byte[][]]$buffers;expected=$expected})}
foreach($case in @(@('CUBBY-SENSITIVE-ZERO',$history,0,$true),@('CUBBY-SENSITIVE-ONE',$history,1,$false),@('CUBBY-SENSITIVE-VIEWER',$viewer,1,$true),@('CUBBY-SENSITIVE-EXCLUDE',$exclude,1,$true))){
 Add-Fixture @(13,$case[1]) @([CaptureFixtures]::Unicode($case[0]),[BitConverter]::GetBytes([int]$case[2])) @{text=$case[0];sensitive=$case[3]}
}
$rgba=@(255,0,0,255,0,255,0,128,0,0,255,64,255,255,255,0)
$pngBytes=[CaptureFixtures]::Png(255)
Add-Fixture @($png) @($pngBytes) @{image_size=@(2,2);image_rgba=$rgba}
$dibRgba=$rgba.Clone();$dibRgba[0]=201
Add-Fixture @(17) @([CaptureFixtures]::DibV5()) @{image_size=@(2,2);image_rgba=$dibRgba}
$wrapperRgba=$rgba.Clone();$wrapperRgba[0]=128
$hybridRgba=$rgba.Clone();$hybridRgba[0]=64
Add-Fixture @(13,$html,$rtf,$png) @([CaptureFixtures]::Unicode(''),[CaptureFixtures]::Html('<b>image wrapper</b>'),[CaptureFixtures]::Utf8('{\rtf1 image wrapper}'),[CaptureFixtures]::Png(128)) @{image_size=@(2,2);image_rgba=$wrapperRgba}
Add-Fixture @(13,15,$png) @([CaptureFixtures]::Unicode('C:\synthetic-image.png'),[CaptureFixtures]::DropFiles('C:\synthetic-image.png'),[CaptureFixtures]::Png(64)) @{image_size=@(2,2);image_rgba=$hybridRgba}
$htmlBody='<html><body><!--StartFragment--><b>HTMLONLY</b><!--EndFragment--></body></html>'
Add-Fixture @($html) @([CaptureFixtures]::Html('<b>HTMLONLY</b>')) @{text='HTMLONLY';html_exact=$htmlBody}
Add-Fixture @($rtf) @([CaptureFixtures]::Utf8('{\rtf1\ansi RTFONLY}')) @{text='RTFONLY';rtf_exact='{\rtf1\ansi RTFONLY}'}
Add-Fixture @(1) @([System.Text.Encoding]::ASCII.GetBytes("CUBBY-ANSI`0")) @{text='CUBBY-ANSI'}
# Live OLE sources answer the reader's GetData themselves, on this thread.
$oleText=New-Object System.Windows.Forms.DataObject
$oleText.SetData([System.Windows.Forms.DataFormats]::UnicodeText,$false,'CUBBY-OLE-LIVE-TEXT')
$fixtures.Add(@{ole=$oleText;expected=@{text='CUBBY-OLE-LIVE-TEXT'}})
$oleDib=New-Object System.Windows.Forms.DataObject
$oleDib.SetData([System.Windows.Forms.DataFormats]::Dib,$false,(New-Object IO.MemoryStream(,[CaptureFixtures]::Dib32())))
# Dib32 stores 0xff3366cc per pixel in BGRA order.
$fixtures.Add(@{ole=$oleDib;expected=@{image_size=@(2,2);image_rgba=@(51,102,204,255)*4}})
function Wait-Pumping([int]$milliseconds){$until=[DateTime]::UtcNow.AddMilliseconds($milliseconds);while([DateTime]::UtcNow -lt $until){[System.Windows.Forms.Application]::DoEvents();Start-Sleep -Milliseconds 5}}
function Start-Probe([string]$name,$expected,[int]$seconds){
 [IO.File]::WriteAllText("$dir\$name-manifest.json",(ConvertTo-Json -InputObject @($expected) -Depth 5),[Text.UTF8Encoding]::new($false))
 Remove-Item "$dir\$name-captures.jsonl" -ErrorAction SilentlyContinue
 $probe=Start-Process $ProbePath -ArgumentList "`"$dir\$name-manifest.json`" `"$dir\$name-captures.jsonl`" $seconds" -PassThru -WindowStyle Hidden -RedirectStandardError "$dir\$name-probe-error.txt"
 $null=$probe.Handle
 $limit=[DateTime]::UtcNow.AddSeconds(8)
 while(!(Test-Path "$dir\$name-captures.jsonl") -or !(Get-Content "$dir\$name-captures.jsonl" | Select-String 'ready')){if($probe.HasExited -or [DateTime]::UtcNow -gt $limit){throw 'Probe startup failed'};Start-Sleep -Milliseconds 100}
 $probe
}
if($HungOwner){
 $probe=Start-Probe 'hung-owner' @{text='CUBBY-AFTER-HUNG-OWNER'} 75
 # Thread.Sleep does not pump, so this owner cannot answer GetData until it wakes.
 $hung="Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.Clipboard]::SetDataObject((New-Object System.Windows.Forms.DataObject([System.Windows.Forms.DataFormats]::UnicodeText,'CUBBY-HUNG-OWNER')),`$false); [System.Threading.Thread]::Sleep(55000)"
 $owner=Start-Process powershell.exe -ArgumentList '-NoProfile','-STA','-Command',$hung -PassThru -WindowStyle Hidden
 $start=[DateTime]::UtcNow
 Start-Sleep -Seconds 4
 # EmptyClipboard waits about 5 s for the frozen owner before this copy lands.
 [CaptureFixtures]::Publish(@(13),@(,[CaptureFixtures]::Unicode('CUBBY-AFTER-HUNG-OWNER')))
 $published=([DateTime]::UtcNow-$start).TotalSeconds
 $captured=$null
 while(!$probe.HasExited){
  if(Get-Content "$dir\hung-owner-captures.jsonl" | Select-String '"matches":\[0\]'){$captured=([DateTime]::UtcNow-$start).TotalSeconds;break}
  Start-Sleep -Milliseconds 200
 }
 $ownerHung=!$owner.HasExited
 $probe.WaitForExit()
 if(!$owner.HasExited){Stop-Process -Id $owner.Id}
 $result=@{published_s=[math]::Round($published,1);captured_s=if($captured){[math]::Round($captured,1)}else{$null};owner_still_frozen_at_capture=$ownerHung;probe_exit=$probe.ExitCode}
 $result | ConvertTo-Json -Compress | Set-Content "$dir\hung-owner-result.json"
 if(!$captured -or !$ownerHung){throw "Copy after a frozen owner was not captured while it stayed frozen: $($result | ConvertTo-Json -Compress)"}
 return
}
$probe=Start-Probe 'extra' @($fixtures | ForEach-Object {$_.expected}) 15
foreach($fixture in $fixtures){
 if($fixture.ole){[System.Windows.Forms.Clipboard]::SetDataObject($fixture.ole,$false)}else{[CaptureFixtures]::Publish($fixture.formats,$fixture.buffers)}
 Wait-Pumping 350
}
$probe.WaitForExit()
@{exit=$probe.ExitCode;utc=[DateTime]::UtcNow.ToString('o')} | ConvertTo-Json -Compress | Set-Content "$dir\extra-result.json"

if($probe.ExitCode -ne 0){throw "Capture regression failed; see $dir"}
$summary=Get-Content "$dir\extra-captures.jsonl" | ForEach-Object {$_ | ConvertFrom-Json} | Where-Object event -eq summary
if(!$summary -or $summary.matched -ne $fixtures.Count -or @($summary.missing).Count -ne 0){throw "Capture regression did not match all $($fixtures.Count) fixtures"}
