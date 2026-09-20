#Requires -Version 7.0
<#
    .SYNOPSIS
    Runs one program and measures its whole process tree: wall-clock time,
    CPU time, and peak memory — the same way for every technology the
    benchmark builds with.

    .DESCRIPTION
    A build is not one process. NSIS's makensis.exe in the install root is a
    2.5 KB launcher for Bin\makensis.exe; a compiler may start helpers. So
    the program is started suspended, placed in a Windows job object of its
    own before its first instruction runs (every process it creates inherits
    the job), and resumed; the meter then follows the job, not the process:

      - wall clock: from the resume until the started process has exited
        and the job holds no process at all;
      - CPU time: the job's own accounting (user + kernel time of every
        process that ever belonged to it, exited processes included) — exact,
        maintained by the kernel;
      - peak commit: the job's PeakJobMemoryUsed (the highest commit charge
        of all its processes together at any instant) and
        PeakProcessMemoryUsed (the highest single process) — exact, kernel-
        maintained, no sampling;
      - peak working set: sampled. Every SampleIntervalMilliseconds the job's
        process list is read and each process's current working set is
        summed (`peakTreeWorkingSetBytes` is the largest sum seen) and its
        Windows-tracked PeakWorkingSetSize is read (`peakProcessWorkingSetBytes`
        is the largest single-process peak seen). Windows does not keep a
        job-wide working-set peak, so this is the one figure sampling can
        miss: a spike shorter than the interval, or a process that lives
        and dies between two samples. The number of samples taken and of
        processes seen against the job's own total are recorded with every
        result so the reader can judge the coverage.

    The interval is the trade between coverage and interference: 20 ms
    gives a 0.4 s build twenty samples and costs the measured build a
    negligible share of one core. The meter runs in the calling PowerShell
    process, which is outside the job and never counted.

    Standard output and error of the program go to LogPath (one file, the
    way the benchmark's build logs always were). The job is created with
    KILL_ON_JOB_CLOSE, so a build that outlives its timeout, or a meter that
    dies, leaves no compiler behind.
#>

Set-StrictMode -Version Latest

$source = @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

namespace TigerSetupBenchmark
{
    public sealed class ProcessPeak
    {
        public int ProcessId;
        public string Name;
        public long PeakWorkingSetBytes;
        public long PeakPagefileBytes;
        public int Samples;
    }

    public sealed class TreeRunResult
    {
        public int ExitCode;
        public bool TimedOut;
        public DateTime StartedAt;
        public DateTime FinishedAt;
        public double WallSeconds;
        public double UserSeconds;
        public double KernelSeconds;
        public long PeakTreeWorkingSetBytes;
        public long PeakProcessWorkingSetBytes;
        public string PeakProcessName;
        public long PeakJobCommitBytes;
        public long PeakProcessCommitBytes;
        public int Samples;
        public int ProcessesSeen;
        public int TotalProcesses;
        public int SampleIntervalMilliseconds;
        public List<ProcessPeak> Processes;
    }

    public static class ProcessTreeMeter
    {
        [StructLayout(LayoutKind.Sequential)]
        struct SECURITY_ATTRIBUTES { public int nLength; public IntPtr lpSecurityDescriptor; public int bInheritHandle; }

        [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
        struct STARTUPINFOW
        {
            public int cb; public IntPtr lpReserved; public IntPtr lpDesktop; public IntPtr lpTitle;
            public int dwX, dwY, dwXSize, dwYSize, dwXCountChars, dwYCountChars, dwFillAttribute, dwFlags;
            public short wShowWindow, cbReserved2; public IntPtr lpReserved2;
            public IntPtr hStdInput, hStdOutput, hStdError;
        }

        [StructLayout(LayoutKind.Sequential)]
        struct PROCESS_INFORMATION { public IntPtr hProcess, hThread; public int dwProcessId, dwThreadId; }

        [StructLayout(LayoutKind.Sequential)]
        struct JOBOBJECT_BASIC_LIMIT_INFORMATION
        {
            public long PerProcessUserTimeLimit, PerJobUserTimeLimit; public uint LimitFlags;
            public ulong MinimumWorkingSetSize, MaximumWorkingSetSize; public uint ActiveProcessLimit;
            public ulong Affinity; public uint PriorityClass, SchedulingClass;
        }

        [StructLayout(LayoutKind.Sequential)]
        struct IO_COUNTERS { public ulong ReadOperationCount, WriteOperationCount, OtherOperationCount, ReadTransferCount, WriteTransferCount, OtherTransferCount; }

        [StructLayout(LayoutKind.Sequential)]
        struct JOBOBJECT_EXTENDED_LIMIT_INFORMATION
        {
            public JOBOBJECT_BASIC_LIMIT_INFORMATION BasicLimitInformation; public IO_COUNTERS IoInfo;
            public ulong ProcessMemoryLimit, JobMemoryLimit, PeakProcessMemoryUsed, PeakJobMemoryUsed;
        }

        [StructLayout(LayoutKind.Sequential)]
        struct JOBOBJECT_BASIC_ACCOUNTING_INFORMATION
        {
            public long TotalUserTime, TotalKernelTime, ThisPeriodTotalUserTime, ThisPeriodTotalKernelTime;
            public uint TotalPageFaultCount, TotalProcesses, ActiveProcesses, TotalTerminatedProcesses;
        }

        [StructLayout(LayoutKind.Sequential)]
        struct PROCESS_MEMORY_COUNTERS_EX
        {
            public uint cb, PageFaultCount;
            public ulong PeakWorkingSetSize, WorkingSetSize, QuotaPeakPagedPoolUsage, QuotaPagedPoolUsage,
                QuotaPeakNonPagedPoolUsage, QuotaNonPagedPoolUsage, PagefileUsage, PeakPagefileUsage, PrivateUsage;
        }

        const int JobObjectBasicAccountingInformation = 1;
        const int JobObjectBasicProcessIdList = 3;
        const int JobObjectExtendedLimitInformation = 9;
        const uint JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE = 0x2000;
        const uint CREATE_SUSPENDED = 0x4;
        const uint CREATE_UNICODE_ENVIRONMENT = 0x400;
        const int STARTF_USESTDHANDLES = 0x100;
        const uint GENERIC_WRITE = 0x40000000;
        const uint FILE_SHARE_READ = 1, FILE_SHARE_WRITE = 2;
        const uint CREATE_ALWAYS = 2;
        const uint FILE_ATTRIBUTE_NORMAL = 0x80;
        const uint PROCESS_QUERY_LIMITED_INFORMATION = 0x1000;
        const uint WAIT_OBJECT_0 = 0, WAIT_TIMEOUT = 0x102;
        const int STD_INPUT_HANDLE = -10;

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)] static extern IntPtr CreateJobObjectW(IntPtr attributes, string name);
        [DllImport("kernel32.dll", SetLastError = true)] static extern bool SetInformationJobObject(IntPtr job, int infoClass, ref JOBOBJECT_EXTENDED_LIMIT_INFORMATION info, int length);
        [DllImport("kernel32.dll", SetLastError = true)] static extern bool QueryInformationJobObject(IntPtr job, int infoClass, ref JOBOBJECT_EXTENDED_LIMIT_INFORMATION info, int length, IntPtr returned);
        [DllImport("kernel32.dll", SetLastError = true)] static extern bool QueryInformationJobObject(IntPtr job, int infoClass, ref JOBOBJECT_BASIC_ACCOUNTING_INFORMATION info, int length, IntPtr returned);
        [DllImport("kernel32.dll", SetLastError = true)] static extern bool QueryInformationJobObject(IntPtr job, int infoClass, IntPtr info, int length, IntPtr returned);
        [DllImport("kernel32.dll", SetLastError = true)] static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);
        [DllImport("kernel32.dll", SetLastError = true)] static extern bool TerminateJobObject(IntPtr job, uint exitCode);
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        static extern bool CreateProcessW(string application, StringBuilder commandLine, IntPtr processAttributes, IntPtr threadAttributes,
            bool inheritHandles, uint creationFlags, IntPtr environment, string currentDirectory, ref STARTUPINFOW startupInfo, out PROCESS_INFORMATION processInformation);
        [DllImport("kernel32.dll", SetLastError = true)] static extern uint ResumeThread(IntPtr thread);
        [DllImport("kernel32.dll", SetLastError = true)] static extern uint WaitForSingleObject(IntPtr handle, uint milliseconds);
        [DllImport("kernel32.dll", SetLastError = true)] static extern bool GetExitCodeProcess(IntPtr process, out uint exitCode);
        [DllImport("kernel32.dll", SetLastError = true)] static extern bool CloseHandle(IntPtr handle);
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        static extern IntPtr CreateFileW(string name, uint access, uint share, ref SECURITY_ATTRIBUTES attributes, uint disposition, uint flags, IntPtr template);
        [DllImport("kernel32.dll", SetLastError = true)] static extern IntPtr GetStdHandle(int handle);
        [DllImport("kernel32.dll", SetLastError = true)] static extern IntPtr OpenProcess(uint access, bool inherit, int processId);
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)] static extern bool QueryFullProcessImageNameW(IntPtr process, uint flags, StringBuilder name, ref int size);
        [DllImport("psapi.dll", SetLastError = true)] static extern bool GetProcessMemoryInfo(IntPtr process, out PROCESS_MEMORY_COUNTERS_EX counters, uint size);

        sealed class Tracked { public IntPtr Handle; public ProcessPeak Peak; }

        static readonly IntPtr INVALID_HANDLE_VALUE = new IntPtr(-1);

        public static TreeRunResult Run(string application, string commandLine, string workingDirectory, string logPath, int sampleIntervalMilliseconds, int timeoutSeconds)
        {
            if (sampleIntervalMilliseconds < 1) sampleIntervalMilliseconds = 1;
            var result = new TreeRunResult { SampleIntervalMilliseconds = sampleIntervalMilliseconds, Processes = new List<ProcessPeak>(), ExitCode = -1 };
            var tracked = new Dictionary<int, Tracked>();
            IntPtr job = IntPtr.Zero, log = INVALID_HANDLE_VALUE;
            var process = new PROCESS_INFORMATION();
            try
            {
                job = CreateJobObjectW(IntPtr.Zero, null);
                if (job == IntPtr.Zero) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error(), "CreateJobObject");
                var limits = new JOBOBJECT_EXTENDED_LIMIT_INFORMATION();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if (!SetInformationJobObject(job, JobObjectExtendedLimitInformation, ref limits, Marshal.SizeOf(typeof(JOBOBJECT_EXTENDED_LIMIT_INFORMATION))))
                    throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error(), "SetInformationJobObject");

                var security = new SECURITY_ATTRIBUTES { nLength = Marshal.SizeOf(typeof(SECURITY_ATTRIBUTES)), bInheritHandle = 1 };
                log = CreateFileW(logPath, GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE, ref security, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, IntPtr.Zero);
                if (log == INVALID_HANDLE_VALUE) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error(), "CreateFile " + logPath);

                var startup = new STARTUPINFOW();
                startup.cb = Marshal.SizeOf(typeof(STARTUPINFOW));
                startup.dwFlags = STARTF_USESTDHANDLES;
                startup.hStdInput = GetStdHandle(STD_INPUT_HANDLE);
                startup.hStdOutput = log;
                startup.hStdError = log;
                var line = new StringBuilder(commandLine, commandLine.Length + 64);
                if (!CreateProcessW(application, line, IntPtr.Zero, IntPtr.Zero, true, CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT, IntPtr.Zero,
                        string.IsNullOrEmpty(workingDirectory) ? null : workingDirectory, ref startup, out process))
                    throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error(), "CreateProcess " + application);
                if (!AssignProcessToJobObject(job, process.hProcess))
                {
                    int error = Marshal.GetLastWin32Error();
                    TerminateJobObject(job, 1);
                    throw new System.ComponentModel.Win32Exception(error, "AssignProcessToJobObject");
                }
                CloseHandle(log); log = INVALID_HANDLE_VALUE; // the child holds its own inherited handle now

                var clock = System.Diagnostics.Stopwatch.StartNew();
                result.StartedAt = DateTime.Now;
                if (ResumeThread(process.hThread) == unchecked((uint)-1)) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error(), "ResumeThread");
                CloseHandle(process.hThread); process.hThread = IntPtr.Zero;

                bool rootExited = false;
                long deadline = (long)timeoutSeconds * 1000;
                for (;;)
                {
                    // One sample per tick; the tick is the wait on the started
                    // process while it runs, and a sleep while its children finish.
                    Sample(job, tracked, result);
                    if (rootExited && ActiveProcesses(job) == 0) break;
                    if (clock.ElapsedMilliseconds > deadline) { result.TimedOut = true; TerminateJobObject(job, 1); break; }
                    if (rootExited) { System.Threading.Thread.Sleep(sampleIntervalMilliseconds); continue; }
                    uint wait = WaitForSingleObject(process.hProcess, (uint)sampleIntervalMilliseconds);
                    if (wait == WAIT_OBJECT_0) rootExited = true;
                    else if (wait != WAIT_TIMEOUT) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error(), "WaitForSingleObject");
                }
                clock.Stop();
                result.FinishedAt = DateTime.Now;
                result.WallSeconds = clock.Elapsed.TotalSeconds;

                uint code;
                if (GetExitCodeProcess(process.hProcess, out code)) result.ExitCode = unchecked((int)code);

                var accounting = new JOBOBJECT_BASIC_ACCOUNTING_INFORMATION();
                if (QueryInformationJobObject(job, JobObjectBasicAccountingInformation, ref accounting, Marshal.SizeOf(typeof(JOBOBJECT_BASIC_ACCOUNTING_INFORMATION)), IntPtr.Zero))
                {
                    result.UserSeconds = accounting.TotalUserTime / 10000000.0;
                    result.KernelSeconds = accounting.TotalKernelTime / 10000000.0;
                    result.TotalProcesses = (int)accounting.TotalProcesses;
                }
                var extended = new JOBOBJECT_EXTENDED_LIMIT_INFORMATION();
                if (QueryInformationJobObject(job, JobObjectExtendedLimitInformation, ref extended, Marshal.SizeOf(typeof(JOBOBJECT_EXTENDED_LIMIT_INFORMATION)), IntPtr.Zero))
                {
                    result.PeakJobCommitBytes = (long)extended.PeakJobMemoryUsed;
                    result.PeakProcessCommitBytes = (long)extended.PeakProcessMemoryUsed;
                }
                foreach (var entry in tracked.Values)
                {
                    result.Processes.Add(entry.Peak);
                    if (entry.Peak.PeakWorkingSetBytes > result.PeakProcessWorkingSetBytes)
                    {
                        result.PeakProcessWorkingSetBytes = entry.Peak.PeakWorkingSetBytes;
                        result.PeakProcessName = entry.Peak.Name;
                    }
                }
                result.ProcessesSeen = tracked.Count;
                return result;
            }
            finally
            {
                foreach (var entry in tracked.Values) if (entry.Handle != IntPtr.Zero) CloseHandle(entry.Handle);
                if (process.hThread != IntPtr.Zero) CloseHandle(process.hThread);
                if (process.hProcess != IntPtr.Zero) CloseHandle(process.hProcess);
                if (log != INVALID_HANDLE_VALUE) CloseHandle(log);
                if (job != IntPtr.Zero) CloseHandle(job); // KILL_ON_JOB_CLOSE ends whatever a timeout left
            }
        }

        static int ActiveProcesses(IntPtr job)
        {
            var accounting = new JOBOBJECT_BASIC_ACCOUNTING_INFORMATION();
            if (!QueryInformationJobObject(job, JobObjectBasicAccountingInformation, ref accounting, Marshal.SizeOf(typeof(JOBOBJECT_BASIC_ACCOUNTING_INFORMATION)), IntPtr.Zero)) return 0;
            return (int)accounting.ActiveProcesses;
        }

        static int[] ProcessIds(IntPtr job)
        {
            const int capacity = 4096;
            int size = 8 + capacity * IntPtr.Size;
            IntPtr buffer = Marshal.AllocHGlobal(size);
            try
            {
                if (!QueryInformationJobObject(job, JobObjectBasicProcessIdList, buffer, size, IntPtr.Zero)) return new int[0];
                int count = Marshal.ReadInt32(buffer, 4);
                var ids = new int[count];
                for (int i = 0; i < count; i++) ids[i] = (int)Marshal.ReadIntPtr(buffer, 8 + i * IntPtr.Size).ToInt64();
                return ids;
            }
            finally { Marshal.FreeHGlobal(buffer); }
        }

        static void Sample(IntPtr job, Dictionary<int, Tracked> tracked, TreeRunResult result)
        {
            int[] ids = ProcessIds(job);
            long workingSet = 0;
            var alive = new HashSet<int>(ids);
            foreach (int id in ids)
            {
                Tracked entry;
                if (!tracked.TryGetValue(id, out entry))
                {
                    IntPtr handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, id);
                    if (handle == IntPtr.Zero) continue; // gone already, or not ours to read
                    var name = new StringBuilder(1024); int length = name.Capacity;
                    string image = QueryFullProcessImageNameW(handle, 0, name, ref length) ? System.IO.Path.GetFileName(name.ToString(0, length)) : ("pid " + id);
                    entry = new Tracked { Handle = handle, Peak = new ProcessPeak { ProcessId = id, Name = image } };
                    tracked[id] = entry;
                }
                PROCESS_MEMORY_COUNTERS_EX counters;
                if (!GetProcessMemoryInfo(entry.Handle, out counters, (uint)Marshal.SizeOf(typeof(PROCESS_MEMORY_COUNTERS_EX)))) continue;
                entry.Peak.Samples++;
                workingSet += (long)counters.WorkingSetSize;
                if ((long)counters.PeakWorkingSetSize > entry.Peak.PeakWorkingSetBytes) entry.Peak.PeakWorkingSetBytes = (long)counters.PeakWorkingSetSize;
                if ((long)counters.PeakPagefileUsage > entry.Peak.PeakPagefileBytes) entry.Peak.PeakPagefileBytes = (long)counters.PeakPagefileUsage;
            }
            // A process that has left the job is not read again; its handle is closed here, its peaks stay recorded.
            foreach (var pair in tracked) if (!alive.Contains(pair.Key) && pair.Value.Handle != IntPtr.Zero) { CloseHandle(pair.Value.Handle); pair.Value.Handle = IntPtr.Zero; }
            if (workingSet > result.PeakTreeWorkingSetBytes) result.PeakTreeWorkingSetBytes = workingSet;
            result.Samples++;
        }
    }
}
'@

if (-not ('TigerSetupBenchmark.ProcessTreeMeter' -as [type])) {
    Add-Type -TypeDefinition $source -Language CSharp
}

function Invoke-MeasuredProcess {
    <#
        .SYNOPSIS
        Runs one program under the process-tree meter and returns its
        measurement as an ordered hashtable ready for a results file.

        .PARAMETER FilePath
        The program to run (an absolute path; nothing is searched).

        .PARAMETER ArgumentList
        Its arguments, quoted for the Windows command line by this function
        (an argument with a space or a quote is wrapped and its quotes
        escaped the way the C runtime parses them).

        .PARAMETER LogPath
        Where standard output and standard error go, together.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)] [string] $FilePath,
        [string[]] $ArgumentList = @(),
        [Parameter(Mandatory)] [string] $LogPath,
        [string] $WorkingDirectory,
        [int] $SampleIntervalMilliseconds = 20,
        [int] $TimeoutSeconds = 3600
    )
    $quoted = foreach ($argument in @($ArgumentList)) {
        if ($argument -match '[\s"]' -or $argument -eq '') { '"' + ($argument -replace '(\\*)"', '$1$1\"' -replace '(\\+)$', '$1$1') + '"' } else { $argument }
    }
    $commandLine = (@('"' + $FilePath + '"') + @($quoted)) -join ' '
    $logDirectory = Split-Path -Parent $LogPath
    if ($logDirectory -and -not (Test-Path -LiteralPath $logDirectory)) { $null = New-Item -ItemType Directory -Path $logDirectory -Force }
    $run = [TigerSetupBenchmark.ProcessTreeMeter]::Run($FilePath, $commandLine, $WorkingDirectory, $LogPath, $SampleIntervalMilliseconds, $TimeoutSeconds)
    [ordered]@{
        commandLine = $commandLine
        exitCode = $run.ExitCode
        timedOut = $run.TimedOut
        startedAt = ([DateTimeOffset] $run.StartedAt).ToString('o')
        finishedAt = ([DateTimeOffset] $run.FinishedAt).ToString('o')
        wallSeconds = [math]::Round($run.WallSeconds, 3)
        cpuSeconds = [math]::Round($run.UserSeconds + $run.KernelSeconds, 3)
        userSeconds = [math]::Round($run.UserSeconds, 3)
        kernelSeconds = [math]::Round($run.KernelSeconds, 3)
        peakTreeWorkingSetBytes = $run.PeakTreeWorkingSetBytes
        peakProcessWorkingSetBytes = $run.PeakProcessWorkingSetBytes
        peakProcessName = $run.PeakProcessName
        peakJobCommitBytes = $run.PeakJobCommitBytes
        peakProcessCommitBytes = $run.PeakProcessCommitBytes
        sampleIntervalMilliseconds = $run.SampleIntervalMilliseconds
        samples = $run.Samples
        processesSeen = $run.ProcessesSeen
        processesTotal = $run.TotalProcesses
        processes = @($run.Processes | Sort-Object ProcessId | ForEach-Object {
            [ordered]@{ pid = $_.ProcessId; name = $_.Name; peakWorkingSetBytes = $_.PeakWorkingSetBytes; peakPagefileBytes = $_.PeakPagefileBytes; samples = $_.Samples }
        })
    }
}

Export-ModuleMember -Function Invoke-MeasuredProcess
