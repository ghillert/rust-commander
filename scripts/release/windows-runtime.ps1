param(
    [Parameter(Mandatory=$true)][string]$Destination,
    [Parameter(Mandatory=$true)][ValidateSet('x64', 'arm64')][string]$Architecture
)
$ErrorActionPreference = 'Stop'
# App-local redistribution keeps the portable ZIP usable without an installer.
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$component = if ($Architecture -eq 'arm64') { 'ARM64' } else { 'x86.x64' }
$vs = & $vswhere -latest -products '*' -requires "Microsoft.VisualStudio.Component.VC.Tools.$component" -property installationPath
if (-not $vs) { throw 'Visual Studio C++ redistributable tools were not found' }
$runtime = Get-ChildItem "$vs\VC\Redist\MSVC\*\$Architecture\Microsoft.VC*.CRT" -Directory | Sort-Object FullName -Descending | Select-Object -First 1
if (-not $runtime) { throw "The $Architecture Visual C++ runtime was not found" }
# The ARM64 redist also contains ARM64EC-only DLLs for x64 processes.
Add-Type -AssemblyName System.Reflection.Metadata
$machine = if ($Architecture -eq 'arm64') { 'Arm64' } else { 'Amd64' }
foreach ($dll in Get-ChildItem -LiteralPath $runtime.FullName -Filter '*.dll' -File) {
    $reader = [System.Reflection.PortableExecutable.PEReader]::new([IO.File]::OpenRead($dll.FullName))
    try {
        if ($reader.PEHeaders.CoffHeader.Machine.ToString() -eq $machine) {
            Copy-Item -LiteralPath $dll.FullName -Destination $Destination
        }
    } finally {
        $reader.Dispose()
    }
}
@'
Microsoft Visual C++ Runtime
These app-local DLLs are redistributed from the Visual Studio REDIST directory
under Microsoft's Visual Studio distribution terms. They are not part of Rust
Commander's GPL license. See https://visualstudio.microsoft.com/license-terms/
'@ | Set-Content "$Destination\Microsoft-VC-Runtime.txt"
$dumpbin = Get-ChildItem "$vs\VC\Tools\MSVC\*\bin\Host$Architecture\$Architecture\dumpbin.exe" | Sort-Object FullName -Descending | Select-Object -First 1
if (-not $dumpbin) { throw 'dumpbin was not found' }
Get-ChildItem $Destination -Include '*.exe','*.dll' -Recurse | ForEach-Object {
    $dependencies = & $dumpbin.FullName /dependents $_.FullName
    if ($LASTEXITCODE -ne 0) { throw "Cannot inspect $($_.Name)" }
    foreach ($line in $dependencies) {
        if ($line -match '^\s+([A-Za-z0-9_.-]+\.dll)\s*$') {
            $dll = $Matches[1]
            if ($dll -notmatch '^(api-ms-|ext-ms-)' -and
                -not (Test-Path "$Destination\$dll") -and
                ((Test-Path "$($runtime.FullName)\$dll") -or
                 -not (Test-Path "$env:SystemRoot\System32\$dll"))) {
                throw "Unbundled runtime dependency: $dll"
            }
        }
    }
}
