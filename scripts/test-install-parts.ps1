$ErrorActionPreference = 'Stop'
$T = Join-Path $env:TEMP ("inst-" + [guid]::NewGuid())
New-Item -ItemType Directory -Force "$T\R", "$T\stage\alexandria", "$T\dest" | Out-Null
Add-Type -OutputAssembly "$T\stage\alexandria\alex.exe" -OutputType ConsoleApplication -TypeDefinition 'public static class P { public static int Main(string[] a) { System.Console.WriteLine("alex " + a[0] + " ok"); return 0; } }'
Copy-Item "$T\stage\alexandria\alex.exe" "$T\stage\alexandria\alexandria-app.exe"
$arch = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'arm64' } else { 'x64' }
Compress-Archive "$T\stage\alexandria" "$T\R\alexandria-windows-$arch.zip"
$big = New-Object byte[] 2500000
(New-Object Random 7).NextBytes($big)
[IO.File]::WriteAllBytes("$T\big.bin", $big)
for ($i = 0; $i -lt 3; $i++) {
  $n = [Math]::Min(1000000, 2500000 - $i * 1000000)
  $b = New-Object byte[] $n
  [Array]::Copy($big, $i * 1000000, $b, 0, $n)
  [IO.File]::WriteAllBytes(("$T\R\big.bin.part{0:D2}" -f $i), $b)
}
$base = 'file:///' + ($T -replace '\\', '/') + '/R'
$sha = (Get-FileHash "$T\big.bin" -Algorithm SHA256).Hash.ToLower()
"big.bin 2500000 $sha $base/big.bin.part00 $base/big.bin.part01 $base/big.bin.part02" | Set-Content "$T\R\full-FILES.txt"
Move-Item "$T\R\big.bin.part02" "$T\hold"
$env:ALEXANDRIA_BASE = $base; $env:ALEXANDRIA_DIR = "$T\dest"; $env:ALEXANDRIA_EDITION = 'full'
$script = Join-Path $PSScriptRoot '..\site\install.ps1'
$first = powershell -NoProfile -ExecutionPolicy Bypass -File $script 2>&1 | Out-String
if ($LASTEXITCODE -eq 0) { throw "a missing part must fail the install: $first" }
Move-Item "$T\hold" "$T\R\big.bin.part02"
$out = powershell -NoProfile -ExecutionPolicy Bypass -File $script 2>&1 | Out-String
if ($LASTEXITCODE -ne 0) { throw "second run failed: $out" }
if ($out -match 'part 1 of 3' -or $out -notmatch 'part 3 of 3') { throw "did not resume at part 3: $out" }
if ((Get-FileHash "$T\dest\Alexandria\big.bin" -Algorithm SHA256).Hash.ToLower() -ne $sha) { throw 'joined file is wrong' }
$env:ALEXANDRIA_EDITION = 'bogus'
$bad = powershell -NoProfile -ExecutionPolicy Bypass -File $script 2>&1 | Out-String
if ($LASTEXITCODE -eq 0 -or $bad -notmatch 'must be lite or full') { throw "bad edition accepted: $bad" }
Write-Host 'installer resumes multi-part downloads: ok'
