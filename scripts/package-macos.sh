#!/bin/sh
set -eu

binary=$1
archive=$2
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
app="$stage/LeetDesk.app"
iconset="$stage/LeetDesk.iconset"

mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$iconset"
cp "$binary" "$app/Contents/MacOS/leetdesk"
chmod 755 "$app/Contents/MacOS/leetdesk"

cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>LeetDesk</string>
  <key>CFBundleDisplayName</key><string>LeetDesk</string>
  <key>CFBundleIdentifier</key><string>io.github.wsplxa.leetdesk</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleExecutable</key><string>leetdesk</string>
  <key>CFBundleIconFile</key><string>LeetDesk.icns</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
EOF

for size in 16 32 128 256 512; do
    sips -z "$size" "$size" assets/leetdesk-icon.png --out "$iconset/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z "$double" "$double" assets/leetdesk-icon.png --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/LeetDesk.icns"
plutil -lint "$app/Contents/Info.plist"
codesign --force --deep --sign - "$app"
ditto -c -k --sequesterRsrc --keepParent "$app" "$archive"
