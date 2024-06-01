Function Test-CommandExists
{
    Param ($command)
    $oldPreference = $ErrorActionPreference
    $ErrorActionPreference = 'stop'
    try {if(Get-Command $command){RETURN $true}}
    Catch {Write-Host “$command does not exist”; RETURN $false}
    Finally {$ErrorActionPreference=$oldPreference}
} #end function test-CommandExists

Function Get-LatestCommitHash
{
   $hash = Invoke-WebRequest https://api.github.com/repos/MegaAntiCheat/MegaAntiCheat-UI/git/refs/heads/main `
   | Select-Object content `
   | ForEach-Object { $_.content } `
   | ConvertFrom-Json `
   | Select-Object -ExpandProperty object `
   | ForEach-Object { $_.sha }
   RETURN $hash
}

if (-Not (Test-CommandExists git)) {
    Write-Error "ERROR: git is not installed! Please install git for Windows."
    Exit
}

$wingetPresent = Test-CommandExists winget
$chocoPresent = Test-CommandExists choco

if (-Not (Test-CommandExists cargo)) {
    if ($chocoPresent) {
        Write-Host "INFO: Installing rust using Chocolatey"
        choco install rust | Out-Null
    } else {
        Write-Host "WARN: checked and could not find 'cargo' installed. You may not be able to compile the program!"
        Write-Host "INFO: You can install rust for windows by downloading and running their installer: https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe"
        Write-Host "INFO: You probably will also need to install the C++ Build tools too: https://visualstudio.microsoft.com/visual-cpp-build-tools/"
        Write-Host "INFO: If you have Chocolatey installed, the script will automatically install the rust toolchain!"
    }
}

$rustupVer = rustup --version 2>&1
$rustupVerRed = ($rustupVer -split "info:")[0]
Write-Host "INFO: rustup version $rustupVerRed installed."

if (-Not (Test-CommandExists npm)) {
    if ($wingetPresent) {
        Write-Host "INFO: Installing npm using Winget and Fast Node Manager"
        winget install Schniz.fnm | Out-Null
        fnm use --install-if-missing 22 | Out-Null
        npm install -g npm@latest | Out-Null
    } elseif ($chocoPresent) {
        Write-Host "INFO: Installing npm using Chocolatey"
        choco install nodejs --version="22.2.0" | Out-Null
        npm install -g npm@latest | Out-Null
    } else {
        Write-Error "ERROR: npm is not installed! Please install npm from https://docs.npmjs.com/downloading-and-installing-node-js-and-npm"
        Write-Error "INFO: If you have Winget or Chocolatey installed, the script will automatically install latest LTS node and npm!"
        Exit
    }
} 

$nodeVer = node --version
$npmVer = npm --version
Write-Host "INFO: node version $nodeVer installed."
Write-Host "INFO: npm version $npmVer installed."

if (-Not (Test-CommandExists pnpm)) {
    Write-Host "INFO: pnpm is not installed, installing it using npm..."
    npm install -g pnpm
} 

$latestGitHash = Get-LatestCommitHash
$compiledHash = "NONE"
$UIFolder = '.\ui\'
if (-Not (Test-Path -Path $UIFolder)) {
    Write-Host "INFO: Creating UI folder to compile into"
    New-Item -Path "." -Name "ui" -ItemType "directory"
}

$hashFilePath = '.\ui\last_commit_hash.txt'
if (Test-Path -Path $hashFilePath) {
    $compiledHash = Get-Content -Path $hashFilePath
}

if ($compiledHash -eq "NONE" -or $compiledHash -ne $latestGitHash) {
    if (Test-Path -Path $UIFolder) {
        Write-Host "INFO: Deleting old UI..."
        Get-ChildItem -Path $UIFolder -Include *.* -Exclude .gitkeep -File -Recurse | ForEach-Object { $_.Delete()}
    }
    

    Write-Host "INFO: Updating UI with latest commits."
    Write-Host "INFO: Cloning from git..."
    git clone --filter=tree:0 https://github.com/MegaAntiCheat/MegaAntiCheat-UI ui_temp >$null 2>&1
    Set-Location -Path ".\ui_temp\" | Out-Null

    Write-Host "INFO: Compiling UI using pnpm."
    npm exec pnpm i | Out-Null
    npm exec pnpm run build
    Copy-Item -Path ".\dist\*" -Destination "..\ui\" -Recurse -Force 
    Set-Location -Path ".." | Out-Null
    New-Item -ItemType File -Name ".\ui\last_commit_hash.txt"
    Set-Content -Path ".\ui\last_commit_hash.txt" -Value $latestGitHash

    Write-Host "INFO: Cleaning up temp files."
    Remove-Item -LiteralPath ".\ui_temp\" -Force -Recurse
} else {
    Write-Host "INFO: Nothing to be done (you already have the latest version!)"
    Write-Host "INFO: Delete the 'last_commit_hash.txt' file in the 'ui' folder to force a rebuild."
}

