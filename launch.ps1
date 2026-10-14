# What the desktop shortcut runs: rebuilds BookTyper if the code changed, then starts it.
# If it's already open, brings that window to the front instead of starting a second copy.

$project = $PSScriptRoot
$exe = Join-Path $project "target\release\book_typer.exe"

$running = Get-Process book_typer -ErrorAction SilentlyContinue | Select-Object -First 1
if ($running) {
    (New-Object -ComObject WScript.Shell).AppActivate($running.Id) | Out-Null
    exit
}

# Takes about a second when nothing changed. If the build fails (say, code
# mid-edit), the last working version still launches; the errors are in the log.
$cargo = Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe"
& $cargo build --release --manifest-path (Join-Path $project "Cargo.toml") *> (Join-Path $project "target\launch-build.log")

Start-Process $exe -WorkingDirectory $project
