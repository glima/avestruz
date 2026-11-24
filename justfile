name := 'cosmic-applet-avestruz'
appid := 'io.github.cosmic-applet-avestruz'
rootdir := ''
prefix := '/usr'

# Installation paths

base-dir := absolute_path(clean(rootdir / prefix))
cargo-target-dir := env('CARGO_TARGET_DIR', 'target')
bloat-target := cargo-target-dir / 'release-bloat' / name
bin-dst := base-dir / 'bin' / name
desktop-dst := base-dir / 'share' / 'applications' / appid + '.desktop'
metainfo-dst := base-dir / 'share' / 'metainfo' / appid + '.metainfo.xml'
icon-dst := base-dir / 'share' / 'icons' / 'hicolor' / 'scalable' / 'apps' / appid + '.svg'
icon-symbolic-dst := base-dir / 'share' / 'icons' / 'hicolor' / 'symbolic' / 'apps' / appid + '-symbolic.svg'
icon-scalable-symbolic-dst := base-dir / 'share' / 'icons' / 'hicolor' / 'scalable' / 'apps' / appid + '-symbolic.svg'

default: build-release

clean:
    rm -rf {{ coverage-dir }}
    cargo clean

# Compiles with debug profile
build-debug *args:
    cargo build {{ args }}

# Compiles with release profile
build-release *args: (build-debug '--release' args)

# Runs a formatting check, clippy check, unused dependency check, commit-message check, licence check, and security audit
check *args:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Checking formatting..."
    cargo fmt --all -- --check
    cargo clippy --all-features {{ args }} -- -W dead_code -D warnings
    # Clippy above deliberately omits --all-targets: the production lint set
    # (unwrap_used, indexing_slicing, …) is wrong for test code. That leaves
    # test code uncompiled, so build it here.
    echo "Checking that tests compile..."
    cargo test --no-run --all-features
    echo "Checking for unused dependencies..."
    if command -v cargo >/dev/null 2>&1 && cargo --list | grep -q machete; then
        cargo machete || exit 1
    else
        echo "cargo-machete not found, skipping unused dependency check (install with: cargo install cargo-machete)"
    fi
    echo "Checking commit messages..."
    if command -v cog >/dev/null 2>&1; then
        # Conventional Commits, from the last tag forward. Types beyond the
        # defaults are declared in cog.toml, and each one has a section in
        # cliff.toml, so a subject that passes here cannot fall out of the
        # release notes. Until there is a tag, cog has nowhere to start
        # from and says so as an error: the whole history is checked.
        if [ -n "$(git tag --list 'v[0-9]*')" ]; then
            cog check --from-latest-tag || exit 1
        else
            cog check || exit 1
        fi
    else
        echo "cocogitto not found, skipping commit-message check (install with: cargo install cocogitto)"
    fi
    echo "Checking that the documentation builds..."
    just doc
    echo "Checking the diagrams in the documentation..."
    just mermaid-check
    echo "Checking dependency licences..."
    if command -v cargo-deny >/dev/null 2>&1; then
        # Allow list and exceptions live in deny.toml. The project is MIT, so
        # this gates what a new dependency may drag in — the licence of a
        # transitive crate is as binding as a direct one.
        cargo deny check licenses --hide-inclusion-graph || exit 1
    else
        echo "cargo-deny not found, skipping licence check (install with: cargo install cargo-deny)"
    fi
    echo "Running cargo audit for security vulnerabilities..."
    if command -v cargo-audit >/dev/null 2>&1; then
        # A vulnerability fails the check. What is ignored below is warned
        # of, not vulnerable (unmaintained, or unsound in a way not reached
        # here), comes in under a dependency that asks for that version,
        # and can't be moved from this side.
        cargo audit \
            --ignore RUSTSEC-2024-0436 `# paste (unmaintained) — via metal (macOS-only) → wgpu-hal → wgpu → cryoglyph → iced_wgpu → libcosmic` \
            --ignore RUSTSEC-2026-0186 `# memmap2 0.8 (unsound) — via xkbcommon 0.7 → iced_winit → libcosmic. Everything else here is on the unaffected 0.9; clears when iced_winit moves to xkbcommon 0.8 or later, which ask for memmap2 ^0.9` \
            --ignore RUSTSEC-2026-0192 `# ttf-parser (unmaintained) — via fontdb, owned_ttf_parser and rustybuzz, all under libcosmic` \
            --ignore RUSTSEC-2026-0206 `# rustybuzz (unmaintained) — via usvg → resvg → iced_tiny_skia → iced → libcosmic` \
            --ignore RUSTSEC-2026-0253 `# lru 0.16 (unsound) — via cryoglyph → iced_wgpu → libcosmic, which asks for lru ^0.16; the fix is in 0.18.2, which everything else here is on` \
            --ignore RUSTSEC-2026-0370 `# zvariant 5 (unmaintained) — under zbus 5, which accesskit_unix, atspi and ashpd ask for, all under libcosmic` \
            --ignore RUSTSEC-2026-0371 `# zbus_names 4 (unmaintained) — the same zbus 5` \
            --ignore RUSTSEC-2026-0372 `# zvariant_derive 5 (unmaintained) — the same zbus 5` \
            --ignore RUSTSEC-2026-0374 `# basic-toml (unmaintained) — via i18n-config, under the proc macros i18n-embed-fl and i18n-embed-impl: it reads i18n.toml when the program is built, and is no part of it` \
            --ignore RUSTSEC-2026-0543 `# chrono (unmaintained) — ours: tracing-subscriber's "chrono" feature, which puts the local time on a log line. Its other way to local time (the "time" crate) can't tell the local offset once there are threads` \
            --ignore RUSTSEC-2026-0574 `# heapless 0.8 (unsound; this and the eight below) — a dependency of faster-hex 0.10, under nostr, which uses it only without its "alloc" feature; nostr turns "alloc" on, so none of it is called. Clears when nostr moves to faster-hex 2, which asks for the fixed heapless ^0.9.4` \
            --ignore RUSTSEC-2026-0575 \
            --ignore RUSTSEC-2026-0576 \
            --ignore RUSTSEC-2026-0577 \
            --ignore RUSTSEC-2026-0578 \
            --ignore RUSTSEC-2026-0579 \
            --ignore RUSTSEC-2026-0580 \
            --ignore RUSTSEC-2026-0581 \
            --ignore RUSTSEC-2026-0582
    else
        echo "cargo-audit not found, skipping security audit (install with: cargo install cargo-audit)"
    fi

# Reformat the whole workspace (fixes what `just check`'s rustfmt gate reports)
fmt:
    cargo fmt --all

# Run tests
test *args:
    #!/usr/bin/env bash
    set -euo pipefail
    if command -v cargo-nextest >/dev/null 2>&1; then
        cargo nextest run --no-fail-fast --status-level=skip {{ args }}
    else
        echo "cargo-nextest not found, falling back to cargo test"
        cargo test {{ args }}
    fi

# Code statistics via tokei
stats:
    #!/usr/bin/env bash
    set -euo pipefail
    if ! command -v tokei >/dev/null 2>&1; then
        echo "Error: tokei not found. Install with: cargo install tokei"
        exit 1
    fi
    tokei .

# Fuzz-test parsing targets (requires nightly: rustup toolchain install nightly)
# Usage: just fuzz                 — run all targets for 60s each
#        just fuzz note_content    — run a single target

# just fuzz note_content 0 — run until interrupted (Ctrl-C)
fuzz target="" duration="60":
    #!/usr/bin/env bash
    set -euo pipefail
    if ! command -v cargo-fuzz >/dev/null 2>&1; then
        echo "Error: cargo-fuzz not found. Install with: cargo install cargo-fuzz"
        exit 1
    fi
    if [ -n "{{ target }}" ]; then
        echo "═══ Fuzzing: {{ target }} ═══"
        if [ "{{ duration }}" = "0" ]; then
            cargo +nightly fuzz run "fuzz_{{ target }}"
        else
            cargo +nightly fuzz run "fuzz_{{ target }}" -- -max_total_time="{{ duration }}"
        fi
    else
        for t in $(cargo +nightly fuzz list 2>/dev/null); do
            echo ""
            echo "═══ Fuzzing: ${t} ({{ duration }}s) ═══"
            cargo +nightly fuzz run "$t" -- -max_total_time="{{ duration }}"
        done
        echo ""
        echo "All fuzz targets passed ✓"
    fi

# Coverage directory

coverage-dir := 'coverage'

# Run coverage analysis (HTML + LCOV) with cargo-llvm-cov
coverage:
    #!/usr/bin/env bash
    set -euo pipefail
    if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
        echo "Error: cargo-llvm-cov not found. Install with: cargo install cargo-llvm-cov"
        exit 1
    fi
    THREADS=$(( $(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4) / 2 ))
    [ "$THREADS" -lt 1 ] && THREADS=1
    rm -rf {{ coverage-dir }}
    mkdir -p {{ coverage-dir }}
    echo "Generating HTML coverage report"
    cargo llvm-cov \
        --html \
        --ignore-filename-regex '/tests?/|/target/' \
        -- --test-threads="$THREADS"
    if [ -d target/llvm-cov/html ]; then
        cp -r target/llvm-cov/html {{ coverage-dir }}/html
    fi
    echo "Generating LCOV report"
    cargo llvm-cov \
        --no-clean \
        --lcov --output-path {{ coverage-dir }}/lcov.info \
        --ignore-filename-regex '/tests?/|/target/' \
        --summary-only \
        -- --test-threads="$THREADS"
    echo ""
    echo "Coverage reports:"
    echo "  HTML: {{ coverage-dir }}/html/index.html"
    echo "  LCOV: {{ coverage-dir }}/lcov.info"
    if command -v xdg-open >/dev/null 2>&1; then
        xdg-open {{ coverage-dir }}/html/index.html >/dev/null 2>&1 || true
    fi

# Print a text-only coverage summary (no HTML output)
coverage-summary:
    #!/usr/bin/env bash
    set -euo pipefail
    if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
        echo "Error: cargo-llvm-cov not found. Install with: cargo install cargo-llvm-cov"
        exit 1
    fi
    THREADS=$(( $(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4) / 2 ))
    [ "$THREADS" -lt 1 ] && THREADS=1
    cargo llvm-cov --no-report \
        -- --test-threads="$THREADS"
    cargo llvm-cov report --summary-only

# Analyze binary size by crate and function using cargo-bloat
bloat-check:
    #!/usr/bin/env bash
    set -euo pipefail
    if ! command -v cargo-bloat >/dev/null 2>&1; then
        echo "Error: cargo-bloat not found. Install with: cargo install cargo-bloat"
        exit 1
    fi
    echo ""
    echo "Building with release-bloat profile (preserves symbols)"
    cargo build --profile release-bloat
    echo ""
    echo "Binary Size Overview"
    echo "Analysis binary size (with symbols):"
    ls -lh {{ bloat-target }} | awk '{print "  " $5}'
    echo "Stripped size (production equivalent):"
    TEMP_STRIPPED=$(mktemp)
    cp {{ bloat-target }} "$TEMP_STRIPPED"
    strip "$TEMP_STRIPPED"
    ls -lh "$TEMP_STRIPPED" | awk '{print "  " $5}'
    rm "$TEMP_STRIPPED"
    echo ""
    echo "Section breakdown:"
    size {{ bloat-target }}
    echo ""
    echo "Top 30 Crates by Size"
    cargo bloat --profile release-bloat --crates -n 30
    echo ""
    echo "Top 20 Functions by Size"
    cargo bloat --profile release-bloat -n 20
    echo ""
    echo "Tip: Run 'just build-release' to create production binary with symbols stripped"

# What rustdoc is told, building the documentation: to put the script that
# draws the diagrams in every page's head (see docs/rustdoc-header.html, named
# from here, where cargo runs rustdoc), and to take a link that leads nowhere
# for an error.
rustdocflags := '--html-in-header docs/rustdoc-header.html -D warnings'

# Build documentation (including private items), with its diagrams drawn
doc *args:
    RUSTDOCFLAGS='{{ rustdocflags }}' cargo doc --document-private-items --no-deps {{ args }}

# Build documentation and open in browser
doc-open: (doc '--open')

# Assemble the project website (site/ plus the icon) into target/site,
# exactly as the Pages workflow publishes it: the scene and three.js packed
# into one file, and the download table filled from the latest GitHub
# release when `gh` can reach one. Needs npm. Preview with
# `python3 -m http.server --directory target/site 8000`.
site out=(cargo-target-dir / 'site'):
    #!/usr/bin/env bash
    set -euo pipefail
    npm ci --prefix site --omit=dev --ignore-scripts --no-audit --no-fund --silent
    rm -rf "{{ out }}"
    mkdir -p "{{ out }}/assets" "{{ out }}/scene"
    cp site/index.html site/style.css "{{ out }}/"
    cp site/scene/boot.js "{{ out }}/scene/"
    site/node_modules/.bin/esbuild site/scene/world.js --bundle --format=esm --minify \
        --target=es2022 --legal-comments=eof --log-level=warning --outfile="{{ out }}/scene/world.js"
    cp -r site/assets/models site/assets/tex site/assets/still.jpg "{{ out }}/assets/"
    cp resources/icon.svg "{{ out }}/assets/"
    { gh release view --repo glima/avestruz --json tagName,url,assets 2>/dev/null || true; } \
        | python3 site/downloads.py "{{ out }}/index.html"
    echo "Site assembled in {{ out }}"

# Assemble the website, serve it on this machine and open it in the
# browser, with the scene on (`just site-open ''` opens it as a visitor
# gets it: the scene only where it would run well). Ctrl+C stops serving.
site-open query='?mode=hifi' port='8000': site
    #!/usr/bin/env python3
    import functools, http.server, subprocess, sys

    handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory="{{ cargo-target-dir / 'site' }}")
    try:
        # Bound before the browser is sent there, so it finds it listening.
        server = http.server.ThreadingHTTPServer(("127.0.0.1", {{ port }}), handler)
    except OSError as error:
        sys.exit(f"could not serve on port {{ port }} ({error.strerror}): try `just site-open '{{ query }}' 8001`")
    url = "http://127.0.0.1:{{ port }}/{{ query }}"
    print(f"Serving {url} (Ctrl+C stops)")
    subprocess.run(["xdg-open", url], check=False)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass

# Make the website scene's props again (site/assets/models, site/assets/tex),
# all of them or those named: SolidCraft shapes them, TextureCraft surfaces
# them, glTF Transform packs them. See site/craft/README.md.
site-props *names:
    npm ci --prefix site --ignore-scripts --no-audit --no-fund --silent
    python3 site/craft/build.py {{ names }}

# Take the picture of the scene the website shows where the scene itself
# is not drawn (site/assets/still.jpg). Needs Playwright with Chromium.
site-still: site
    python3 site/craft/still.py "{{ cargo-target-dir / 'site' }}" site/assets/still.jpg

# Parse the diagrams in the doc comments, which rustdoc builds without reading
mermaid-check:
    #!/usr/bin/env bash
    set -euo pipefail
    if ! command -v node >/dev/null 2>&1; then
        echo "node not found, skipping the diagram check (install Node.js 20 or newer)"
    elif [ ! -d tools/mermaid-check/node_modules ]; then
        echo "tools/mermaid-check has no node_modules, skipping the diagram check (install with: npm ci --prefix tools/mermaid-check)"
    else
        node tools/mermaid-check/check.mjs src
    fi

# Run the application for testing purposes. Outside the panel the applet is a
# small window holding its button; logs go to the terminal.
run *args:
    env RUST_BACKTRACE=full cargo run --release {{ args }}

# Installs release build
install: (_install 'release')

# Installs debug build (unstripped, unoptimised — useful for debugging)
install-debug: (_install 'debug')

# Internal: install binary from the given profile plus shared resources.
# The one icon goes in under its symbolic name too, the name the desktop
# entry asks for: what lists the applets then draws it in the theme's
# foreground, light on a dark theme and dark on a light one.
# Installed onto this machine (no `rootdir`, which is a package being put
# together), whoever installs also has their yt-dlp seen to: see below.
[private]
_install profile:
    install -Dm0755 {{ cargo-target-dir / profile / name }} {{ bin-dst }}
    install -Dm0644 resources/app.desktop {{ desktop-dst }}
    install -Dm0644 resources/app.metainfo.xml {{ metainfo-dst }}
    install -Dm0644 resources/icon.svg {{ icon-dst }}
    install -Dm0644 resources/icon.svg {{ icon-symbolic-dst }}
    install -Dm0644 resources/icon.svg {{ icon-scalable-symbolic-dst }}
    -{{ if rootdir == '' { 'sh tools/yt-dlp-setup.sh' } else { 'true' } }}

# Sees that yt-dlp can read YouTube when the applet runs it: adds a JavaScript
# runtime, by its full path, to your yt-dlp configuration if it names none
yt-dlp-setup:
    sh tools/yt-dlp-setup.sh

# Fetches an emoji font the applet can draw in colour into its own fonts, for
# a system whose colour emoji font it can't draw. Only Avestruz reads it
emoji-font-setup:
    sh tools/emoji-font-setup.sh

# Uninstalls installed files
uninstall:
    rm -f {{ bin-dst }} {{ desktop-dst }} {{ metainfo-dst }} {{ icon-dst }} {{ icon-symbolic-dst }} {{ icon-scalable-symbolic-dst }}
