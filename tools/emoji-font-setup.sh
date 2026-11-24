#!/bin/sh
# SPDX-License-Identifier: MIT
#
# Fetches an emoji font Avestruz can draw in colour, into the applet's own
# fonts: for a system whose colour emoji font it can't draw.
#
# The text system Avestruz is built on draws colour glyphs that are bitmaps
# (CBDT), and not the ones made of gradients (COLR version 1), which is how
# some distributions ship Noto Color Emoji. On those, emoji show in one
# colour, or not at all. This fetches the bitmap build of the same font,
# from the project that makes it (https://github.com/googlefonts/noto-emoji,
# SIL Open Font License 1.1), into
#
#     $XDG_DATA_HOME/cosmic-applet-avestruz/fonts/
#
# which only Avestruz reads: no other program draws anything differently
# for it. To undo it, delete the file.
#
# Run by hand as `just emoji-font-setup`, once. A font that is already
# there is left as it is.

set -eu

say() { printf 'emoji font setup: %s\n' "$*"; }

url=https://github.com/googlefonts/noto-emoji/raw/main/2D/fonts/NotoColorEmoji.ttf

# The fonts are not root's: they are whoever asked. Start over as them.
if [ "$(id -u)" -eq 0 ]; then
    user=${SUDO_USER:-}
    if [ -z "$user" ] || [ "$user" = root ]; then
        say "run as root for nobody in particular, so nobody's fonts were added to. Run 'just emoji-font-setup' as yourself."
        exit 0
    fi
    if command -v runuser >/dev/null 2>&1; then
        exec runuser -u "$user" -- sh "$0"
    fi
    exec sudo -u "$user" -H sh "$0"
fi

dir=${XDG_DATA_HOME:-$HOME/.local/share}/cosmic-applet-avestruz/fonts
font=$dir/NotoColorEmoji.ttf

if [ -s "$font" ]; then
    say "$font is already there. Delete it to fetch it again."
    exit 0
fi

mkdir -p "$dir"
# Fetched beside where it goes, and moved there only once it is whole:
# what the applet finds in its fonts is a font, or nothing.
fetched=$(mktemp "$dir/.fetching.XXXXXX")
trap 'rm -f "$fetched"' EXIT

say "fetching $url"
if command -v curl >/dev/null 2>&1; then
    curl --fail --silent --show-error --location --output "$fetched" "$url"
elif command -v wget >/dev/null 2>&1; then
    wget --quiet --output-document "$fetched" "$url"
else
    say "neither curl nor wget is installed: nothing was fetched."
    exit 1
fi

# What came is the bitmap build if it has the table bitmaps are in.
if ! grep -q CBDT "$fetched"; then
    say "what was fetched is not the bitmap build of the font: nothing was added."
    exit 1
fi

chmod 0644 "$fetched"
mv "$fetched" "$font"
say "added $font"
say "Avestruz reads its fonts when it starts: restart the applet (or the panel) to see emoji in colour."
