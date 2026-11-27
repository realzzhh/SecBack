param([Parameter(Mandatory=$true)][string]$Apk,[Parameter(Mandatory=$true)][string]$Native)
$ErrorActionPreference='Stop'
function Get-SecBackHash([string]$Path) {
  $taskFile=[IO.File]::OpenRead([IO.Path]::GetFullPath($Path))
  $taskHasher=[Security.Cryptography.SHA256]::Create()
  try { return ([BitConverter]::ToString($taskHasher.ComputeHash($taskFile))).Replace('-','') }
  finally {$taskFile.Dispose();$taskHasher.Dispose()}
}
Add-Type -AssemblyName System.IO.Compression.FileSystem
$taskArchive=[IO.Compression.ZipFile]::OpenRead((Resolve-Path -LiteralPath $Apk))
try {
  $taskEntries=@($taskArchive.Entries | Where-Object { $_.FullName -like 'lib/*/*.so' })
  if(@($taskEntries | Where-Object { $_.FullName -notlike 'lib/arm64-v8a/*' }).Count) { throw 'APK contains an unexpected ABI' }
  $taskEntry=$taskArchive.GetEntry('lib/arm64-v8a/libsecback_lib.so')
  if(!$taskEntry){throw 'APK native library missing'}
  $taskStream=$taskEntry.Open()
  try {$taskSha=[Security.Cryptography.SHA256]::Create();$taskNativeHash=([BitConverter]::ToString($taskSha.ComputeHash($taskStream))).Replace('-','')}
  finally {$taskStream.Dispose();$taskSha.Dispose()}
} finally {$taskArchive.Dispose()}
if($taskNativeHash -ne (Get-SecBackHash $Native)){throw 'APK contains stale native code'}
$taskSdk=$env:ANDROID_HOME
if(!$taskSdk){throw 'Set ANDROID_HOME'}
$taskSigner=Get-ChildItem -LiteralPath (Join-Path $taskSdk 'build-tools') -Directory | Sort-Object Name -Descending | Select-Object -First 1
$taskCertOutput=& (Join-Path $taskSigner.FullName 'apksigner.bat') verify --print-certs $Apk
if($LASTEXITCODE){throw 'APK signature verification failed'}
$taskCert=($taskCertOutput | Select-String 'certificate SHA-256 digest:').Line.Split(':',2)[1].Trim().ToUpperInvariant()
if($taskCert -ne 'D291417A79D40EE5EF54CB6D841DDDD8FFA4F2A1B676E76A9F353A0D8986EA48'){throw 'Release signer changed'}
$taskRoot=Split-Path $PSScriptRoot -Parent
$taskMetadata=Get-Content -LiteralPath (Join-Path $taskRoot 'package.json') -Raw | ConvertFrom-Json
$taskVersion=$taskMetadata.version
$taskGitArgs=@('-c',('safe.directory='+$taskRoot.Replace('\','/')),'-c',('core.excludesFile='+ (Join-Path $taskRoot '.git/info/exclude')),'-C',$taskRoot)
$taskCommit=(& git @taskGitArgs rev-parse HEAD 2>$null | Out-String).Trim()
if($LASTEXITCODE){throw 'Cannot identify source commit'}
$taskChanges=@(& git @taskGitArgs status --porcelain --untracked-files=normal 2>$null)
if($LASTEXITCODE){throw 'Cannot identify source status'}
$taskBadging=& (Join-Path $taskSigner.FullName 'aapt.exe') dump badging $Apk
if($LASTEXITCODE){throw 'Cannot read APK metadata'}
$taskPackage=($taskBadging | Select-String '^package:').Line
if($taskPackage -notmatch "name='com.secback.vault'" -or $taskPackage -notmatch ("versionName='"+[regex]::Escape($taskVersion)+"'")){throw 'APK package or version does not match source'}
$taskVersionCode=[int]$taskMetadata.androidVersionCode
if($taskPackage -notmatch ("versionCode='"+$taskVersionCode+"'")){throw 'APK versionCode does not match source'}
if($taskBadging -match '^application-debuggable'){throw 'Release APK is debuggable'}
$taskManifest=@{version=$taskVersion;versionCode=$taskVersionCode;abi='arm64-v8a';apkSha256=(Get-SecBackHash $Apk);nativeSha256=$taskNativeHash;certificateSha256=$taskCert;gitCommit=$taskCommit;sourceDirty=($taskChanges.Count -gt 0);createdUtc=[DateTime]::UtcNow.ToString('o')}
$taskManifest | ConvertTo-Json | Set-Content -LiteralPath ($Apk+'.manifest.json') -Encoding utf8
Write-Output 'APK native library, ABI and release signature verified'
