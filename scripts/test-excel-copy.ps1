# Excel copy regression for the real capture listener. Disposable Windows VM only:
# run interactively (session with a desktop, STA). It replaces the clipboard and
# drives Excel with real Ctrl+C input. Synthetic data only; it records lengths,
# booleans and clipboard sequence numbers, never clipboard payloads.
#
#   powershell -STA -File test-excel-copy.ps1 -Setup
#   powershell -STA -File test-excel-copy.ps1 -Arms "closed;base=C:\a\cubby.exe;fix=C:\b\cubby.exe"
#
# Arms are interleaved and their order rotates each round. Per trial it writes
# <Label>.jsonl (paste checks, sequence before/after), <Label>-warnings.csv (Excel's
# "clipboard in use" dialog, detected through UI Automation and closed), and
# <Label>-captures.jsonl (the sequence of every [perf][clipboard_ingest] record the
# arm logged). A copy counts as captured when an ingest sequence falls between its
# own sequence and the next copy's.
param(
 [string]$Arms,
 [switch]$Setup,
 [int]$Rounds=2,[int]$Single=50,[int]$Formatted=50,[int]$Large=30,[int]$Rapid=300,
 [string]$Label='excel-copy'
)
$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$dir="$env:TEMP\cubby-excel-investigation"
$log="$env:LOCALAPPDATA\ai.southforge.cubbyclipboard\logs\Cubby Clipboard.log"
if($Setup){
 # 20 x 10,002 values, A1:D20 bold on yellow with two decimals.
 New-Item -ItemType Directory -Force $dir | Out-Null
 $excel=New-Object -ComObject Excel.Application
 $excel.Visible=$true
 $book=$excel.Workbooks.Add()
 $sheet=$book.Worksheets.Item(1)
 $sheet.Name='Synthetic'
 $sheet.Range('A1:T10002').Formula='=ROW()*100+COLUMN()'
 $sheet.Range('A1:T10002').Value2=$sheet.Range('A1:T10002').Value2
 $sheet.Range('A1:D20').Font.Bold=$true
 $sheet.Range('A1:D20').Interior.Color=65535
 $sheet.Range('A1:D20').NumberFormat='0.00'
 $book.SaveAs("$dir\synthetic.xlsx",51)
 $sheet.Range('A1').Select()
 return
}
if(!$Arms){throw 'Pass -Arms, or -Setup to create the synthetic workbook first'}
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
Add-Type -ReferencedAssemblies @('System.dll','System.Core.dll', [System.Windows.Automation.AutomationElement].Assembly.Location,[System.Windows.Automation.TreeScope].Assembly.Location) -TypeDefinition @'
using System;
using System.Text;
using System.Threading;
using System.Collections.Generic;
using System.Collections.Concurrent;
using System.Runtime.InteropServices;
using System.Windows.Automation;
public static class ExcelTrial {
 public delegate bool EnumProc(IntPtr h, IntPtr l);
 [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f,IntPtr l);
 [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr h,EnumProc f,IntPtr l);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr h,StringBuilder b,int n);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h,StringBuilder b,int n);
 [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h,out uint p);
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
 [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll")] public static extern uint GetClipboardSequenceNumber();
 [DllImport("user32.dll")] static extern IntPtr GetOpenClipboardWindow();
 [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
 [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h,uint m,IntPtr w,IntPtr l);
 [DllImport("user32.dll")] static extern bool PostMessage(IntPtr h,uint m,IntPtr w,IntPtr l);
 [DllImport("user32.dll")] static extern void keybd_event(byte v,byte s,uint f,UIntPtr e);
 public static readonly ConcurrentQueue<string> Warnings=new ConcurrentQueue<string>();
 public static volatile bool Running;
 public static volatile int Trial;
 public static volatile string Arm="setup";
 static Thread watcher;
 public static void Stop(){Running=false;if(watcher!=null)watcher.Join(2000);}
 public static void Copy() { keybd_event(17,0,0,UIntPtr.Zero); keybd_event(67,0,0,UIntPtr.Zero); keybd_event(67,0,2,UIntPtr.Zero); keybd_event(17,0,2,UIntPtr.Zero); }
 public static void Watch(uint excelPid) {
  Running=true;
  watcher=new Thread(()=>{
   var seen=new HashSet<IntPtr>();
   while(Running){
    var present=new HashSet<IntPtr>();
    EnumWindows((h,l)=>{
     uint p;GetWindowThreadProcessId(h,out p);
     if(p!=excelPid || !IsWindowVisible(h))return true;
     var cls=new StringBuilder(256);GetClassName(h,cls,cls.Capacity);
     if(cls.ToString()=="XLMAIN")return true;
     var b=new StringBuilder(4096);GetWindowText(h,b,b.Capacity);
     var all=new StringBuilder(b.ToString());
     EnumChildWindows(h,(c,x)=>{b.Clear();GetWindowText(c,b,b.Capacity);all.Append(' ').Append(b);return true;},IntPtr.Zero);
     var elements=new List<AutomationElement>();
     try {
      var root=AutomationElement.FromHandle(h);
      var found=root.FindAll(TreeScope.Descendants,Condition.TrueCondition);
      for(int n=0;n<found.Count;n++){var a=found[n];all.Append(' ').Append(a.Current.Name);elements.Add(a);}
     }catch{}
     var text=all.ToString();
     if(text.IndexOf("clipboard",StringComparison.OrdinalIgnoreCase)<0 || text.IndexOf("another application",StringComparison.OrdinalIgnoreCase)<0)return true;
     present.Add(h);
     if(seen.Add(h)){
      var open=GetOpenClipboardWindow(); uint holder=0;if(open!=IntPtr.Zero)GetWindowThreadProcessId(open,out holder);
      Warnings.Enqueue(String.Format("{0:o},{1},{2},{3},{4},{5}",DateTime.UtcNow,Arm,Trial,GetClipboardSequenceNumber(),open.ToInt64(),holder));
      bool dismissed=false;
      foreach(var a in elements){try {object pat;string name=a.Current.Name.Replace("&","");if((name.Equals("OK",StringComparison.OrdinalIgnoreCase)||name.Equals("Close",StringComparison.OrdinalIgnoreCase)) && a.TryGetCurrentPattern(InvokePattern.Pattern,out pat)){((InvokePattern)pat).Invoke();dismissed=true;break;}}catch{}}
      if(!dismissed)PostMessage(h,0x10,IntPtr.Zero,IntPtr.Zero);
     }
     return true;
    },IntPtr.Zero);
    seen.IntersectWith(present);Thread.Sleep(10);
   }
  }){IsBackground=true};watcher.Start();
 }
 public static readonly ConcurrentQueue<string> LogLines=new ConcurrentQueue<string>();
 static Thread tailer; static volatile bool Tailing;
 public static void StopTail(){Tailing=false;if(tailer!=null)tailer.Join(2000);}
 // The app log rotates at ~40 KB and keeps one file: poll often so ingest records are not lost.
 public static void TailLog(string path) {
  Tailing=true;
  tailer=new Thread(()=>{
   long offset=-1; string partial="";
   while(Tailing){
    try {
     using(var fs=new System.IO.FileStream(path,System.IO.FileMode.Open,System.IO.FileAccess.Read,System.IO.FileShare.ReadWrite|System.IO.FileShare.Delete)){
      if(offset<0)offset=fs.Length;
      if(fs.Length<offset){offset=0;partial="";}
      if(fs.Length>offset){
       fs.Seek(offset,System.IO.SeekOrigin.Begin);
       var buf=new byte[fs.Length-offset]; int n=fs.Read(buf,0,buf.Length); offset+=n;
       var lines=(partial+Encoding.UTF8.GetString(buf,0,n)).Split('\n'); partial=lines[lines.Length-1];
       for(int i=0;i<lines.Length-1;i++){var l=lines[i];if(l.Contains("[perf][clipboard_ingest]")||(l.Contains("[cubby::clipboard]")&&(l.Contains("[WARN]")||l.Contains("[ERROR]"))))LogLines.Enqueue(l.TrimEnd('\r'));}
      }
     }
    } catch {}
    Thread.Sleep(5);
   }
  }){IsBackground=true};tailer.Start();
 }
 public static string Expected(int first,int last,int col,int cols,bool formatted) {
  var b=new StringBuilder();for(int r=first;r<=last;r++){for(int c=col;c<col+cols;c++){if(c>col)b.Append('\t');b.Append(r*100+c);if(formatted)b.Append(".00");}b.Append("\r\n");}return b.ToString();
 }
}
'@
$armList=@()
foreach($a in $Arms.Split(';')){ if($a -eq 'closed'){$armList+=@{name='closed';path=$null}} else {$kv=$a.Split('=',2);$armList+=@{name=$kv[0];path=$kv[1]}} }
foreach($a in $armList){ if($a.path -and !(Test-Path $a.path)){throw "Missing arm executable $($a.path)"} }
$output=New-Object System.IO.StreamWriter("$dir\$Label.jsonl",$false); $output.AutoFlush=$true
$warnings=New-Object System.IO.StreamWriter("$dir\$Label-warnings.csv",$false); $warnings.AutoFlush=$true
$warnings.WriteLine('utc,arm,trial,sequence,open_hwnd,open_pid')
$captures=New-Object System.IO.StreamWriter("$dir\$Label-captures.jsonl",$false); $captures.AutoFlush=$true
$progress="$dir\$Label-progress.txt"
$cubby=$null
try {
 $excel=[Runtime.InteropServices.Marshal]::GetActiveObject('Excel.Application')
 if($excel.Workbooks.Count -ne 1 -or $excel.ActiveWorkbook.FullName -ne "$dir\synthetic.xlsx"){throw 'Expected only the synthetic test workbook'}
 $sheet=$excel.ActiveWorkbook.Worksheets.Item(1)
 $hwnd=[IntPtr]$excel.Hwnd
 [uint32]$excelPid=0
 [void][ExcelTrial]::GetWindowThreadProcessId($hwnd,[ref]$excelPid)
 [ExcelTrial]::Arm='detector-self-test'
 [ExcelTrial]::Watch($excelPid)
 [void][ExcelTrial]::SetForegroundWindow($hwnd)
 Start-Sleep -Milliseconds 200
 [void]$excel.InputBox('Clipboard detector self-test: clipboard in use by another application','Cubby diagnostic self-test', 'OK')
 Start-Sleep -Milliseconds 100
 if([ExcelTrial]::Warnings.Count -ne 1){throw 'Warning detector self-test failed'}
 $control=$null
 [void][ExcelTrial]::Warnings.TryDequeue([ref]$control)
 $control | Set-Content "$dir\$Label-detector-self-test.csv"
 [ExcelTrial]::Stop()
 [ExcelTrial]::Watch($excelPid)
 [ExcelTrial]::TailLog($log)
 $form=New-Object System.Windows.Forms.Form
 $form.Text='Cubby Excel paste verifier (synthetic data only)'
 $box=New-Object System.Windows.Forms.TextBox
 $box.Multiline=$true;$box.MaxLength=0;$box.Dock='Fill'
 $form.Controls.Add($box);$form.Show()
 [System.Windows.Forms.Application]::DoEvents()
 $cases=@(
  @{name='single';count=$Single;pause=300;ranges=@('A1','B1');expected=@([ExcelTrial]::Expected(1,1,1,1,$true),[ExcelTrial]::Expected(1,1,2,1,$true))},
  @{name='formatted';count=$Formatted;pause=500;ranges=@('A1:D10','A11:D20');expected=@([ExcelTrial]::Expected(1,10,1,4,$true),[ExcelTrial]::Expected(11,20,1,4,$true))},
  @{name='large';count=$Large;pause=1000;ranges=@('A21:T10000','A22:T10001');expected=@([ExcelTrial]::Expected(21,10000,1,20,$false),[ExcelTrial]::Expected(22,10001,1,20,$false))},
  @{name='rapid';count=$Rapid;pause=50;ranges=@('A1','B1');expected=@([ExcelTrial]::Expected(1,1,1,1,$true),[ExcelTrial]::Expected(1,1,2,1,$true))}
 )
 $trial=0;$block=0
 for($round=1;$round -le $Rounds;$round++){
  # Rotate arm order every round so no arm always runs first or last.
  $order=@(); for($k=0;$k -lt $armList.Count;$k++){$order+=$armList[($k+$round-1)%$armList.Count]}
  foreach($arm in $order){
   $block++
   if(Get-Process | Where-Object {$_.Path -like '*cubby*'}){throw 'Unexpected Cubby instance; refusing to alter it'}
   $line=$null;while([ExcelTrial]::LogLines.TryDequeue([ref]$line)){}
   if($arm.path){
    $cubby=Start-Process $arm.path -WindowStyle Hidden -PassThru
    Start-Sleep -Seconds 5
    if($cubby.HasExited){throw "Cubby arm $($arm.name) exited at startup"}
   }
   [void][ExcelTrial]::SetForegroundWindow($hwnd)
   $focusDeadline=[DateTime]::UtcNow.AddSeconds(60)
   while([ExcelTrial]::GetForegroundWindow() -ne $hwnd){
    if([DateTime]::UtcNow -gt $focusDeadline){throw "Excel focus handoff timed out ($hwnd)"}
    Start-Sleep -Milliseconds 100
    [void][ExcelTrial]::SetForegroundWindow($hwnd)
   }
   [ExcelTrial]::Arm="$round-$($arm.name)"
   "round=$round arm=$($arm.name) block=$block trial=$trial utc=$([DateTime]::UtcNow.ToString('o'))" | Set-Content $progress
   foreach($case in $cases){
    for($i=0;$i -lt $case.count;$i++){
     $trial++;$variant=$i%2
     $sheet.Range($case.ranges[$variant]).Select()
     [void][ExcelTrial]::SetForegroundWindow($hwnd)
     if($case.name -ne 'rapid'){Start-Sleep -Milliseconds 75}
     for($focusTry=0;[ExcelTrial]::GetForegroundWindow() -ne $hwnd -and $focusTry -lt 20;$focusTry++){Start-Sleep -Milliseconds 50;[void][ExcelTrial]::SetForegroundWindow($hwnd)}
     if([ExcelTrial]::GetForegroundWindow() -ne $hwnd){throw 'Excel did not obtain foreground'}
     $before=[ExcelTrial]::GetClipboardSequenceNumber()
     $start=[DateTime]::UtcNow
     [ExcelTrial]::Trial=$trial
     [ExcelTrial]::Copy()
     Start-Sleep -Milliseconds $case.pause
     $after=[ExcelTrial]::GetClipboardSequenceNumber()
     $paste=$case.name -ne 'rapid' -or ($i+1)%10 -eq 0
     $matches=$null;$pasteLength=$null;$retryMatch=$null
     if($paste){
      $box.Clear()
      [void][ExcelTrial]::SendMessage($box.Handle,0x302,[IntPtr]::Zero,[IntPtr]::Zero)
      $pasteLength=$box.Text.Length
      $matches=$box.Text.TrimEnd("`r","`n") -ceq $case.expected[$variant].TrimEnd("`r","`n")
      if(!$matches){Start-Sleep -Milliseconds 250;$box.Clear();[void][ExcelTrial]::SendMessage($box.Handle,0x302,[IntPtr]::Zero,[IntPtr]::Zero);$retryMatch=$box.Text.TrimEnd("`r","`n") -ceq $case.expected[$variant].TrimEnd("`r","`n")}
     }
     $output.WriteLine((@{utc=$start.ToString('o');round=$round;block=$block;arm=$arm.name;case=$case.name;trial=$trial;variant=$variant;sequence_before=$before;sequence_after=$after;paste_attempted=$paste;paste_match=$matches;paste_length=$pasteLength;paste_retry_match=$retryMatch} | ConvertTo-Json -Compress))
     $warning=$null;while([ExcelTrial]::Warnings.TryDequeue([ref]$warning)){$warnings.WriteLine($warning)}
    }
   }
   Start-Sleep -Seconds 3
   if($cubby){Stop-Process -Id $cubby.Id; $cubby.WaitForExit();$cubby=$null;Start-Sleep -Seconds 2}
   # Metadata-only ingest records written while this arm ran.
   $issues=@{};$evicted=0
   $line=$null
   while([ExcelTrial]::LogLines.TryDequeue([ref]$line)){
    $m=[regex]::Match($line,'\[perf\]\[clipboard_ingest\] sequence=(\d+) type=(text|image) existing=(true|false) full_bytes=(\d+) thumb_bytes=\d+ materialize_ms=(\d+)')
    if($m.Success){$captures.WriteLine((@{block=$block;arm=$arm.name;sequence=[uint32]$m.Groups[1].Value;type=$m.Groups[2].Value;existing=$m.Groups[3].Value;full_bytes=[long]$m.Groups[4].Value;materialize_ms=[int]$m.Groups[5].Value} | ConvertTo-Json -Compress))}
    else {
     $ev=[regex]::Match($line,'evicted (\d+) older pending'); if($ev.Success){$evicted+=[int]$ev.Groups[1].Value}
     $k=($line -replace '^\[[^\]]*\]','') -replace '\d+','#'; $issues[$k]=1+[int]$issues[$k]
    }
   }
   @{block=$block;arm=$arm.name;evicted=$evicted;log_issues=$issues} | ConvertTo-Json -Compress | Add-Content "$dir\$Label-log-issues.jsonl"
  }
 }
 "done utc=$([DateTime]::UtcNow.ToString('o'))" | Set-Content $progress
} catch { $_.Exception.ToString() | Set-Content "$dir\$Label-error.txt"; throw }
finally {
 [ExcelTrial]::Stop()
 [ExcelTrial]::StopTail()
 $warning=$null;while([ExcelTrial]::Warnings.TryDequeue([ref]$warning)){$warnings.WriteLine($warning)}
 if($cubby -and !$cubby.HasExited){Stop-Process -Id $cubby.Id}
 if($form){$form.Close()}
 $warnings.Dispose();$output.Dispose();$captures.Dispose()
}
