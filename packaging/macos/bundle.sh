#!/usr/bin/env bash
# Wraps a compiled orion binary in Orion.app and zips it for download.
#
# usage: packaging/macos/bundle.sh <orion binary> <version> <output folder> [label [file...]]
#
# The label names the kind of Mac the binary runs on and ends up in the zip's name.
# It defaults to "macos-universal", for a binary that holds both Apple silicon and
# Intel code. Any files named after it, such as licences, are copied inside the app.
set -euo pipefail

if [ "$#" -lt 3 ]; then
    echo "usage: $0 <orion binary> <version> <output folder> [label [file...]]" >&2
    exit 2
fi
binary="$1"
version="$2"
output="$3"
label="${4:-macos-universal}"
# Whatever follows the label is a file to include.
shift 3
if [ "$#" -gt 0 ]; then
    shift
fi

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
app="$output/Orion.app"

mkdir -p "$app/Contents/MacOS"
cp "$binary" "$app/Contents/MacOS/orion"
sed "s/__VERSION__/$version/g" "$here/Info.plist" > "$app/Contents/Info.plist"
if [ "$#" -gt 0 ]; then
    mkdir -p "$app/Contents/Resources"
    cp "$@" "$app/Contents/Resources/"
fi

# Apple silicon refuses to run code with no signature at all. An ad-hoc signature
# ("-") satisfies that without a paid developer certificate, but macOS will still
# warn on first launch that the developer cannot be verified.
codesign --force --sign - "$app"

# ditto keeps the bundle's structure and signature intact inside the zip. Finder
# metadata is left out: it is not part of the signature and would only add clutter.
ditto -c -k --norsrc --noextattr --keepParent "$app" "$output/Orion-$version-$label.zip"
echo "$output/Orion-$version-$label.zip"
