#!/bin/bash
# items.id=720 (THROWAWAY): assemble target/bundle/spike720.app from the release build and the CEF framework.
#   tools/bundle-mac.sh [adhoc|hardened]
# adhoc    = ad-hoc signature, no hardened runtime (default; what the spike runs).
# hardened = ad-hoc signature WITH hardened runtime + tools/entitlements.plist (signing check: does the production
#            recipe launch with the sandbox ON?). No Apple Developer ID / notarization is used or available.
# Layout and Info.plist keys follow cef-rs's cef/src/build_util/mac.rs (151.1.0) and CEF's GeneralUsage wiki.
set -euo pipefail
cd "$(dirname "$0")/.."
SIGN="${1:-adhoc}"
NAME=spike720
ID=ai.quietrabbit.spike720
OUT=target/bundle
APP="$OUT/$NAME.app"
FW_SRC=$(find .cef-cache -maxdepth 4 -name 'Chromium Embedded Framework.framework' -type d | head -1)
[ -n "$FW_SRC" ] || { echo "CEF framework not found under .cef-cache (run the build first)"; exit 1; }
for f in target/release/spike720 target/release/spike720_helper; do [ -x "$f" ] || { echo "missing $f (cargo build --release --bins)"; exit 1; }; done

plist() { # exec id ui_element(0|1) main(0|1)
  local exe="$1" id="$2" ui="$3" main="$4"
  cat <<PL
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>$exe</string>
  <key>CFBundleDisplayName</key><string>$exe</string>
  <key>CFBundleExecutable</key><string>$exe</string>
  <key>CFBundleIdentifier</key><string>$id</string>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleSignature</key><string>????</string>
  <key>CFBundleVersion</key><string>0.0.1</string>
  <key>CFBundleShortVersionString</key><string>0.0.1</string>
  <key>LSEnvironment</key><dict><key>MallocNanoZone</key><string>0</string></dict>
  <key>LSFileQuarantineEnabled</key><true/>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
PL
  if [ "$ui" = 1 ]; then echo "  <key>LSUIElement</key><string>1</string>"; fi
  if [ "$main" = 1 ]; then
    echo "  <key>NSPrincipalClass</key><string>NSApplication</string>"
    echo "  <key>NSHighResolutionCapable</key><true/>"
  fi
  echo "</dict></plist>"
}

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$APP/Contents/Frameworks"
plist "$NAME" "$ID" 0 1 > "$APP/Contents/Info.plist"
cp target/release/spike720 "$APP/Contents/MacOS/$NAME"
cp -R "$FW_SRC" "$APP/Contents/Frameworks/"
for h in "Helper" "Helper (GPU)" "Helper (Renderer)" "Helper (Plugin)" "Helper (Alerts)"; do
  hn="$NAME $h"
  hid="$ID.$(echo "$h" | tr -d ' ()' | tr 'A-Z' 'a-z')"
  d="$APP/Contents/Frameworks/$hn.app/Contents"
  mkdir -p "$d/MacOS"
  plist "$hn" "$hid" 1 0 > "$d/Info.plist"
  cp target/release/spike720_helper "$d/MacOS/$hn"
done

cs() { # sign one path; hardened adds runtime + entitlements
  if [ "$SIGN" = hardened ]; then
    codesign --force --timestamp=none --options runtime --entitlements tools/entitlements.plist --sign - "$1"
  else
    codesign --force --timestamp=none --sign - "$1"
  fi
}
# inside-out: framework libraries, framework, helper apps, main app
for l in "$APP"/Contents/Frameworks/Chromium\ Embedded\ Framework.framework/Libraries/*.dylib; do cs "$l"; done
cs "$APP/Contents/Frameworks/Chromium Embedded Framework.framework"
for h in "$APP"/Contents/Frameworks/*Helper*.app; do cs "$h"; done
cs "$APP"
echo "signing mode: $SIGN"
codesign --verify --deep --strict --verbose=2 "$APP" 2>&1 | tail -3
codesign -dv --verbose=2 "$APP" 2>&1 | grep -E 'Identifier|flags|Signature|Authority|TeamIdentifier' || true
echo "bundle: $APP ($(du -sh "$APP" | cut -f1))"
