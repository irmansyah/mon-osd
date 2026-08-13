#!/usr/bin/env bash
set -euo pipefail
APP="MonOSD.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp packaging/macos/Info.plist "$APP/Contents/Info.plist"
cp target/release/mon-osd-menubar "$APP/Contents/MacOS/"
codesign --force --deep --sign - "$APP"
echo "Built $APP — run with: open $APP"
