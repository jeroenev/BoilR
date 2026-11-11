param([string]$Tag = 'manual')
$ErrorActionPreference = 'Continue'
$out = Join-Path $PSScriptRoot "probe-$Tag.txt"; if (Test-Path $out) { $out = Join-Path $PSScriptRoot "probe-$Tag-2.txt" }
$lines = New-Object System.Collections.Generic.List[string]
function W($s) { $lines.Add([string]$s) }

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class JobProbe {
  [DllImport("kernel32.dll", SetLastError=true)] public static extern bool IsProcessInJob(IntPtr hProcess, IntPtr hJob, out bool result);
  [DllImport("kernel32.dll", SetLastError=true)] public static extern bool QueryInformationJobObject(IntPtr hJob, int cls, IntPtr info, int len, out int retLen);
  [DllImport("kernel32.dll", SetLastError=true)] public static extern IntPtr GetCurrentProcess();
  [DllImport("kernel32.dll", SetLastError=true)] public static extern IntPtr OpenProcess(uint access, bool inherit, int pid);
  [DllImport("kernel32.dll", SetLastError=true)] public static extern bool CloseHandle(IntPtr h);
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] public static extern IntPtr GetModuleHandleW(string name);
  [DllImport("kernel32.dll", SetLastError=true)] public static extern uint WaitForSingleObject(IntPtr h, uint ms);
  [DllImport("kernel32.dll", SetLastError=true)] public static extern bool GetExitCodeProcess(IntPtr h, out uint code);
  [DllImport("kernel32.dll", SetLastError=true)] public static extern IntPtr CreateJobObjectW(IntPtr sa, IntPtr name);
  [DllImport("kernel32.dll", SetLastError=true)] public static extern bool AssignProcessToJobObject(IntPtr job, IntPtr proc);

  [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)]
  public struct STARTUPINFO { public int cb; public string lpReserved; public string lpDesktop; public string lpTitle;
    public int dwX, dwY, dwXSize, dwYSize, dwXCountChars, dwYCountChars, dwFillAttribute, dwFlags; public short wShowWindow, cbReserved2;
    public IntPtr lpReserved2, hStdInput, hStdOutput, hStdError; }
  [StructLayout(LayoutKind.Sequential)]
  public struct PROCESS_INFORMATION { public IntPtr hProcess, hThread; public int dwProcessId, dwThreadId; }
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
  public static extern bool CreateProcessW(string app, StringBuilder cmd, IntPtr pa, IntPtr ta, bool inherit, uint flags, IntPtr env, string dir, ref STARTUPINFO si, out PROCESS_INFORMATION pi);

  public static string Query(int cls, int size, out byte[] data) {
    IntPtr buf = Marshal.AllocHGlobal(size);
    try {
      for (int i = 0; i < size; i++) Marshal.WriteByte(buf, i, 0);
      int ret;
      bool ok = QueryInformationJobObject(IntPtr.Zero, cls, buf, size, out ret);
      int err = Marshal.GetLastWin32Error();
      data = new byte[size]; Marshal.Copy(buf, data, 0, size);
      return ok ? ("ok retLen=" + ret) : ("FAILED err=" + err);
    } finally { Marshal.FreeHGlobal(buf); }
  }
  public static string InJob(int pid) {
    IntPtr h = pid == 0 ? GetCurrentProcess() : OpenProcess(0x1000, false, pid);
    if (h == IntPtr.Zero) return "OpenProcess failed err=" + Marshal.GetLastWin32Error();
    bool r; bool ok = IsProcessInJob(h, IntPtr.Zero, out r);
    int err = Marshal.GetLastWin32Error();
    if (pid != 0) CloseHandle(h);
    return ok ? r.ToString() : ("IsProcessInJob failed err=" + err);
  }
  public static string Spawn(uint flags) { return SpawnExe(flags, "cmd.exe /c exit 0"); }
  public static string SpawnExe(uint flags, string cmdline) {
    var si = new STARTUPINFO(); si.cb = Marshal.SizeOf(si);
    PROCESS_INFORMATION pi;
    var cmd = new StringBuilder("cmd.exe /c exit 0");
    bool ok = CreateProcessW(null, cmd, IntPtr.Zero, IntPtr.Zero, false, flags | 0x08000000 /*CREATE_NO_WINDOW*/ | 0x4 /*SUSPENDED*/, IntPtr.Zero, null, ref si, out pi);
    int err = Marshal.GetLastWin32Error();
    if (!ok) return "CreateProcess FAILED GetLastError=" + err;
    bool inJob; IsProcessInJob(pi.hProcess, IntPtr.Zero, out inJob);
    ResumeThread(pi.hThread);
    WaitForSingleObject(pi.hProcess, 5000);
    uint code; GetExitCodeProcess(pi.hProcess, out code);
    CloseHandle(pi.hThread); CloseHandle(pi.hProcess);
    return "ok child pid=" + pi.dwProcessId + " childInJob=" + inJob + " exit=" + code;
  }
  [DllImport("kernel32.dll")] public static extern uint ResumeThread(IntPtr h);
  public static string NestTest() {
    IntPtr j = CreateJobObjectW(IntPtr.Zero, IntPtr.Zero);
    if (j == IntPtr.Zero) return "CreateJobObject failed " + Marshal.GetLastWin32Error();
    // create a suspended child and try to put it in a fresh (nested) job
    var si = new STARTUPINFO(); si.cb = Marshal.SizeOf(si);
    PROCESS_INFORMATION pi; var cmd = new StringBuilder("cmd.exe /c exit 0");
    if (!CreateProcessW(null, cmd, IntPtr.Zero, IntPtr.Zero, false, 0x08000000 | 0x4, IntPtr.Zero, null, ref si, out pi)) return "spawn failed " + Marshal.GetLastWin32Error();
    bool ok = AssignProcessToJobObject(j, pi.hProcess);
    int err = Marshal.GetLastWin32Error();
    ResumeThread(pi.hThread); WaitForSingleObject(pi.hProcess, 5000);
    CloseHandle(pi.hThread); CloseHandle(pi.hProcess); CloseHandle(j);
    return ok ? "AssignProcessToJobObject(new job) ok -> nesting allowed" : ("AssignProcessToJobObject failed err=" + err);
  }
}
'@

$me = Get-CimInstance Win32_Process -Filter "ProcessId=$PID"
$parent = Get-CimInstance Win32_Process -Filter "ProcessId=$($me.ParentProcessId)"
W "Tag: $Tag   time: $(Get-Date -Format o)"
W "Self PID $PID  64bit=$([Environment]::Is64BitProcess)  cmd: $($me.CommandLine)"
W "Parent PID $($me.ParentProcessId) $($parent.Name)  cmd: $($parent.CommandLine)"
W ""
W "IsProcessInJob(self, NULL): $([JobProbe]::InJob(0))"

$data = $null
$r = [JobProbe]::Query(9, 144, [ref]$data)
W "QueryInformationJobObject(NULL, ExtendedLimit=9): $r"
if ($r -like 'ok*') {
  $flags = [BitConverter]::ToUInt32($data, 16)
  W ("  LimitFlags = 0x{0:X8}" -f $flags)
  $names = [ordered]@{ 0x1='WORKINGSET'; 0x2='PROCESS_TIME'; 0x4='JOB_TIME'; 0x8='ACTIVE_PROCESS'; 0x10='AFFINITY'; 0x20='PRIORITY_CLASS'; 0x40='PRESERVE_JOB_TIME'; 0x80='SCHEDULING_CLASS'; 0x100='PROCESS_MEMORY'; 0x200='JOB_MEMORY'; 0x400='DIE_ON_UNHANDLED_EXCEPTION'; 0x800='BREAKAWAY_OK'; 0x1000='SILENT_BREAKAWAY_OK'; 0x2000='KILL_ON_JOB_CLOSE'; 0x4000='SUBSET_AFFINITY'; 0x8000='JOB_MEMORY_LOW' }
  $set = @(); foreach ($k in $names.Keys) { if ($flags -band $k) { $set += $names[$k] } }
  W "  decoded: $($set -join ', ')"
  W "  ActiveProcessLimit = $([BitConverter]::ToUInt32($data, 40))"
}
$r = [JobProbe]::Query(1, 48, [ref]$data)
W "BasicAccounting(1): $r"
if ($r -like 'ok*') { W "  TotalProcesses=$([BitConverter]::ToUInt32($data,36)) ActiveProcesses=$([BitConverter]::ToUInt32($data,40)) TotalTerminated=$([BitConverter]::ToUInt32($data,44))" }
$r = [JobProbe]::Query(3, 8 + 8*256, [ref]$data)
W "BasicProcessIdList(3): $r"
if ($r -like 'ok*') {
  $n = [BitConverter]::ToUInt32($data, 0); $m = [BitConverter]::ToUInt32($data, 4)
  W "  assigned=$n inList=$m"
  for ($i = 0; $i -lt $m; $i++) {
    $p = [BitConverter]::ToUInt64($data, 8 + 8*$i)
    $pp = Get-CimInstance Win32_Process -Filter "ProcessId=$p" -ErrorAction SilentlyContinue
    W "  PID $p $($pp.Name) (parent $($pp.ParentProcessId))"
  }
}
W ""
W "Ancestors in a job (IsProcessInJob any job):"
$cur = $me.ParentProcessId
for ($i = 0; $i -lt 6 -and $cur; $i++) {
  $pp = Get-CimInstance Win32_Process -Filter "ProcessId=$cur" -ErrorAction SilentlyContinue
  if (-not $pp) { break }
  W "  PID $cur $($pp.Name): inJob=$([JobProbe]::InJob([int]$cur))"
  $cur = $pp.ParentProcessId
}
$steam = Get-Process steam -ErrorAction SilentlyContinue | Select -First 1
if ($steam) { W "  steam.exe PID $($steam.Id): inJob=$([JobProbe]::InJob($steam.Id))" }
W ""
W "GameOverlayRenderer64.dll loaded in self: $([JobProbe]::GetModuleHandleW('GameOverlayRenderer64.dll') -ne [IntPtr]::Zero)"
W "GameOverlayRenderer.dll loaded in self: $([JobProbe]::GetModuleHandleW('GameOverlayRenderer.dll') -ne [IntPtr]::Zero)"
$mods = (Get-Process -Id $PID).Modules | ? { $_.ModuleName -match 'steam|overlay|gameoverlay' } | % { $_.FileName }
W "Modules matching steam/overlay: $($mods -join '; ')"
W ""
W "CreateProcessW cmd.exe with CREATE_BREAKAWAY_FROM_JOB: $([JobProbe]::Spawn(0x01000000))"
W "CreateProcessW cmd.exe without flag:                  $([JobProbe]::Spawn(0))"
W "32-bit SysWOW64 cmd.exe with BREAKAWAY: $([JobProbe]::SpawnExe(0x01000000, 'C:\Windows\SysWOW64\cmd.exe /c exit 0'))"
W "32-bit SysWOW64 cmd.exe without flag:  $([JobProbe]::SpawnExe(0, 'C:\Windows\SysWOW64\cmd.exe /c exit 0'))"
W "32-bit Alice-like (SysWOW64 whoami) BREAKAWAY|SUSPENDED again: $([JobProbe]::SpawnExe(0x01000000, 'C:\Windows\SysWOW64\whoami.exe'))"
W "Nested job test: $([JobProbe]::NestTest())"
W ""
W "Steam env:"
Get-ChildItem env: | ? { $_.Name -match '^(Steam|SteamOverlay|ENABLE_VK|SDL_|EnableConfiguratorSupport)' } | % { W "  $($_.Name)=$($_.Value)" }
$lines | Set-Content -Path $out -Encoding UTF8

