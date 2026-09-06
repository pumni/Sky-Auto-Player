# scripts/v4_updater_credential_broker.ps1
# Bounded repository helper for Windows Credential Manager generic session credential transport.
# Governed by docs/v4-release-execution-topology.md, docs/v4-updater-key-custody.md, and SECURITY.md.

[CmdletBinding()]
param(
    [Parameter(Mandatory = $false)]
    [ValidateSet("Read", "Delete", "Test")]
    [string]$Action,

    [Parameter(Mandatory = $false)]
    [string]$BrokerTarget
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not [System.OperatingSystem]::IsWindows()) {
    throw "V4 updater credential broker requires Windows platform"
}

$script:CanonicalCredentialTarget = "SkyAutoPlayer/V4UpdaterProduction"

if (-not ([System.Management.Automation.PSTypeName]'SkyAutoPlayer.V4UpdaterCredentialBroker').Type) {
    $csharpSource = @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;

namespace SkyAutoPlayer
{
    public static class V4UpdaterCredentialBroker
    {
        private const uint CRED_TYPE_GENERIC = 1;
        private const uint CRED_PERSIST_SESSION = 1;
        private const int ERROR_NOT_FOUND = 1168;

        [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
        private struct CREDENTIAL_READ
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

        [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
        private struct CREDENTIAL_WRITE
        {
            public uint Flags;
            public uint Type;
            public string TargetName;
            public string Comment;
            public System.Runtime.InteropServices.ComTypes.FILETIME LastWritten;
            public uint CredentialBlobSize;
            public IntPtr CredentialBlob;
            public uint Persist;
            public uint AttributeCount;
            public IntPtr Attributes;
            public string TargetAlias;
            public string UserName;
        }

        [DllImport("Advapi32.dll", EntryPoint = "CredReadW", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern bool CredRead(string target, uint type, uint reservedFlag, out IntPtr credentialPtr);

        [DllImport("Advapi32.dll", EntryPoint = "CredWriteW", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern bool CredWrite(ref CREDENTIAL_WRITE credential, uint flags);

        [DllImport("Advapi32.dll", EntryPoint = "CredDeleteW", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern bool CredDelete(string target, uint type, uint flags);

        [DllImport("Advapi32.dll", SetLastError = true)]
        private static extern void CredFree(IntPtr buffer);

        public static string ReadCredential(string target)
        {
            if (string.IsNullOrWhiteSpace(target))
            {
                throw new ArgumentException("Credential target is required", nameof(target));
            }

            IntPtr credentialPtr;
            if (!CredRead(target, CRED_TYPE_GENERIC, 0, out credentialPtr))
            {
                int error = Marshal.GetLastWin32Error();
                if (error == ERROR_NOT_FOUND)
                {
                    throw new InvalidOperationException("Credential is absent in Windows Credential Manager");
                }
                throw new Win32Exception(error);
            }

            try
            {
                var cred = (CREDENTIAL_READ)Marshal.PtrToStructure(credentialPtr, typeof(CREDENTIAL_READ));
                if (cred.CredentialBlob == IntPtr.Zero || cred.CredentialBlobSize == 0)
                {
                    throw new InvalidOperationException("Credential blob is empty");
                }

                byte[] blob = new byte[cred.CredentialBlobSize];
                Marshal.Copy(cred.CredentialBlob, blob, 0, (int)cred.CredentialBlobSize);

                for (int i = 0; i < (int)cred.CredentialBlobSize; i++)
                {
                    Marshal.WriteByte(cred.CredentialBlob, i, 0);
                }

                string result;
                if (blob.Length >= 2 && blob[1] == 0 && (blob.Length % 2 == 0))
                {
                    result = Encoding.Unicode.GetString(blob).TrimEnd('\0');
                }
                else
                {
                    result = Encoding.UTF8.GetString(blob).TrimEnd('\0');
                }

                Array.Clear(blob, 0, blob.Length);

                if (string.IsNullOrEmpty(result))
                {
                    throw new InvalidOperationException("Credential blob is empty");
                }

                return result;
            }
            finally
            {
                CredFree(credentialPtr);
            }
        }

        public static bool HasNonEmptyCredential(string target)
        {
            if (string.IsNullOrWhiteSpace(target))
            {
                return false;
            }

            IntPtr credentialPtr;
            if (!CredRead(target, CRED_TYPE_GENERIC, 0, out credentialPtr))
            {
                int error = Marshal.GetLastWin32Error();
                if (error == ERROR_NOT_FOUND)
                {
                    return false;
                }
                throw new Win32Exception(error);
            }

            try
            {
                var cred = (CREDENTIAL_READ)Marshal.PtrToStructure(credentialPtr, typeof(CREDENTIAL_READ));
                return cred.CredentialBlob != IntPtr.Zero && cred.CredentialBlobSize > 0;
            }
            finally
            {
                CredFree(credentialPtr);
            }
        }

        public static bool DeleteCredential(string target)
        {
            if (string.IsNullOrWhiteSpace(target))
            {
                return false;
            }

            if (!CredDelete(target, CRED_TYPE_GENERIC, 0))
            {
                int error = Marshal.GetLastWin32Error();
                if (error == ERROR_NOT_FOUND)
                {
                    return false;
                }
                throw new Win32Exception(error);
            }

            return true;
        }

        public static void WriteSessionCredential(string target, string secret)
        {
            if (string.IsNullOrWhiteSpace(target))
            {
                throw new ArgumentException("Credential target is required", nameof(target));
            }
            if (string.IsNullOrEmpty(secret))
            {
                throw new ArgumentException("Credential secret cannot be empty", nameof(secret));
            }

            byte[] blob = Encoding.UTF8.GetBytes(secret);
            WriteRawCredential(target, blob, CRED_PERSIST_SESSION);
            Array.Clear(blob, 0, blob.Length);
        }

        public static void WriteRawCredential(string target, byte[] blob, uint persist)
        {
            if (string.IsNullOrWhiteSpace(target))
            {
                throw new ArgumentException("Credential target is required", nameof(target));
            }

            IntPtr blobPtr = IntPtr.Zero;
            int blobLength = (blob != null) ? blob.Length : 0;

            try
            {
                if (blobLength > 0)
                {
                    blobPtr = Marshal.AllocHGlobal(blobLength);
                    Marshal.Copy(blob, 0, blobPtr, blobLength);
                }

                var cred = new CREDENTIAL_WRITE
                {
                    Flags = 0,
                    Type = CRED_TYPE_GENERIC,
                    TargetName = target,
                    Comment = "Sky Auto Player V4 Updater Production Session Credential",
                    CredentialBlobSize = (uint)blobLength,
                    CredentialBlob = blobPtr,
                    Persist = persist,
                    AttributeCount = 0,
                    Attributes = IntPtr.Zero,
                    TargetAlias = null,
                    UserName = "SkyAutoPlayerOperator"
                };

                if (!CredWrite(ref cred, 0))
                {
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                }
            }
            finally
            {
                if (blobPtr != IntPtr.Zero)
                {
                    for (int i = 0; i < blobLength; i++)
                    {
                        Marshal.WriteByte(blobPtr, i, 0);
                    }
                    Marshal.FreeHGlobal(blobPtr);
                }
            }
        }
    }
}
'@
    Add-Type -TypeDefinition $csharpSource -Language CSharp
}

function Get-V4UpdaterProductionCredential {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $false)]
        [string]$Target = $script:CanonicalCredentialTarget
    )

    if (-not [System.OperatingSystem]::IsWindows()) {
        throw "V4 updater credential broker requires Windows platform"
    }

    try {
        return [SkyAutoPlayer.V4UpdaterCredentialBroker]::ReadCredential($Target)
    } catch {
        throw "V4 updater credential broker: failed to read credential for target $Target - $($_.Exception.Message)"
    }
}

function Remove-V4UpdaterProductionCredential {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $false)]
        [string]$Target = $script:CanonicalCredentialTarget
    )

    if (-not [System.OperatingSystem]::IsWindows()) {
        throw "V4 updater credential broker requires Windows platform"
    }

    try {
        [void][SkyAutoPlayer.V4UpdaterCredentialBroker]::DeleteCredential($Target)
    } catch {
        throw "V4 updater credential broker: failed to delete credential for target $Target - $($_.Exception.Message)"
    }
}

function Test-V4UpdaterProductionCredential {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $false)]
        [string]$Target = $script:CanonicalCredentialTarget
    )

    if (-not [System.OperatingSystem]::IsWindows()) {
        return $false
    }

    try {
        return [SkyAutoPlayer.V4UpdaterCredentialBroker]::HasNonEmptyCredential($Target)
    } catch {
        return $false
    }
}

if ($PSCmdlet.MyInvocation.BoundParameters.ContainsKey("Action")) {
    $targetToUse = if (-not [string]::IsNullOrWhiteSpace($BrokerTarget)) { $BrokerTarget } else { $script:CanonicalCredentialTarget }
    switch ($Action) {
        "Delete" {
            Remove-V4UpdaterProductionCredential -Target $targetToUse
            Write-Host "[PASS] V4 updater session credential deleted"
        }
        "Test" {
            if (Test-V4UpdaterProductionCredential -Target $targetToUse) {
                Write-Host "PRESENT_NONEMPTY"
            } else {
                Write-Host "ABSENT"
            }
        }
        "Read" {
            $null = Get-V4UpdaterProductionCredential -Target $targetToUse
            Write-Host "[PASS] V4 updater credential read successfully"
        }
    }
}
