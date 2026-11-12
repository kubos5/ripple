#!/bin/sh
# Rebuilds the macOS 26+ layered app icon (Assets.car) from src-tauri/icons/macos/Ripple.icon.
# Needs Xcode 26 or later. The compiled file is committed, so ordinary builds don't need this.
set -e
cd "$(dirname "$0")/../src-tauri/icons/macos"
out=$(mktemp -d)
xcrun actool "$(pwd)/Ripple.icon" --compile "$out" --output-format human-readable-text --errors --warnings \
  --output-partial-info-plist "$out/partial.plist" --app-icon Ripple --include-all-app-icons \
  --enable-on-demand-resources NO --development-region en --target-device mac \
  --minimum-deployment-target 11.0 --platform macosx
cp "$out/Assets.car" Assets.car
rm -rf "$out"
echo "Updated $(pwd)/Assets.car"
