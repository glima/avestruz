// SPDX-License-Identifier: MIT

// Parses the Mermaid diagrams in the doc comments of the Rust sources.
//
// rustdoc knows nothing of Mermaid: a diagram that doesn't parse builds
// without a word and shows as an error where the diagram should be. This
// parses every ```mermaid fence found in a doc comment (`//!` or `///`)
// with the Mermaid release the documentation draws them with, and says
// where each that doesn't parse is.
//
// Usage: node check.mjs <directory>...

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

import { JSDOM } from "jsdom";

// Mermaid looks for a page to draw in, even to parse.
const page = new JSDOM("<!doctype html><html><body></body></html>");
globalThis.window = page.window;
globalThis.document = page.window.document;
const { default: mermaid } = await import("mermaid");
mermaid.initialize({ startOnLoad: false });

/// Every `.rs` file under `directory`.
function sources(directory) {
    return readdirSync(directory).flatMap((name) => {
        const path = join(directory, name);
        if (statSync(path).isDirectory()) {
            return sources(path);
        }
        return name.endsWith(".rs") ? [path] : [];
    });
}

/// The diagrams in the doc comments of `text`: what each says, and the
/// line its fence opens on.
export function diagrams(text) {
    const found = [];
    let open = null;
    text.split("\n").forEach((line, index) => {
        const comment = line.match(/^\s*\/\/[\/!] ?(.*)$/);
        if (!comment) {
            open = null;
            return;
        }
        const said = comment[1];
        if (open) {
            if (said.trim() === "```") {
                found.push(open);
                open = null;
            } else {
                open.source += `${said}\n`;
            }
        } else if (said.trim() === "```mermaid") {
            open = { line: index + 1, source: "" };
        }
    });
    return found;
}

const directories = process.argv.slice(2);
if (directories.length === 0) {
    console.error("usage: node check.mjs <directory>...");
    process.exit(2);
}

let checked = 0;
let failed = 0;
for (const path of directories.flatMap(sources).sort()) {
    for (const { line, source } of diagrams(readFileSync(path, "utf8"))) {
        checked += 1;
        try {
            await mermaid.parse(source);
        } catch (error) {
            failed += 1;
            console.error(`${path}:${line}: ${String(error.message ?? error).trim()}\n`);
        }
    }
}
console.log(`${checked} diagram(s) parsed, ${failed} failed`);
process.exit(failed === 0 ? 0 : 1);
