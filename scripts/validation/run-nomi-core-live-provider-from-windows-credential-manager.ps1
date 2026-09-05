#!/usr/bin/env pwsh

<#
Runs the credential-isolating Nomi-core live smoke using a Windows Credential
Manager entry. The secret is never placed in this script, argv, a log, or a
repository file. It is transiently visible only to the trusted Bun runner
process; that runner removes the environment variable before Cargo and the
test executable are launched, then passes the value once over stdin to the
test executable.

Usage:
  powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File scripts/validation/run-nomi-core-live-provider-from-windows-credential-manager.ps1 -Setup
  powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File scripts/validation/run-nomi-core-live-provider-from-windows-credential-manager.ps1
  powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File scripts/validation/run-nomi-core-live-provider-from-windows-credential-manager.ps1 -Delete
#>

[CmdletBinding()]
param(
  [switch]$Setup,
  [switch]$Delete,
  [string]$TargetName = 'NomiFun/StepFun/LiveProvider'
)

$ErrorActionPreference = 'Stop'

if ($Setup -and $Delete) {
  throw 'Setup and Delete cannot be used together.'
}

if (-not ('NomiFunCredentialManager' -as [type])) {
  Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;

public static class NomiFunCredentialManager
{
    private const uint CRED_TYPE_GENERIC = 1;
    private const uint CRED_PERSIST_LOCAL_MACHINE = 2;

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct NativeCredential
    {
        public uint Flags;
        public uint Type;
        public IntPtr TargetName;
        public IntPtr Comment;
        public System.Runtime.InteropServices.ComTypes.FILETIME LastWritten;
        public uint CredentialBlobSize;
        public IntPtr CredentialBlob;
        public uint Persist;
        public uint AttributeCount;
        public IntPtr Attributes;
        public IntPtr TargetAlias;
        public IntPtr UserName;
    }

    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool CredWrite(ref NativeCredential credential, uint flags);

    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool CredRead(
        string target,
        uint type,
        uint flags,
        out IntPtr credential
    );

    [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool CredDelete(string target, uint type, uint flags);

    [DllImport("advapi32.dll", SetLastError = true)]
    private static extern void CredFree(IntPtr credential);

    public static void Write(string target, string value)
    {
        if (String.IsNullOrWhiteSpace(target))
            throw new ArgumentException("credential target must not be empty");
        if (value == null || value.Length == 0)
            throw new ArgumentException("credential value must not be empty");
        if (value.IndexOf('\r') >= 0 || value.IndexOf('\n') >= 0)
            throw new ArgumentException("credential value must not contain newlines");

        byte[] bytes = Encoding.UTF8.GetBytes(value);
        IntPtr targetPtr = IntPtr.Zero;
        IntPtr userPtr = IntPtr.Zero;
        IntPtr blobPtr = IntPtr.Zero;
        try
        {
            targetPtr = Marshal.StringToCoTaskMemUni(target);
            userPtr = Marshal.StringToCoTaskMemUni("nomifun-live-provider");
            blobPtr = Marshal.AllocHGlobal(bytes.Length);
            Marshal.Copy(bytes, 0, blobPtr, bytes.Length);
            var credential = new NativeCredential
            {
                Type = CRED_TYPE_GENERIC,
                TargetName = targetPtr,
                CredentialBlob = blobPtr,
                CredentialBlobSize = (uint)bytes.Length,
                Persist = CRED_PERSIST_LOCAL_MACHINE,
                UserName = userPtr,
            };
            if (!CredWrite(ref credential, 0))
                throw new Win32Exception(Marshal.GetLastWin32Error(), "CredWrite failed");
        }
        finally
        {
            if (blobPtr != IntPtr.Zero)
            {
                for (int index = 0; index < bytes.Length; index++)
                    Marshal.WriteByte(blobPtr, index, 0);
                Marshal.FreeHGlobal(blobPtr);
            }
            if (targetPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(targetPtr);
            if (userPtr != IntPtr.Zero) Marshal.FreeCoTaskMem(userPtr);
            Array.Clear(bytes, 0, bytes.Length);
        }
    }

    public static string Read(string target)
    {
        IntPtr credentialPtr;
        if (!CredRead(target, CRED_TYPE_GENERIC, 0, out credentialPtr))
        {
            int error = Marshal.GetLastWin32Error();
            if (error == 1168) return null; // ERROR_NOT_FOUND
            throw new Win32Exception(error, "CredRead failed");
        }
        try
        {
            var credential = Marshal.PtrToStructure<NativeCredential>(credentialPtr);
            if (credential.CredentialBlob == IntPtr.Zero ||
                credential.CredentialBlobSize == 0 ||
                credential.CredentialBlobSize > 16 * 1024)
                throw new InvalidOperationException("stored credential has an invalid size");
            byte[] bytes = new byte[credential.CredentialBlobSize];
            Marshal.Copy(credential.CredentialBlob, bytes, 0, bytes.Length);
            try
            {
                return new UTF8Encoding(false, true).GetString(bytes);
            }
            finally
            {
                Array.Clear(bytes, 0, bytes.Length);
            }
        }
        finally
        {
            CredFree(credentialPtr);
        }
    }

    public static void Delete(string target)
    {
        if (CredDelete(target, CRED_TYPE_GENERIC, 0)) return;
        int error = Marshal.GetLastWin32Error();
        if (error == 1168) return; // ERROR_NOT_FOUND
        throw new Win32Exception(error, "CredDelete failed");
    }
}
'@
}

if ($Delete) {
  [NomiFunCredentialManager]::Delete($TargetName)
  Write-Output 'credential_manager_status=deleted'
  exit 0
}

if ($Setup) {
  $secure = Read-Host 'Enter StepFun API key (hidden input)' -AsSecureString
  $pointer = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure)
  try {
    $value = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($pointer)
    [NomiFunCredentialManager]::Write($TargetName, $value)
  }
  finally {
    if ($pointer -ne [IntPtr]::Zero) {
      [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($pointer)
    }
    Remove-Variable value -ErrorAction SilentlyContinue
    Remove-Variable secure -ErrorAction SilentlyContinue
  }
  Write-Output 'credential_manager_status=stored'
}

$credential = [NomiFunCredentialManager]::Read($TargetName)
if ([string]::IsNullOrWhiteSpace($credential)) {
  throw "no credential stored for target '$TargetName'; run with -Setup once"
}

try {
  $env:NOMIFUN_LIVE_STEPFUN_API_KEY = $credential
  & bun run test:nomi-core-live-provider
  $exitCode = $LASTEXITCODE
}
finally {
  Remove-Item Env:NOMIFUN_LIVE_STEPFUN_API_KEY -ErrorAction SilentlyContinue
  Remove-Variable credential -ErrorAction SilentlyContinue
}

exit $exitCode
