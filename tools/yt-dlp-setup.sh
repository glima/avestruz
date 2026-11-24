#!/bin/sh
# SPDX-License-Identifier: MIT
#
# Sees that yt-dlp, which Avestruz asks for the stream of a YouTube video,
# can read YouTube when the applet runs it: that its configuration names a
# JavaScript runtime by its full path, and lets it have its solver scripts.
#
# yt-dlp needs a JavaScript runtime for YouTube, and looks for one on PATH.
# A panel applet does not have a shell's PATH, so a runtime that a version
# manager put there is not found: the configuration has to say where it is.
# See https://github.com/yt-dlp/yt-dlp/wiki/EJS
#
# Run by `just install`, and by hand as `just yt-dlp-setup`. It only ever
# adds what is missing: a configuration that already says which runtime to
# use, or where the solver scripts come from, is left as it says. Running
# it again changes nothing.

set -eu

say() { printf 'yt-dlp setup: %s\n' "$*"; }

# Installing is done as root, and the configuration is not root's: it is
# whoever asked. Start over as them.
if [ "$(id -u)" -eq 0 ]; then
    user=${SUDO_USER:-}
    if [ -z "$user" ] || [ "$user" = root ]; then
        say "run as root for nobody in particular, so nobody's yt-dlp was set up. Run 'just yt-dlp-setup' as yourself."
        exit 0
    fi
    if command -v runuser >/dev/null 2>&1; then
        exec runuser -u "$user" -- sh "$0"
    fi
    exec sudo -u "$user" -H sh "$0"
fi

# Where a program may be that the PATH here does not say: this may run with
# the short PATH of sudo.
PATH="$PATH:$HOME/.local/bin"

if ! command -v yt-dlp >/dev/null 2>&1; then
    say "yt-dlp is not installed. YouTube videos will not play until it is; then run 'just yt-dlp-setup'."
    exit 0
fi

# The configuration yt-dlp reads for this user: the first there is of the
# places it looks in, and the first of them if there is none yet.
config_home=${XDG_CONFIG_HOME:-$HOME/.config}
config=$config_home/yt-dlp/config
for known in "$config_home/yt-dlp/config" "$config_home/yt-dlp/config.txt" "$config_home/yt-dlp.conf" \
    "$HOME/yt-dlp.conf" "$HOME/yt-dlp.conf.txt" "$HOME/.yt-dlp/config" "$HOME/.yt-dlp/config.txt"; do
    if [ -f "$known" ]; then
        config=$known
        break
    fi
done

# Whether the configuration already has the option `$1`.
has() { [ -f "$config" ] && grep -Eq "^[[:space:]]*$1([[:space:]=]|\$)" "$config"; }

# A JavaScript runtime, as yt-dlp names it and where it is: the first there
# is of those it can use, in the order it prefers them, each looked for
# where it stays put before where the PATH has it. What a version manager
# puts on the PATH of one shell is followed to where it really is.
runtime() {
    data=${XDG_DATA_HOME:-$HOME/.local/share}
    for candidate in \
        "deno:$HOME/.deno/bin/deno" "deno:$(command -v deno 2>/dev/null || true)" \
        "node:${FNM_DIR:-$data/fnm}/aliases/default/bin/node" "node:$HOME/.volta/bin/node" \
        "node:/usr/bin/node" "node:/usr/local/bin/node" "node:$(command -v node 2>/dev/null || true)" \
        "bun:$HOME/.bun/bin/bun" "bun:$(command -v bun 2>/dev/null || true)" \
        "quickjs:$(command -v qjs 2>/dev/null || true)"; do
        name=${candidate%%:*}
        path=${candidate#*:}
        [ -n "$path" ] && [ -x "$path" ] || continue
        case $path in
            # One shell's own, gone when the shell is: where it leads.
            /run/* | /tmp/*) path=$(readlink -f "$path") ;;
        esac
        printf '%s:%s\n' "$name" "$path"
        return 0
    done
    return 1
}

added=

# Add the line `$2` to the configuration, under the comment `$1`.
add() {
    mkdir -p "$(dirname "$config")"
    if [ -z "$added" ] && [ -s "$config" ]; then
        printf '\n' >>"$config"
    fi
    printf '# %s\n%s\n' "$1" "$2" >>"$config"
    added=yes
    say "added '$2' to $config"
}

if has --js-runtimes; then
    say "$config already says which JavaScript runtime to use."
elif found=$(runtime); then
    add "Added by Avestruz's 'just install': a JavaScript runtime for reading YouTube, by its full path." "--js-runtimes $found"
else
    say "no JavaScript runtime found (deno, node, bun or quickjs). YouTube videos will not play until yt-dlp has one: https://github.com/yt-dlp/yt-dlp/wiki/EJS"
fi

# The scripts that solve what YouTube sets come with some builds of yt-dlp,
# which then says so of itself. Others have to be let fetch them.
if has --remote-components; then
    say "$config already says where yt-dlp's solver scripts come from."
elif yt-dlp --ignore-config -v 2>&1 | grep -q 'yt_dlp_ejs'; then
    say "yt-dlp has its solver scripts with it."
else
    add "Added by Avestruz's 'just install': lets yt-dlp fetch its solver scripts from its own releases on GitHub." "--remote-components ejs:github"
fi

[ -n "$added" ] || say "nothing to change."
