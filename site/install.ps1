$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$Version = 'v0.2.0'
$Repo = 'itsthemoon/alexandria'
$Edition = if ($env:ALEXANDRIA_EDITION) { $env:ALEXANDRIA_EDITION } else { 'lite' }
$RootKey = 'afcbc7a3ff47fac00ed3bcca1b2459d2a6a03053f9a0ed7bb572997af69d2f4a'
$Base = if ($env:ALEXANDRIA_BASE) { $env:ALEXANDRIA_BASE } else { "https://github.com/$Repo/releases/download/$Version" }

$AppSha = @{
  x64 = '366436794708b956310afed99a2af14eacc78c5e5f1d6ff00230342535309279'
  arm64 = 'b6d2ee5a54ed5911fcc29bf2af4945a89d193c1666944d2f2ba3df9a8347c39e'
}
$Hosts = @('https://github.com/itsthemoon/alexandria/releases/download/', 'https://download.kiwix.org/zim/', 'https://huggingface.co/LiquidAI/')
$Proto = if ($env:ALEXANDRIA_BASE) { '=https,file' } else { '=https' }
$Reserved = @('CON', 'PRN', 'AUX', 'NUL') + (1..9 | ForEach-Object { "COM$_"; "LPT$_" })

function Say($m) { Write-Host $m -ForegroundColor Cyan }
function Fail($m) { Write-Host "Error: $m" -ForegroundColor Red; throw $m }
function Gb($b) { '{0:N1} GB' -f ($b / 1e9) }
function Get-File($url, $out, [switch]$Resume) {
  $a = @('-fL', '-g', '--proto', $Proto, '--retry', '5', '--retry-delay', '3', '--progress-bar', '-o', $out)
  if ($Resume) { $a += @('-C', '-') }
  & curl.exe @a $url
  if ($LASTEXITCODE -ne 0) { Fail "Download failed: $url. Run the installer again to resume." }
}
function Sha($p) { (Get-FileHash -Algorithm SHA256 -LiteralPath $p).Hash.ToLower() }

if ($Edition -notin @('lite', 'full')) { Fail 'ALEXANDRIA_EDITION must be lite or full.' }
if (-not (Get-Command curl.exe -ErrorAction SilentlyContinue)) { Fail 'curl.exe is missing (needs Windows 10 1803 or later).' }
if (Get-Process -Name alexandria-app -ErrorAction SilentlyContinue) { Fail 'Alexandria is open. Close it, then run the install command again.' }
$Tmp = Join-Path $env:TEMP ("alexandria-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $Tmp | Out-Null

try {
  Get-File "$Base/$Edition-FILES.txt" "$Tmp\files.txt"
  $files = Get-Content "$Tmp\files.txt" | Where-Object { $_.Trim() } | ForEach-Object {
    $p = $_.Trim() -split '\s+'
    $urlsOk = $p.Count -ge 4 -and -not ($p[3..($p.Count - 1)] | Where-Object {
      $u = $_; -not (($Hosts | Where-Object { $u.StartsWith($_) }) -or ($env:ALEXANDRIA_BASE -and $u.StartsWith("$($env:ALEXANDRIA_BASE)/"))) })
    $stem = ($p[0] -split '\.')[0].ToUpper()
    if (-not ($p[0] -cmatch '^[A-Za-z0-9][A-Za-z0-9._-]*$' -and $p[0] -notmatch '\.\.' -and $p[0] -notmatch '[. ]$' -and
              $Reserved -notcontains $stem -and
              $p[1] -match '^[0-9]{1,15}$' -and $p[2] -cmatch '^[0-9a-f]{64}$' -and $urlsOk)) {
      Fail "The file list from the release is malformed (line: $_). Nothing was written."
    }
    [pscustomobject]@{ Name = $p[0]; Bytes = [int64]$p[1]; Sha = $p[2]; Urls = $p[3..($p.Count - 1)] }
  }
  $need = ($files | Measure-Object Bytes -Sum).Sum

  $arch = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64' -or $env:PROCESSOR_ARCHITEW6432 -eq 'ARM64') { 'arm64' } else { 'x64' }
  Say "Installing Alexandria for Windows ($arch)..."
  Get-File "$Base/alexandria-windows-$arch.zip" "$Tmp\app.zip"
  if (-not $env:ALEXANDRIA_BASE -and (Sha "$Tmp\app.zip") -ne $AppSha[$arch]) { Fail 'The app download does not match the checksum in this installer. Nothing was installed.' }
  $AppDir = Join-Path $env:LOCALAPPDATA 'Programs\Alexandria'
  if (Test-Path $AppDir) { Remove-Item -Recurse -Force $AppDir }
  Expand-Archive -Path "$Tmp\app.zip" -DestinationPath "$Tmp\app"
  New-Item -ItemType Directory -Force (Split-Path $AppDir) | Out-Null
  Move-Item "$Tmp\app\alexandria" $AppDir
  $Alex = Join-Path $AppDir 'alex.exe'
  $Exe = Join-Path $AppDir 'alexandria-app.exe'
  $Lnk = Join-Path ([Environment]::GetFolderPath('Programs')) 'Alexandria.lnk'
  $sh = (New-Object -ComObject WScript.Shell).CreateShortcut($Lnk)
  $sh.TargetPath = $Exe; $sh.WorkingDirectory = $AppDir; $sh.Save()


  $default = if ($env:ALEXANDRIA_DIR) { $env:ALEXANDRIA_DIR } else { $env:USERPROFILE }
  $asked = if ($env:ALEXANDRIA_DIR -or -not [Environment]::UserInteractive) { $null } else { Read-Host "Where should the library go? (an 'Alexandria' folder is made there) [$default]" }
  $parent = if ($asked) { $asked } else { $default }
  if (-not $parent) { $parent = $default }
  if (-not (Test-Path $parent -PathType Container)) { Fail "$parent does not exist." }
  $Lib = Join-Path $parent 'Alexandria'
  New-Item -ItemType Directory -Force -Path $Lib | Out-Null

  $have = 0
  foreach ($f in $files) {
    $p = Join-Path $Lib $f.Name
    if ((Test-Path $p) -and (Get-Item $p).Length -eq $f.Bytes) { $have += $f.Bytes }
  }
  $todo = $need - $have
  $index = [int64]($need * 0.25 + 2GB)
  $drive = (Get-Item $Lib).PSDrive
  $free = (Get-PSDrive $drive.Name).Free
  Say "Library: $(Gb $need) total, $(Gb $todo) to download, plus about $(Gb $index) for search. $(Gb $free) free on $($drive.Name):"
  if ($free -lt ($todo + $index + 200MB)) { Fail "Not enough free space. Free up $(Gb ($todo + $index + 200MB - $free)) or pick another drive." }

  $n = 0
  foreach ($f in $files) {
    $n++
    $p = Join-Path $Lib $f.Name
    if ((Test-Path $p) -and (Get-Item $p).Length -eq $f.Bytes -and (Sha $p) -eq $f.Sha) {
      Write-Host "[$n/$($files.Count)] $($f.Name) (already here)"; continue
    }
    Write-Host "[$n/$($files.Count)] $($f.Name) ($(Gb $f.Bytes))"
    foreach ($ext in @('part', 'piece', 'done')) {
      $c = "$p.$ext"
      $item = Get-Item -Force -ErrorAction SilentlyContinue $c
      if ($item -and $item.LinkType) { Remove-Item -Force $c }
    }
    $part = "$p.part"
    if ($f.Urls.Count -eq 1) {
      if ((Test-Path -LiteralPath $part) -and (Get-Item -LiteralPath $part).Length -ge $f.Bytes) { Remove-Item -LiteralPath $part }
      Get-File $f.Urls[0] $part -Resume
    } else {
      $done = "$p.done"; $piece = "$p.piece"; $k = 0; $got = 0
      if (Test-Path $done) {
        $d = (Get-Content $done) -split ' '
        $k = [int]$d[0]; $got = [int64]$d[1]
      }
      if (Test-Path $part) {
        $size = (Get-Item $part).Length
        if ($size -gt $got) {
          $fs = [IO.File]::Open($part, 'Open', 'ReadWrite')
          try { $fs.SetLength($got) } finally { $fs.Close() }
        } elseif ($size -lt $got) {
          $k = 0; $got = 0
          [IO.File]::WriteAllBytes($part, @())
        }
      } else {
        $k = 0; $got = 0
        [IO.File]::WriteAllBytes($part, @())
      }
      for ($i = $k; $i -lt $f.Urls.Count; $i++) {
        Write-Host "  part $($i + 1) of $($f.Urls.Count)"
        Remove-Item $piece -ErrorAction SilentlyContinue
        Get-File $f.Urls[$i] $piece -Resume
        $out = [IO.File]::Open($part, 'Append')
        try { $in = [IO.File]::OpenRead($piece); $in.CopyTo($out); $in.Close() } finally { $out.Close() }
        Set-Content -Path $done -Value "$($i + 1) $((Get-Item $part).Length)"
      }
      Remove-Item $piece -ErrorAction SilentlyContinue
      Remove-Item $done -ErrorAction SilentlyContinue
    }
    if ((Get-Item $part).Length -ne $f.Bytes) { Remove-Item $part; Remove-Item "$p.done" -ErrorAction SilentlyContinue; Fail "$($f.Name) downloaded with the wrong size. Run the installer again to resume." }
    if ((Sha $part) -ne $f.Sha) { Remove-Item $part; Remove-Item "$p.done" -ErrorAction SilentlyContinue; Fail "$($f.Name) failed its checksum. Run the installer again." }
    Move-Item -Force $part $p
  }

  Say 'Checking every file against the release signature (a minute for Lite, longer for a large library)...'
  & $Alex verify $Lib $RootKey | Select-Object -Last 1
  if ($LASTEXITCODE -ne 0) { Fail 'The library did not pass verification.' }

  Say 'Preparing search on this PC (5 to 30 minutes for Lite, a few hours for a large library; it can resume)...'
  & $Alex index-library $Lib $RootKey
  if ($LASTEXITCODE -ne 0) { Fail 'Building search failed. Run the installer again to resume.' }

  $Conf = Join-Path $env:LOCALAPPDATA 'Alexandria'
  New-Item -ItemType Directory -Force -Path $Conf | Out-Null
  Set-Content -Path (Join-Path $Conf 'library-path') -Value $Lib -Encoding UTF8 -NoNewline

  Say 'Done. Alexandria is installed and works offline from now on.'
  Write-Host 'Open it from the Start menu: Alexandria'
  Write-Host "Library: $Lib"
} finally {
  Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
}
