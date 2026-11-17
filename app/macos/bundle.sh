#!/bin/sh
# Builds the macOS application bundle from the release binary.
#
# Why this exists: `cargo build --release` produces a bare Mach-O, and macOS hands a bare Mach-O
# to Terminal.app when something asks to "open" it - Terminal.app holds the "Shell" role for
# public.unix-executable, the type every command line program is. So a double click on
# target/release/system-ping in Finder, or an `open` on it, does start the app, but inside a
# terminal window that then sits there for as long as the app runs. A folder that ends in .app and
# carries a Contents/Info.plist (see macos/Info.plist) is a program of its own to the system, and
# for that shape no terminal is involved. The same bundle also carries the file icon, which on
# macOS has no place inside the binary (see the paragraph about the icon in the README).
#
# Nothing in the app changes by this: the state folder is the per-user one on macOS either way
# (platform::statedir -> ~/Library/Application Support/system-ping), so a bundle finds the targets,
# history and window place the bare binary left behind.
#
#   ./macos/bundle.sh              build the program, then the bundle
#   ./macos/bundle.sh --no-build   bundle the binary that is already there
#
# Needs no tool beyond the command line tools: cargo, sips and iconutil for the icon, plutil to
# check the property list, codesign and lsregister as best effort.

set -eu

cd "$(dirname "$0")/.."

binary=target/release/system-ping
out=target/macos/System-Ping.app
icon_src=../assets/icon.png
build=1

for arg in "$@"; do
    case "$arg" in
        --no-build) build=0 ;;
        --help | -h)
            echo "usage: macos/bundle.sh [--no-build]"
            echo "  builds $out from $binary (from cargo build --release unless --no-build)"
            exit 0
            ;;
        *)
            echo "bundle.sh: unknown argument '$arg' (try --help)" >&2
            exit 2
            ;;
    esac
done

if [ "$build" = 1 ]; then
    cargo build --release
fi

if [ ! -x "$binary" ]; then
    echo "bundle.sh: $binary is missing - build it first (or drop --no-build)" >&2
    exit 1
fi

if [ ! -f "$icon_src" ]; then
    echo "bundle.sh: $icon_src is missing" >&2
    exit 1
fi

# The version comes from the manifest, so the bundle never disagrees with the program inside it.
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -1)
if [ -z "$version" ]; then
    echo "bundle.sh: no [package] version in Cargo.toml" >&2
    exit 1
fi

# Always from scratch: a leftover Contents/Resources/icon.icns from an earlier run would be a file
# icon this build did not ask for.
rm -rf "$out"
mkdir -p "$out/Contents/MacOS" "$out/Contents/Resources"

cp "$binary" "$out/Contents/MacOS/system-ping"
chmod 755 "$out/Contents/MacOS/system-ping"

sed "s/@VERSION@/$version/g" macos/Info.plist >"$out/Contents/Info.plist"
if command -v plutil >/dev/null 2>&1; then
    # A property list macOS cannot read means a folder that is not an app at all, and Finder says
    # nothing more useful than "cannot be opened".
    if ! plutil -lint "$out/Contents/Info.plist" >/dev/null; then
        echo "bundle.sh: $out/Contents/Info.plist does not parse" >&2
        exit 1
    fi
fi

# The icon: one PNG per size, in the names iconutil insists on, packed into one .icns. The source
# is 256 px, so every size below it is a real reduction; the sizes above it are left out rather
# than enlarged - macOS asks the display server for 256 px on a Retina Dock tile, which is exactly
# what assets/icon.png has.
iconset=target/macos/AppIcon.iconset
rm -rf "$iconset"
mkdir -p "$iconset"
for spec in 16:icon_16x16.png 32:icon_16x16@2x.png 32:icon_32x32.png 64:icon_32x32@2x.png \
    128:icon_128x128.png 256:icon_128x128@2x.png 256:icon_256x256.png; do
    size=${spec%%:*}
    name=${spec#*:}
    sips -z "$size" "$size" "$icon_src" --out "$iconset/$name" >/dev/null
done
iconutil -c icns "$iconset" -o "$out/Contents/Resources/icon.icns"
rm -rf "$iconset"

# Ad-hoc signature (the "-" identity), so the bundle is one signed thing rather than a folder with
# a signed file in it. Best effort: without it the app still starts on this machine, macOS just has
# nothing to check, and the signature is not what the app needs to send a ping.
if command -v codesign >/dev/null 2>&1; then
    if ! codesign --force --sign - --identifier com.github.ajoe.system-ping "$out" 2>/dev/null; then
        echo "bundle.sh: signing failed, continuing unsigned" >&2
    fi
fi

# Tell LaunchServices about the new bundle now, instead of waiting for Finder to notice it - without
# this the icon in the Dock and in Finder can stay the generic one for a while after a rebuild.
lsregister=/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/LaunchServices.framework/Versions/A/Support/lsregister
if [ -x "$lsregister" ]; then
    "$lsregister" -f "$out" 2>/dev/null || true
fi

touch "$out"
echo "bundled: $out (version $version)"
echo "start:   open $out"
