param([Parameter(Mandatory=$true)][string]$Destination)
$ErrorActionPreference = 'Stop'
# App-local redistribution keeps the portable ZIP usable without an installer.
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vs = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $vs) { throw 'Visual Studio C++ redistributable tools were not found' }
$runtime = Get-ChildItem "$vs\VC\Redist\MSVC\*\x64\Microsoft.VC143.CRT" -Directory | Sort-Object FullName -Descending | Select-Object -First 1
if (-not $runtime) { throw 'The x64 Visual C++ runtime was not found' }
Copy-Item "$($runtime.FullName)\*.dll" $Destination
@'
Microsoft Visual C++ Runtime
These app-local DLLs are redistributed from the Visual Studio REDIST directory
under Microsoft's Visual Studio distribution terms. They are not part of Rust
Commander's GPL license. See https://visualstudio.microsoft.com/license-terms/
'@ | Set-Content "$Destination\Microsoft-VC-Runtime.txt"
$dumpbin = Get-ChildItem "$vs\VC\Tools\MSVC\*\bin\Hostx64\x64\dumpbin.exe" | Sort-Object FullName -Descending | Select-Object -First 1
if (-not $dumpbin) { throw 'dumpbin was not found' }
Get-ChildItem $Destination -Include '*.exe','*.dll' -Recurse | ForEach-Object {
    $dependencies = & $dumpbin.FullName /dependents $_.FullName
    if ($LASTEXITCODE -ne 0) { throw "Cannot inspect $($_.Name)" }
    foreach ($line in $dependencies) {
        if ($line -match '^\s+([A-Za-z0-9_.-]+\.dll)\s*$') {
            $dll = $Matches[1]
            if ($dll -notmatch '^(api-ms-|ext-ms-)' -and
                -not (Test-Path "$Destination\$dll") -and
                -not (Test-Path "$env:SystemRoot\System32\$dll")) {
                throw "Unbundled runtime dependency: $dll"
            }
        }
    }
}
