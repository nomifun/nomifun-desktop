param([Parameter(Mandatory = $true)][string]$Configuration)
$ErrorActionPreference = 'Stop'
$settings = Get-Content -LiteralPath $Configuration -Raw | ConvertFrom-Json
if ($settings.purpose -ne 'isolated-synthetic-keyboard-update-recovery-v1') { throw 'Unexpected fixture configuration' }

# Start the exact fixture suspended, attach it to a private job, and only then
# permit execution. Every child (including its private WebView2) has the same
# lifetime owner; closing this launcher cannot leave a recording/app behind.
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

public static class KeyboardRecoveryLauncher {
    [StructLayout(LayoutKind.Sequential)] struct SecurityAttributes {
        public int Length; public IntPtr Descriptor;
        [MarshalAs(UnmanagedType.Bool)] public bool Inherit;
    }
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)] struct StartupInfo {
        public int Size; public string Reserved; public string Desktop; public string Title;
        public uint X, Y, XSize, YSize, XChars, YChars, Fill, Flags;
        public ushort Show, ReservedSize; public IntPtr ReservedBytes, Input, Output, Error;
    }
    [StructLayout(LayoutKind.Sequential)] struct ProcessInfo {
        public IntPtr Process, Thread; public uint Pid, Tid;
    }
    [StructLayout(LayoutKind.Sequential)] struct BasicLimits {
        public long ProcessTime, JobTime; public uint Flags;
        public UIntPtr MinWorkingSet, MaxWorkingSet;
        public uint ActiveLimit; public UIntPtr Affinity; public uint Priority, Scheduling;
    }
    [StructLayout(LayoutKind.Sequential)] struct IoCounters {
        public ulong ReadOperations, WriteOperations, OtherOperations, ReadBytes, WriteBytes, OtherBytes;
    }
    [StructLayout(LayoutKind.Sequential)] struct ExtendedLimits {
        public BasicLimits Basic; public IoCounters Io;
        public UIntPtr ProcessMemory, JobMemory, PeakProcessMemory, PeakJobMemory;
    }
    [StructLayout(LayoutKind.Sequential)] struct Accounting {
        public long UserTime, KernelTime, PeriodUserTime, PeriodKernelTime;
        public uint PageFaults, TotalProcesses, ActiveProcesses, TerminatedProcesses;
    }
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern IntPtr CreateJobObject(IntPtr attributes, string name);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool SetInformationJobObject(IntPtr job, int type, ref ExtendedLimits value, uint size);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool QueryInformationJobObject(IntPtr job, int type, out Accounting value, uint size, IntPtr returned);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool TerminateJobObject(IntPtr job, uint code);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool TerminateProcess(IntPtr process, uint code);
    [DllImport("kernel32.dll", SetLastError=true)] static extern uint ResumeThread(IntPtr thread);
    [DllImport("kernel32.dll", SetLastError=true)] static extern uint WaitForSingleObject(IntPtr handle, uint milliseconds);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool GetExitCodeProcess(IntPtr process, out uint code);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern IntPtr CreateFile(string path, uint access, uint share, ref SecurityAttributes attributes, uint creation, uint flags, IntPtr template);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern bool CreateProcess(string image, StringBuilder command, IntPtr processAttributes, IntPtr threadAttributes, bool inheritHandles, uint flags, IntPtr environment, string directory, ref StartupInfo startup, out ProcessInfo process);

    static void Check(bool success) { if (!success) throw new Win32Exception(Marshal.GetLastWin32Error()); }
    static string Quote(string value) {
        var result = new StringBuilder("\""); int slashes = 0;
        foreach (char c in value) {
            if (c == '\\') { slashes++; continue; }
            result.Append('\\', c == '"' ? slashes * 2 + 1 : slashes);
            result.Append(c); slashes = 0;
        }
        result.Append('\\', slashes * 2); return result.Append('"').ToString();
    }
    static Accounting Count(IntPtr job) {
        Accounting value;
        Check(QueryInformationJobObject(job, 1, out value, (uint)Marshal.SizeOf(typeof(Accounting)), IntPtr.Zero));
        return value;
    }
    public static Dictionary<string, object> Run(string image, string mode, string report, string directory, string log, string[] environment, uint timeout) {
        IntPtr job = IntPtr.Zero, output = IntPtr.Zero, input = IntPtr.Zero, block = IntPtr.Zero;
        ProcessInfo process = new ProcessInfo(); bool assigned = false;
        var result = new Dictionary<string, object>();
        try {
            job = CreateJobObject(IntPtr.Zero, null); Check(job != IntPtr.Zero);
            var limits = new ExtendedLimits(); limits.Basic.Flags = 0x2000; // kill descendants when the owner closes
            Check(SetInformationJobObject(job, 9, ref limits, (uint)Marshal.SizeOf(typeof(ExtendedLimits))));
            var attributes = new SecurityAttributes { Length = Marshal.SizeOf(typeof(SecurityAttributes)), Inherit = true };
            output = CreateFile(log, 0x40000000, 3, ref attributes, 2, 0x80, IntPtr.Zero);
            Check(output != new IntPtr(-1));
            input = CreateFile("NUL", 0x80000000, 3, ref attributes, 3, 0x80, IntPtr.Zero);
            Check(input != new IntPtr(-1));
            Array.Sort(environment, StringComparer.OrdinalIgnoreCase);
            block = Marshal.StringToHGlobalUni(String.Join("\0", environment) + "\0\0");
            var startup = new StartupInfo { Size = Marshal.SizeOf(typeof(StartupInfo)), Flags = 0x100, Input = input, Output = output, Error = output };
            var command = new StringBuilder(Quote(image) + " " + Quote(mode) + " " + Quote(report));
            Check(CreateProcess(image, command, IntPtr.Zero, IntPtr.Zero, true, 0x08000404, block, directory, ref startup, out process));
            Check(AssignProcessToJobObject(job, process.Process)); assigned = true;
            Check(ResumeThread(process.Thread) != 0xffffffff);
            uint waited = WaitForSingleObject(process.Process, timeout);
            Check(waited != 0xffffffff);
            bool timedOut = waited == 258;
            if (timedOut) { Check(TerminateJobObject(job, 124)); WaitForSingleObject(process.Process, 5000); }
            uint exitCode; Check(GetExitCodeProcess(process.Process, out exitCode));
            DateTime deadline = DateTime.UtcNow.AddSeconds(5);
            Accounting accounting = Count(job);
            while (accounting.ActiveProcesses != 0 && DateTime.UtcNow < deadline) { Thread.Sleep(50); accounting = Count(job); }
            bool forced = accounting.ActiveProcesses != 0;
            if (forced) {
                Check(TerminateJobObject(job, 125)); deadline = DateTime.UtcNow.AddSeconds(5);
                do { Thread.Sleep(50); accounting = Count(job); } while (accounting.ActiveProcesses != 0 && DateTime.UtcNow < deadline);
            }
            result["pid"] = process.Pid; result["exitCode"] = exitCode;
            result["timedOut"] = timedOut; result["cleanupForced"] = forced;
            result["activeOwnedProcesses"] = accounting.ActiveProcesses;
            result["totalOwnedProcesses"] = accounting.TotalProcesses;
            result["suspendedBeforeJobAssignment"] = true;
            result["privateJobOwnedChildren"] = true;
            return result;
        } finally {
            if (process.Process != IntPtr.Zero && !assigned) TerminateProcess(process.Process, 126);
            if (job != IntPtr.Zero) CloseHandle(job);
            if (process.Thread != IntPtr.Zero) CloseHandle(process.Thread);
            if (process.Process != IntPtr.Zero) CloseHandle(process.Process);
            if (output != IntPtr.Zero && output != new IntPtr(-1)) CloseHandle(output);
            if (input != IntPtr.Zero && input != new IntPtr(-1)) CloseHandle(input);
            if (block != IntPtr.Zero) Marshal.FreeHGlobal(block);
        }
    }
}
'@
$environment = @($settings.environment.PSObject.Properties | ForEach-Object { $_.Name + '=' + [string]$_.Value })
$outcome = [KeyboardRecoveryLauncher]::Run($settings.binary, $settings.mode, $settings.fixtureReport, $settings.profile, $settings.log, $environment, 240000)
$outcome | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $settings.processReport -Encoding UTF8
