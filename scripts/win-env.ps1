# Dot-source before building on Windows:  . .\scripts\win-env.ps1
# Builds whisper.cpp with clang-cl + Ninja. MSVC builds of whisper.cpp ran
# about 10x slower here (20 s vs 1.8 s for a 6 s clip on a Ryzen 5 4600H).

$llvm = "C:\Program Files\LLVM\bin"
$ninja = Get-Command ninja -ErrorAction SilentlyContinue
if (-not $ninja) {
    $found = Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages" -Recurse -Filter ninja.exe -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($found) { $env:Path = "$($found.DirectoryName);$env:Path"; $ninja = Get-Command ninja }
}
if (-not (Test-Path "$llvm\clang-cl.exe")) { throw "Install LLVM: winget install LLVM.LLVM" }
if (-not $ninja) { throw "Install Ninja: winget install Ninja-build.Ninja" }

$env:Path = "$env:USERPROFILE\.cargo\bin;C:\Program Files\CMake\bin;$llvm;$env:Path"
$env:LIBCLANG_PATH = $llvm
$env:CMAKE_GENERATOR = "Ninja"
$env:CMAKE_MAKE_PROGRAM = $ninja.Source
$env:CMAKE_C_COMPILER = "clang-cl"
$env:CMAKE_CXX_COMPILER = "clang-cl"
$env:CXXFLAGS_x86_64_pc_windows_msvc = "/EHsc"
# Local builds: skip whole-program LTO and parallelise codegen. Cuts a rebuild
# from ~5 min to ~1–2 min for a slightly larger exe. CI release builds keep
# the full settings from Cargo.toml.
$env:CARGO_PROFILE_RELEASE_LTO = "false"
$env:CARGO_PROFILE_RELEASE_CODEGEN_UNITS = "16"
Write-Host "Windows build environment ready (clang-cl + Ninja, fast local profile)."
