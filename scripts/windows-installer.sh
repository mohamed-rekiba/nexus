#!/usr/bin/env bash
# Makes dist/nexus-windows-x86_64-setup.exe, the Windows installer, from the folder
# scripts/package.sh made (packaging/windows/nexus.iss): `scripts/windows-installer.sh <version>`.
# Runs in Git's bash on Windows with Inno Setup 6, which it installs with Chocolatey when missing.
set -euo pipefail
cd "$(dirname "$0")/.."
version="${1:?usage: $0 <version>}"
iscc="/c/Program Files (x86)/Inno Setup 6/ISCC.exe"
if [ ! -x "$iscc" ]; then
    choco install innosetup --no-progress -y > /dev/null
fi
[ -x "$iscc" ] || { echo "no Inno Setup at $iscc" >&2; exit 1; }
"$iscc" //Q "//DAppVersion=$version" "//DFolder=dist\\nexus-windows-x86_64" "//DOutputDir=..\\..\\dist" \
    packaging\\windows\\nexus.iss
ls -l dist/nexus-windows-x86_64-setup.exe
