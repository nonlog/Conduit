[CmdletBinding()]
param(
    [string]$ManifestPath = (Join-Path $PSScriptRoot '..\windows\conduit-ui\ShareTargetPackage\AppxManifest.xml'),
    [Parameter(Mandatory)]
    [string]$OutputPackage,

    [string]$OutputCertificate
)

$ErrorActionPreference = 'Stop'
$manifest = (Resolve-Path -LiteralPath $ManifestPath).Path
$output = [IO.Path]::GetFullPath($OutputPackage)
$certificateOutput = if ([string]::IsNullOrWhiteSpace($OutputCertificate)) {
    [IO.Path]::ChangeExtension($output, '.cer')
} else {
    [IO.Path]::GetFullPath($OutputCertificate)
}

$makeAppx = Get-Command MakeAppx.exe -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source -First 1
if ([string]::IsNullOrWhiteSpace($makeAppx)) {
    $kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
    $makeAppx = Get-ChildItem -LiteralPath $kits -Filter MakeAppx.exe -File -Recurse -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -match '[\\/]x64[\\/]MakeAppx\.exe$' } |
        Sort-Object FullName -Descending |
        Select-Object -ExpandProperty FullName -First 1
}
if ([string]::IsNullOrWhiteSpace($makeAppx) -or -not (Test-Path -LiteralPath $makeAppx -PathType Leaf)) {
    throw 'MakeAppx.exe was not found in the Windows SDK'
}

$signTool = Join-Path ([IO.Path]::GetDirectoryName($makeAppx)) 'signtool.exe'
if (-not (Test-Path -LiteralPath $signTool -PathType Leaf)) {
    throw "signtool.exe was not found beside MakeAppx.exe: $signTool"
}

$stage = Join-Path ([IO.Path]::GetTempPath()) "conduit-share-target-$PID"
try {
    New-Item -ItemType Directory -Force -Path $stage | Out-Null
    Copy-Item -LiteralPath $manifest -Destination (Join-Path $stage 'AppxManifest.xml') -Force
    $outputDir = [IO.Path]::GetDirectoryName($output)
    New-Item -ItemType Directory -Force -Path $outputDir | Out-Null
    Remove-Item -LiteralPath $output -Force -ErrorAction SilentlyContinue

    & $makeAppx pack /d $stage /p $output /nv /o
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $output -PathType Leaf)) {
        throw "MakeAppx failed to create $output"
    }

    # Sparse/external-location packages are expected to be signed. A short-lived self-signed
    # build certificate keeps the private key off the repository and artifacts: CI signs the
    # identity package, exports only the public certificate, then deletes the private key. The
    # installer trusts that exact public certificate for the current user before registration.
    $cert = New-SelfSignedCertificate `
        -Type Custom `
        -Subject 'CN=Conduit' `
        -FriendlyName 'Conduit Share Target Build' `
        -CertStoreLocation 'Cert:\CurrentUser\My' `
        -KeyAlgorithm RSA `
        -KeyLength 2048 `
        -HashAlgorithm SHA256 `
        -KeyUsage DigitalSignature `
        -TextExtension @('2.5.29.37={text}1.3.6.1.5.5.7.3.3')
    try {
        $certificateDir = [IO.Path]::GetDirectoryName($certificateOutput)
        New-Item -ItemType Directory -Force -Path $certificateDir | Out-Null
        Remove-Item -LiteralPath $certificateOutput -Force -ErrorAction SilentlyContinue
        Export-Certificate -Cert $cert -FilePath $certificateOutput -Type CERT | Out-Null

        & $signTool sign /fd SHA256 /sha1 $cert.Thumbprint /s My $output
        if ($LASTEXITCODE -ne 0) { throw "signtool failed to sign $output" }

        # `signtool sign` validates the package structure while producing the signature. Do not add
        # this ephemeral self-signed certificate to a CI runner's Root store just to make `verify`
        # trust it: Windows can surface a root-trust confirmation UI and stall a headless runner.
        # The target installer is the real verification gate; it trusts this exact public leaf in
        # CurrentUser\TrustedPeople before Add-AppxPackage validates and registers the package.
        $signature = Get-AuthenticodeSignature -LiteralPath $output
        if ($null -eq $signature.SignerCertificate -or
            $signature.SignerCertificate.Thumbprint -ne $cert.Thumbprint -or
            $signature.Status -eq [System.Management.Automation.SignatureStatus]::HashMismatch) {
            throw "the signed sparse package does not carry the expected certificate"
        }
    }
    finally {
        Remove-Item -LiteralPath ("Cert:\CurrentUser\My\" + $cert.Thumbprint) -Force -ErrorAction SilentlyContinue
    }

    [pscustomobject]@{
        Package = $output
        Certificate = $certificateOutput
        SizeBytes = (Get-Item -LiteralPath $output).Length
        Sha256 = (Get-FileHash -LiteralPath $output -Algorithm SHA256).Hash.ToLowerInvariant()
        CertificateSha256 = (Get-FileHash -LiteralPath $certificateOutput -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}
finally {
    Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue
}
