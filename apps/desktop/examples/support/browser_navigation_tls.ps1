# Disposable loopback TLS witness. No certificate store or trust policy edits.
param(
    [Parameter(Mandatory=$true)][string]$CounterPath,
    [Parameter(Mandatory=$true)][string]$StopPath,
    [Parameter(Mandatory=$true)][string]$CleanupPath
)
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Net.Security;
using System.Net.Sockets;
using System.Security.Authentication;
using System.Security.Cryptography.X509Certificates;
using System.Threading;
using System.Threading.Tasks;
public sealed class NomifunTlsPreflightResult {
 public bool CertificateDelivered, UntrustedRoot, HostnameInvalid, Authenticated;
}
public static class NomifunTlsPreflight {
 public static Task<NomifunTlsPreflightResult> Start(int port) {
  return Task.Run(async () => {
   var result=new NomifunTlsPreflightResult();
   using(var tcp=new TcpClient())
   using(var deadline=new CancellationTokenSource(5000)) {
    await tcp.ConnectAsync("127.0.0.1",port,deadline.Token);
    using(var ssl=new SslStream(tcp.GetStream(),false)) {
     var options=new SslClientAuthenticationOptions {
      TargetHost="localhost", EnabledSslProtocols=SslProtocols.Tls12
     };
     options.RemoteCertificateValidationCallback=(sender,certificate,chain,errors)=>{
      result.CertificateDelivered=certificate!=null;
      result.HostnameInvalid=(errors & SslPolicyErrors.RemoteCertificateNameMismatch)!=0;
      if(chain!=null) foreach(var state in chain.ChainStatus)
       if((state.Status & X509ChainStatusFlags.UntrustedRoot)!=0) result.UntrustedRoot=true;
      // Preserve the normal policy. This client must reject the self-signed certificate.
      return errors==SslPolicyErrors.None;
     };
     try {
      await ssl.AuthenticateAsClientAsync(options,deadline.Token);
      result.Authenticated=true;
     } catch(AuthenticationException) { }
     catch(System.IO.IOException) { }
    }
   }
   return result;
  });
 }
}
'@
$rsa = [System.Security.Cryptography.RSA]::Create(2048)
$certificate = $null
$listener = $null
$ownedKeyPath = $null
$ownedKeyInitiallyPresent = $false
$cleanup = @{certificate_disposed=$false;rsa_disposed=$false;temporary_native_key_removed=$false}
try {
    $request = [System.Security.Cryptography.X509Certificates.CertificateRequest]::new(
        'CN=localhost', $rsa, [System.Security.Cryptography.HashAlgorithmName]::SHA256,
        [System.Security.Cryptography.RSASignaturePadding]::Pkcs1)
    $san = [System.Security.Cryptography.X509Certificates.SubjectAlternativeNameBuilder]::new()
    $san.AddDnsName('localhost')
    $san.AddIpAddress([System.Net.IPAddress]::Loopback)
    [void]$request.CertificateExtensions.Add($san.Build())
    [void]$request.CertificateExtensions.Add([System.Security.Cryptography.X509Certificates.X509BasicConstraintsExtension]::new($false,$false,0,$true))
    [void]$request.CertificateExtensions.Add([System.Security.Cryptography.X509Certificates.X509KeyUsageExtension]::new(
        [System.Security.Cryptography.X509Certificates.X509KeyUsageFlags]::DigitalSignature,$true))
    $usage = [System.Security.Cryptography.OidCollection]::new()
    [void]$usage.Add([System.Security.Cryptography.Oid]::new('1.3.6.1.5.5.7.3.1'))
    [void]$request.CertificateExtensions.Add([System.Security.Cryptography.X509Certificates.X509EnhancedKeyUsageExtension]::new($usage,$true))
    $ephemeral = $request.CreateSelfSigned([System.DateTimeOffset]::UtcNow.AddMinutes(-1),[System.DateTimeOffset]::UtcNow.AddDays(1))
    $password = [System.Guid]::NewGuid().ToString('N')
    $bytes = $ephemeral.Export([System.Security.Cryptography.X509Certificates.X509ContentType]::Pfx,$password)
    try {
        # Schannel rejects the direct ephemeral key with SEC_E_NO_CREDENTIALS.
        # DefaultKeySet imports a temporary native key; do not use PersistKeySet.
        $certificate = [System.Security.Cryptography.X509Certificates.X509Certificate2]::new(
            $bytes,$password,[System.Security.Cryptography.X509Certificates.X509KeyStorageFlags]::DefaultKeySet)
    } finally {
        [System.Array]::Clear($bytes)
        $password = $null
        $ephemeral.Dispose()
    }
    $nativeKey = [System.Security.Cryptography.X509Certificates.RSACertificateExtensions]::GetRSAPrivateKey($certificate)
    try {
        if ($nativeKey -isnot [System.Security.Cryptography.RSACng] -or $nativeKey.Key.IsMachineKey -or $nativeKey.Key.IsEphemeral) {
            throw 'Controlled TLS temporary native key ownership unavailable'
        }
        $uniqueName = $nativeKey.Key.UniqueName
        if ($uniqueName -notmatch '^[a-zA-Z0-9_-]+$') { throw 'Controlled TLS key name malformed' }
        $keyRoot = [System.IO.Path]::GetFullPath((Join-Path $env:APPDATA 'Microsoft/Crypto/Keys'))
        $ownedKeyPath = [System.IO.Path]::GetFullPath((Join-Path $keyRoot $uniqueName))
        if (-not $ownedKeyPath.StartsWith($keyRoot+[System.IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)) {
            throw 'Controlled TLS key escaped expected container'
        }
        # Read only the exact freshly imported key. Never scan or delete user keys.
        $ownedKeyInitiallyPresent = [System.IO.File]::Exists($ownedKeyPath)
        if (-not $ownedKeyInitiallyPresent) { throw 'Controlled TLS exact key witness unavailable' }
    } finally { $nativeKey.Dispose() }

    $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback,0)
    $listener.Start()
    $helloCount = 0
    $httpRequests = 0
    $httpResponses = 0
    $authenticated = 0
    $noCredentials = 0
    $lastErrors = @()
    $preflight = $null
    $preflightPending = $true
    $preflightTask = [NomifunTlsPreflight]::Start($listener.LocalEndpoint.Port)
    function Save-Witness {
        $record=@{
            tls_client_hellos=$helloCount;http_requests=$httpRequests;completed_http_documents=$httpResponses
            server_authenticated_connections=$authenticated;server_no_credentials_failures=$noCredentials
            last_safe_errors=$lastErrors;certificate_has_private_key=$certificate.HasPrivateKey
            preflight_certificate_delivered=$preflight.CertificateDelivered;preflight_untrusted_root=$preflight.UntrustedRoot
            preflight_hostname_invalid=$preflight.HostnameInvalid;preflight_client_authenticated=$preflight.Authenticated
            preflight_excluded_from_browser_counts=$true;temporary_native_key_import=$true
            certificate_file_persisted=$false;trust_store_modified=$false;browser_certificate_policy_modified=$false
            helper='existing_powershell_dotnet'
        }
        $next=$CounterPath+'.next'
        [System.IO.File]::WriteAllText($next,($record | ConvertTo-Json -Depth 8 -Compress))
        [System.IO.File]::Move($next,$CounterPath,$true)
    }
    while (-not [System.IO.File]::Exists($StopPath)) {
        $accept=$listener.AcceptTcpClientAsync()
        while (-not $accept.Wait(100)) {
            if ([System.IO.File]::Exists($StopPath)) { break }
        }
        if ([System.IO.File]::Exists($StopPath)) { break }
        $client=$accept.GetAwaiter().GetResult()
        $ssl=$null
        $deadline=$null
        $lastErrors=@()
        try {
            $client.ReceiveTimeout=5000
            $client.SendTimeout=5000
            $prefix=[byte[]]::new(3)
            $read=$client.Client.Receive($prefix,0,3,[System.Net.Sockets.SocketFlags]::Peek)
            if ($read -ne 3 -or $prefix[0] -ne 22 -or $prefix[1] -ne 3) { continue }
            if (-not $preflightPending) { $helloCount++ }
            $ssl=[System.Net.Security.SslStream]::new($client.GetStream(),$false)
            $options=[System.Net.Security.SslServerAuthenticationOptions]::new()
            $options.ServerCertificate=$certificate
            $options.EnabledSslProtocols=[System.Security.Authentication.SslProtocols]::Tls12
            $options.ClientCertificateRequired=$false
            $deadline=[System.Threading.CancellationTokenSource]::new(5000)
            [void]$ssl.AuthenticateAsServerAsync($options,$deadline.Token).GetAwaiter().GetResult()
            if (-not $preflightPending) { $authenticated++ }
            # An authenticated server can finish before the client's trust rejection.
            # Count real HTTP only after a request arrives; never write unsolicited HTTP.
            $requestBytes=[byte[]]::new(1024)
            $requestLength=$ssl.ReadAsync($requestBytes,0,$requestBytes.Length,$deadline.Token).GetAwaiter().GetResult()
            $requestText=[System.Text.Encoding]::ASCII.GetString($requestBytes,0,$requestLength)
            if (-not $requestText.StartsWith('GET /native-tls-fixture HTTP/1.1',[StringComparison]::Ordinal)) { continue }
            if (-not $preflightPending) { $httpRequests++ }
            $body='UNTRUSTED_CERTIFICATE_MUST_NOT_RENDER'
            $crlf=[string][char]13+[char]10
            $response="HTTP/1.1 200 OK"+$crlf+"Content-Type: text/plain"+$crlf+"Content-Length: "+$body.Length+$crlf+"Connection: close"+$crlf+$crlf+$body
            $responseBytes=[System.Text.Encoding]::ASCII.GetBytes($response)
            $ssl.Write($responseBytes,0,$responseBytes.Length)
            $ssl.Flush()
            if (-not $preflightPending) { $httpResponses++ }
        } catch {
            $tlsException=$_.Exception
            $depth=0
            while ($tlsException -and $depth -lt 5) {
                $type=$tlsException.GetType().FullName
                if ($type -notin @('System.Management.Automation.MethodInvocationException',
                    'System.Security.Authentication.AuthenticationException','System.ComponentModel.Win32Exception',
                    'System.IO.IOException','System.Net.Sockets.SocketException','System.Threading.Tasks.TaskCanceledException',
                    'System.OperationCanceledException')) { $type='other_exception' }
                $code=if($tlsException -is [System.ComponentModel.Win32Exception]){$tlsException.NativeErrorCode}else{$null}
                $lastErrors+=@{type=$type;hresult=$tlsException.HResult;native_error=$code}
                if (-not $preflightPending -and $code -eq -2146893042) { $noCredentials++ }
                $tlsException=$tlsException.InnerException
                $depth++
            }
        } finally {
            if ($deadline) { $deadline.Dispose() }
            if ($ssl) { $ssl.Dispose() }
            $client.Dispose()
            if ($preflightPending) {
                if (-not $preflightTask.Wait(6000)) { throw 'Controlled TLS preflight deadline' }
                $preflight=$preflightTask.GetAwaiter().GetResult()
                if (-not $preflight.CertificateDelivered -or -not $preflight.UntrustedRoot -or $preflight.HostnameInvalid -or $preflight.Authenticated) {
                    throw 'Controlled TLS certificate delivery and rejection preflight failed'
                }
                $preflightPending=$false
                $lastErrors=@()
                Save-Witness
                [System.Console]::Out.WriteLine('TLS_FIXTURE_READY:'+$listener.LocalEndpoint.Port)
                [System.Console]::Out.Flush()
            } else { Save-Witness }
        }
    }
} finally {
    if ($listener) { $listener.Stop() }
    if ($certificate) {
        $certificate.Dispose()
        $cleanup.certificate_disposed=$true
    }
    $rsa.Dispose()
    $cleanup.rsa_disposed=$true
    $cleanup.temporary_native_key_removed=($ownedKeyInitiallyPresent -and -not [System.IO.File]::Exists($ownedKeyPath))
    [System.IO.File]::WriteAllText($CleanupPath,($cleanup | ConvertTo-Json -Compress))
}
if (-not $cleanup.temporary_native_key_removed) { exit 2 }
