// SPDX-License-Identifier: MIT

//! Markdown, as far as notes are written in it: what a note marks as
//! bold, slanted, struck out or code, its headings, what it quotes, its
//! lists, its tables, and the links it names by something other than
//! their address.
//!
//! A note is plain text, and most are only that. Some are written as
//! Markdown, in whole or in a word, and nothing in a note says which. So
//! there is no deciding whether a note is Markdown: each mark is taken for
//! what it is where it could be nothing else, and left as written
//! everywhere else. A note with none of them comes out as it went in, one
//! run of text, the same to the letter.
//!
//! ```mermaid
//! flowchart TD
//!     note["What a note says"] --> lines["Line by line"]
//!     lines --> fence{"Between two fences?"}
//!     fence -->|"yes"| block["Code, as written"]
//!     fence -->|"no"| kind{"How the line starts"}
//!     kind -->|"# "| heading["A heading: bold, larger"]
//!     kind -->|"> "| quoted["Quoted: quiet, behind a bar"]
//!     kind -->|"- "| item["An item: behind a dot or a box"]
//!     kind -->|"--- alone"| rule["A rule"]
//!     kind -->|"cells, and --- under them"| table["A table"]
//!     kind -->|"anything else"| text["Text"]
//!     heading --> inline["Within the line"]
//!     quoted --> inline
//!     item --> inline
//!     text --> inline
//!     inline --> runs["Runs: text in one style, code, links, blocks"]
//!     table -->|"each cell"| inline
//!     block --> runs
//!     rule --> runs
//! ```
//!
//! What is taken for Markdown, and what is not:
//!
//! | Written | Shown as | Left as written when |
//! |---|---|---|
//! | `*slanted*`, `_slanted_` | Slanted | The marks are inside a word (`2*3*4`, `snake_case`), have a space on their inside, are not closed on the line, have a `\` before them, or have no letter or figure between them (`*_*`) |
//! | `**bold**`, `__bold__`, `***both***` | Bold | Likewise (`f**k`) |
//! | `~~struck~~` | Struck out | Likewise; one `~` is never a mark |
//! | `` `code` ``, with any number of backticks | Code | The same number does not close it on the line, or the backticks are inside a word (`don`t`) |
//! | A fence of three backticks or tildes, alone at the start of a line, and what follows down to another | A block of code | No fence closes it |
//! | `# Heading`, with up to six `#`, or a line with `===` under it | Bold, and the first three levels larger | There is no space after the `#`: that is a hashtag |
//! | `> quoted`, `>quoted` | Quiet, behind a bar | What follows the `>` with no space is not a letter (`>_<`, `>100`) |
//! | `- item`, `* item`, `+ item`, set in or not | The item, behind a dot | No other item stands on the line before or after: that is a dash, a footnote, a signature |
//! | `- [ ] to do`, `- [x] done` | The item, behind a box, ticked or not | |
//! | `1. item` | As written: it reads as what it is | |
//! | `---`, `***` or `___` alone on a line | A rule | |
//! | A line of cells between `\|`, and under it a line of `---` between as many | A table, each column set left, right or in the middle as its `:---`, `---:` or `:---:` says | The line under it is no such line |
//! | `[label](https://…)` | A link that reads as its label | The address is not one for the web |
//! | `![label](https://…)` | The picture or the video there, or a link | Likewise |
//! | `<https://…>` | The link | |
//! | `<br>` | A line break | |
//! | `\*`, and a `\` before any other mark | The mark, and no `\` | It is a shrug (`¯\_(ツ)_/¯`): a `\_` with no letter on either side |
//!
//! Not Markdown here, and left as written: a line with `---` under it (it
//! is a line and a rule, not a heading), code set in by four spaces (notes
//! are set in for other reasons), links whose address is given elsewhere
//! (`[label][1]`), footnotes, and HTML other than `<br>`, which is never
//! taken for anything.
//!
//! A line break is a line break: lines are not joined into paragraphs, as
//! Markdown would have it, since a note's lines are its author's.
//!
//! Web addresses are passed over whole: what is in one is not a mark. A
//! mark at the very end of one can still close what was opened before it
//! (`*see https://example.com/a*`).
//!
//! Nothing here reads links, mentions or hashtags: that is
//! [`content`](crate::nostr::content)'s, which reads each run of text it
//! is handed. What is code is not read for them at all.

use std::collections::HashSet;

use nostr_sdk::prelude::Url;

/// How a run of text is set.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Style {
    pub bold: bool,
    /// Slanted.
    pub italic: bool,
    /// Struck out.
    pub strike: bool,
    /// Code: in a face whose letters are all as wide, as it was written.
    pub code: bool,
    /// Quieter than the text around it: what is quoted.
    pub quiet: bool,
    /// A heading of this level, 1 to 6, or 0 for none.
    pub heading: u8,
}

impl Style {
    /// Whether this is text as any other.
    pub fn is_plain(&self) -> bool {
        *self == Self::default()
    }
}

/// A piece of a note, in reading order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Run {
    /// Text in one style, still to be read for the links, the mentions and
    /// the hashtags in it.
    Text { text: String, style: Style },
    /// Code: shown as written, and read for nothing.
    Code { text: String, style: Style },
    /// A link the note gives a label of its own, which may be empty.
    Link { url: Url, label: String },
    /// What stands apart from the text, between the lines before it and
    /// the lines after: no line break is left on either side of it.
    Block(Block<Vec<Run>>),
}

/// What stands apart from a note's text, with cells of `Cell`: of runs
/// here, and of what [`content`](crate::nostr::content) reads them into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block<Cell> {
    /// Lines of code, as written.
    Code(String),
    /// A rule between two parts of a note.
    Rule,
    Table(Table<Cell>),
}

/// A table: its columns, and its rows of as many cells each, the first
/// of them the one that says what the columns are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table<Cell> {
    pub columns: Vec<Align>,
    pub rows: Vec<Vec<Cell>>,
}

/// Where in its column a cell stands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Align {
    #[default]
    Left,
    Middle,
    Right,
}

impl<Cell> Block<Cell> {
    /// The same block, each cell of it made into another by `read`.
    pub fn map<Other>(self, mut read: impl FnMut(Cell) -> Other) -> Block<Other> {
        match self {
            Self::Code(code) => Block::Code(code),
            Self::Rule => Block::Rule,
            Self::Table(Table { columns, rows }) => Block::Table(Table {
                columns,
                rows: rows.into_iter().map(|row| row.into_iter().map(&mut read).collect()).collect(),
            }),
        }
    }
}

/// What stands before what a note quotes.
const QUOTE_BAR: &str = "\u{258e} ";

/// What stands before an item of a list, and before one that is to be
/// done or is.
const ITEM_DOT: &str = "\u{2022} ";
const ITEM_TO_DO: &str = "\u{2610} ";
const ITEM_DONE: &str = "\u{2611} ";

/// How many cells a table may have, all told. One of more is text.
const TABLE_MAX_CELLS: usize = 2_000;

/// How deep marks inside marks are followed. Past this they are text:
/// nobody writes them so, and a note is whatever its author sent.
const MAX_DEPTH: usize = 6;

/// How far a link's label and its address are looked for the end of, in
/// characters: longer than any is, and short enough that a note made of
/// nothing but openings is read in the time any other is.
const LINK_PART_MAX_CHARS: usize = 2048;

/// The characters a mark may come straight after, a space apart: what
/// opens a bracket or a quotation, and a dash.
const OPENING: &str = "([{\"'\u{201c}\u{2018}\u{ab}\u{2013}\u{2014}";

/// The characters a mark may come straight before, a space apart: what
/// closes a bracket or a quotation, or ends a clause, and a dash.
const CLOSING: &str = ".,!?;:)]}\"'\u{201d}\u{2019}\u{bb}\u{2026}\u{2013}\u{2014}";

/// The punctuation that follows a web address without being part of it.
const AFTER_ADDRESS: &str = ".,;:!?)]}";

/// Whether `c` is written without spaces between words, as Chinese,
/// Japanese and Korean are: a mark beside one is not inside a word.
fn unspaced(c: char) -> bool {
    matches!(u32::from(c), 0x3000..=0x30ff | 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xac00..=0xd7af | 0xff00..=0xffef | 0x2_0000..=0x2_ffff)
}

/// Whether `c` is a letter or a figure of a word: one written with spaces
/// between words.
fn of_a_word(c: char) -> bool {
    c.is_alphanumeric() && !unspaced(c)
}

/// Whether `c` is one of the characters marks are made of: what a `\`
/// before it keeps from being one.
fn is_mark(c: char) -> bool {
    matches!(c, '*' | '_' | '~' | '`' | '[' | ']' | '|')
}

/// What the mark of `count` times `c` adds to a style, if it is a mark.
fn stress(c: char, count: usize, style: Style) -> Option<Style> {
    match (c, count) {
        ('*' | '_', 1) => Some(Style { italic: true, ..style }),
        ('*' | '_', 2) => Some(Style { bold: true, ..style }),
        ('*' | '_', 3) => Some(Style { bold: true, italic: true, ..style }),
        ('~', 2) => Some(Style { strike: true, ..style }),
        _ => None,
    }
}

/// The runs a note is made of, gathered: text of one style that follows
/// text of the same is one run.
#[derive(Default)]
struct Runs(Vec<Run>);

impl Runs {
    fn text(&mut self, text: &str, style: Style) {
        if text.is_empty() {
            return;
        }
        match self.0.last_mut() {
            Some(Run::Text { text: last, style: same }) if *same == style => last.push_str(text),
            _ => self.0.push(Run::Text { text: text.to_string(), style }),
        }
    }

    fn code(&mut self, text: &str, style: Style) {
        if !text.is_empty() {
            self.0.push(Run::Code { text: text.to_string(), style: Style { code: true, ..style } });
        }
    }

    /// Set a block down: apart from the text, so without the line breaks
    /// and the empty lines that stood between it and the text before.
    fn block(&mut self, block: Block<Vec<Run>>) {
        while let Some(Run::Text { text, .. }) = self.0.last_mut() {
            text.truncate(text.trim_end().len());
            if !text.is_empty() {
                break;
            }
            self.0.pop();
        }
        self.0.push(Run::Block(block));
    }

    /// All of it as text, without its styles: what a label says.
    fn said(self) -> String {
        self.0
            .into_iter()
            .map(|run| match run {
                Run::Text { text, .. } | Run::Code { text, .. } => text,
                Run::Link { label, .. } => label,
                Run::Block(_) => String::new(),
            })
            .collect()
    }
}

/// One line of a note, or a part of one, read for the marks in it.
struct Inline<'a> {
    text: &'a str,
    /// Its characters, each with where it starts.
    chars: Vec<(usize, char)>,
    /// The closing marks that were looked for to the end and not found:
    /// which character, how many times. There is none further on either.
    missing: HashSet<(char, usize)>,
}

impl<'a> Inline<'a> {
    fn new(text: &'a str) -> Self {
        Self { text, chars: text.char_indices().collect(), missing: HashSet::new() }
    }

    /// The character at `at`, counted in characters.
    fn char(&self, at: usize) -> Option<char> {
        self.chars.get(at).map(|(_, c)| *c)
    }

    /// The text from the character at `from` up to the one at `to`.
    fn slice(&self, from: usize, to: usize) -> &'a str {
        let byte = |at: usize| self.chars.get(at).map_or(self.text.len(), |(byte, _)| *byte);
        self.text.get(byte(from)..byte(to)).unwrap_or_default()
    }

    /// How many times the character at `at` stands there in a row.
    fn run(&self, at: usize) -> usize {
        let c = self.char(at);
        (at..).take_while(|next| c.is_some() && self.char(*next) == c).count()
    }

    /// Whether the `\` at `at` stands before a mark: neither is one then.
    fn escapes(&self, at: usize) -> bool {
        self.char(at) == Some('\\') && self.char(at + 1).is_some_and(is_mark)
    }

    /// Whether the `\` at `at`, which stands before a mark, is left out
    /// of what is shown: always, but before a `_` with no letter or
    /// figure on either side of the two, which is the arm of a shrug.
    fn escape_goes(&self, at: usize) -> bool {
        let lettered = |at: Option<usize>| at.and_then(|at| self.char(at)).is_some_and(char::is_alphanumeric);
        self.char(at + 1) != Some('_') || lettered(at.checked_sub(1)) || lettered(Some(at + 2))
    }

    /// What stands at `at` between `<` and `>`, and where it ends: a web
    /// address, which is shown without them and read for the link it is,
    /// or a `br`, which is a line break.
    fn angled(&self, at: usize, style: Style) -> Option<(Run, usize)> {
        let end =
            (at + 1..at + LINK_PART_MAX_CHARS).find(|at| self.char(*at).is_none_or(|c| c == '>' || c == '<' || c == '\n'))?;
        if self.char(end) != Some('>') {
            return None;
        }
        let inside = self.slice(at + 1, end);
        if matches!(inside.to_ascii_lowercase().as_str(), "br" | "br/" | "br /") {
            return Some((Run::Text { text: "\n".to_string(), style }, end + 1));
        }
        let address = (inside.starts_with("https://") || inside.starts_with("http://")) && !inside.contains(char::is_whitespace);
        address.then(|| (Run::Text { text: inside.to_string(), style }, end + 1))
    }

    /// Where the web address ends whose `://` starts with the `:` at
    /// `colon`, if that is what it is: at a space, at what no address
    /// holds, or at the end.
    fn address_end(&self, colon: usize) -> Option<usize> {
        let scheme = colon.checked_sub(1).and_then(|before| self.char(before)).is_some_and(|c| c.is_ascii_alphanumeric());
        if !scheme || self.char(colon + 1) != Some('/') || self.char(colon + 2) != Some('/') {
            return None;
        }
        let ends = |c: char| c.is_whitespace() || !c.is_ascii() || matches!(c, '`' | '<' | '>' | '"');
        Some((colon + 3..).find(|at| self.char(*at).is_none_or(ends)).unwrap_or(self.chars.len()))
    }

    /// Where the address that ends at `end` has `count` times `mark` as
    /// the last of it, before the punctuation after it: marks that close
    /// what was opened before the address, and are no part of it.
    fn closed_by_address(&self, colon: usize, end: usize, mark: char, count: usize) -> Option<usize> {
        let mut last = end;
        while last > colon && self.char(last - 1).is_some_and(|c| AFTER_ADDRESS.contains(c)) {
            last -= 1;
        }
        let at = last.checked_sub(count).filter(|at| *at > colon + 2)?;
        let marks = (at..last).all(|each| self.char(each) == Some(mark));
        (marks && self.char(at - 1) != Some(mark)).then_some(at)
    }

    /// The code the backticks at `at` open, if they do: how many they
    /// are, and where as many, no more and no fewer, close it. Not a
    /// backtick inside a word, or one that would close in one: those are
    /// apostrophes of a keyboard that has none (`don`t`, `it`s`).
    fn code(&mut self, at: usize) -> Option<(usize, usize)> {
        let count = self.run(at);
        let inside = at.checked_sub(1).and_then(|before| self.char(before)).is_some_and(of_a_word);
        if inside || self.missing.contains(&('`', count)) {
            return None;
        }
        let mut next = at + count;
        while let Some(c) = self.char(next) {
            if c != '`' {
                next += 1;
                continue;
            }
            let ticks = self.run(next);
            if ticks == count && !self.char(next + ticks).is_some_and(of_a_word) {
                return Some((count, next));
            }
            next += ticks;
        }
        self.missing.insert(('`', count));
        None
    }

    /// Whether `count` marks at `at` may open: after a space, an opening
    /// bracket or nothing, and with something other than a space after.
    fn opens(&self, at: usize, count: usize) -> bool {
        let before = at.checked_sub(1).and_then(|before| self.char(before));
        let after = self.char(at + count);
        before.is_none_or(|c| c.is_whitespace() || OPENING.contains(c) || unspaced(c))
            && after.is_some_and(|c| !c.is_whitespace())
    }

    /// Whether `count` marks at `at` may close: with something other than
    /// a space before, and a space, what ends a clause or nothing after.
    fn closes(&self, at: usize, count: usize) -> bool {
        let before = at.checked_sub(1).and_then(|before| self.char(before));
        let after = self.char(at + count);
        before.is_some_and(|c| !c.is_whitespace())
            && after.is_none_or(|c| c.is_whitespace() || CLOSING.contains(c) || unspaced(c))
    }

    /// Where what `count` times `mark` open at `at` is closed, if it is,
    /// on this line: code and web addresses are passed over on the way.
    fn stress_end(&mut self, at: usize, mark: char, count: usize) -> Option<usize> {
        if self.missing.contains(&(mark, count)) {
            return None;
        }
        let mut next = at + count;
        while let Some(c) = self.char(next) {
            match c {
                '\\' if self.escapes(next) => next += 2,
                '`' => {
                    next = match self.code(next) {
                        Some((ticks, end)) => end + ticks,
                        None => next + self.run(next),
                    };
                }
                ':' => match self.address_end(next) {
                    Some(end) => {
                        if let Some(closing) = self.closed_by_address(next, end, mark, count) {
                            return Some(closing);
                        }
                        next = end;
                    }
                    None => next += 1,
                },
                c if c == mark => {
                    let marks = self.run(next);
                    if marks == count && self.closes(next, count) {
                        return Some(next);
                    }
                    next += marks;
                }
                _ => next += 1,
            }
        }
        self.missing.insert((mark, count));
        None
    }

    /// The link written at `at` as `[label](address)`, or with a `!`
    /// before it, and where it ends: a link if the address is one for the
    /// web, and the address alone, as text, if it is a `nostr:` one, which
    /// is read for what it names like any other.
    fn link(&self, at: usize, depth: usize) -> Option<(Run, usize)> {
        let open = if self.char(at) == Some('!') { at + 1 } else { at };
        if self.char(open) != Some('[') {
            return None;
        }
        let label_end =
            (open + 1..open + LINK_PART_MAX_CHARS).find(|at| self.char(*at).is_none_or(|c| matches!(c, ']' | '[')))?;
        if self.char(label_end) != Some(']') || self.char(label_end + 1) != Some('(') {
            return None;
        }
        // The address runs to the bracket that closes the one before it,
        // those inside it apart; what follows a space in there is a
        // title, which is not shown.
        let start = label_end + 2;
        let (mut nested, mut address_end, mut end) = (0_usize, None, None);
        for at in start..start + LINK_PART_MAX_CHARS {
            match self.char(at)? {
                '(' => nested += 1,
                ')' if nested == 0 => {
                    end = Some(at);
                    break;
                }
                ')' => nested -= 1,
                c if c.is_whitespace() && address_end.is_none() => address_end = Some(at),
                _ => {}
            }
        }
        let end = end?;
        let address = self.slice(start, address_end.unwrap_or(end)).trim_matches(['<', '>']);
        if address.starts_with("nostr:") {
            return Some((Run::Text { text: address.to_string(), style: Style::default() }, end + 1));
        }
        let url = Url::parse(address).ok().filter(|url| matches!(url.scheme(), "http" | "https"))?;
        // What the label says, without the marks in it.
        let mut label = Runs::default();
        Inline::new(self.slice(open + 1, label_end)).read(Style::default(), depth + 1, &mut label);
        Some((Run::Link { url, label: label.said().trim().to_string() }, end + 1))
    }

    /// Read the line into `runs`, in `style` where nothing marks it
    /// otherwise.
    fn read(&mut self, style: Style, depth: usize, runs: &mut Runs) {
        // Where the text not yet handed over starts, and where reading is.
        let (mut pending, mut at) = (0, 0);
        while let Some(c) = self.char(at) {
            // What a mark at `at` turns into, and where reading goes on.
            let marked: Option<(Run, usize)> = match c {
                '\\' if self.escapes(at) => {
                    if self.escape_goes(at) {
                        runs.text(self.slice(pending, at), style);
                        pending = at + 1;
                    }
                    at += 2;
                    continue;
                }
                '<' => self.angled(at, style),
                '`' => match self.code(at) {
                    Some((ticks, end)) => {
                        let code = Run::Code {
                            text: unpadded(self.slice(at + ticks, end)).to_string(),
                            style: Style { code: true, ..style },
                        };
                        Some((code, end + ticks))
                    }
                    None => {
                        at += self.run(at);
                        continue;
                    }
                },
                '!' | '[' if depth < MAX_DEPTH => self.link(at, depth),
                ':' => {
                    at = self.address_end(at).unwrap_or(at + 1);
                    continue;
                }
                '*' | '_' | '~' => {
                    let marks = self.run(at);
                    let stressed = stress(c, marks, style).filter(|_| depth < MAX_DEPTH && self.opens(at, marks));
                    let end = stressed.and_then(|stressed| Some((stressed, self.stress_end(at, c, marks)?)));
                    // What is stressed is words: marks around anything
                    // else are a face (`*_*`, `(_._)`), or decoration.
                    let end = end.filter(|(_, end)| self.slice(at + marks, *end).chars().any(char::is_alphanumeric));
                    let Some((stressed, end)) = end else {
                        at += marks;
                        continue;
                    };
                    runs.text(self.slice(pending, at), style);
                    Inline::new(self.slice(at + marks, end)).read(stressed, depth + 1, runs);
                    at = end + marks;
                    pending = at;
                    continue;
                }
                _ => None,
            };
            match marked {
                Some((run, end)) => {
                    runs.text(self.slice(pending, at), style);
                    match run {
                        Run::Text { text, style } => runs.text(&text, style),
                        Run::Code { text, style } => runs.code(&text, style),
                        other => runs.0.push(other),
                    }
                    at = end;
                    pending = at;
                }
                None => at += 1,
            }
        }
        runs.text(self.slice(pending, self.chars.len()), style);
    }
}

/// Code as it was written between its backticks, less the one space at
/// either end that sets it off from them, if there is one at both.
fn unpadded(code: &str) -> &str {
    code.strip_prefix(' ').and_then(|code| code.strip_suffix(' ')).filter(|code| !code.trim().is_empty()).unwrap_or(code)
}

/// The character a fence is made of and how many of them, if `line` is
/// one: three or more backticks or tildes at its start. `opening`, it may
/// say after them what the code is written in, which a closing one
/// doesn't.
fn fence(line: &str, opening: bool) -> Option<(char, usize)> {
    let mark = line.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let count = line.chars().take_while(|c| *c == mark).count();
    let rest = line.get(count * mark.len_utf8()..)?;
    let fits = if opening { !rest.contains('`') } else { rest.trim().is_empty() };
    (count >= 3 && fits).then_some((mark, count))
}

/// How deep a heading is and what it says, if `line` is one: up to six
/// `#`, a space, and something after it, less the `#` some close a
/// heading with, a space after what it says.
fn heading(line: &str) -> Option<(u8, &str)> {
    let said = line.trim_start_matches('#');
    let level = u8::try_from(line.len() - said.len()).ok().filter(|level| (1..=6).contains(level))?;
    if !said.starts_with([' ', '\t']) {
        return None;
    }
    let said = said.trim();
    let open = said.trim_end_matches('#');
    let said = if open.ends_with(' ') { open.trim_end() } else { said };
    (!said.is_empty()).then_some((level, said))
}

/// Whether `line` is the line under a heading written as a line of text
/// with `===` under it.
fn underlines(line: &str) -> bool {
    let marks = line.trim_end();
    marks.len() >= 3 && marks.chars().all(|c| c == '=')
}

/// What is quoted, if `line` quotes: after a `>`, or several, each with a
/// space after it or not. With no space at all, only what starts with a
/// letter is quoted: `>_<` is a face, and `>100` a number.
fn quoted(line: &str) -> Option<&str> {
    let mut rest = line.strip_prefix('>')?;
    let mut spaced = false;
    loop {
        if let Some(inner) = rest.strip_prefix(' ') {
            spaced = true;
            rest = inner;
        }
        match rest.strip_prefix('>') {
            Some(inner) => rest = inner,
            None => break,
        }
    }
    (spaced || rest.is_empty() || rest.starts_with(char::is_alphabetic)).then_some(rest)
}

/// The room before it and what it says, if `line` is written as an item
/// of a list: a `-`, a `*` or a `+`, a space, and something after it.
fn item(line: &str) -> Option<(&str, &str)> {
    let said = line.trim_start_matches(' ');
    let room = line.get(..line.len() - said.len())?;
    let said = said.strip_prefix(['-', '*', '+'])?.strip_prefix(' ')?.trim_start();
    (!said.is_empty() && !is_rule(line)).then_some((room, said))
}

/// Whether it is done and what it is, if what an item says is something
/// to do: `[ ]` or `[x]` before it.
fn to_do(said: &str) -> Option<(bool, &str)> {
    let done = if said.starts_with("[ ] ") {
        false
    } else if said.starts_with("[x] ") || said.starts_with("[X] ") {
        true
    } else {
        return None;
    };
    Some((done, said.get(4..)?.trim_start()))
}

/// Whether `line` is a rule: three or more of `-`, `*` or `_`, all the
/// same, and nothing else but spaces.
fn is_rule(line: &str) -> bool {
    let mut marks = line.chars().filter(|c| *c != ' ');
    let Some(mark) = marks.next().filter(|c| matches!(c, '-' | '*' | '_')) else {
        return false;
    };
    let more = marks.try_fold(1, |count, c| (c == mark).then_some(count + 1));
    more.is_some_and(|count| count >= 3)
}

/// The cells of a line of a table: what stands between its `|`, less
/// the empty one before the first `|` and after the last. A `|` with a
/// `\` before it is no end of a cell, in code either, and is a `|` in it.
fn cells(line: &str) -> Vec<String> {
    let line = line.trim();
    let mut cells = vec![String::new()];
    let mut escaped = false;
    for c in line.chars() {
        let Some(cell) = cells.last_mut() else { break };
        match c {
            '|' if escaped => {
                cell.pop();
                cell.push('|');
            }
            '|' => cells.push(String::new()),
            _ => cell.push(c),
        }
        escaped = c == '\\' && !escaped;
    }
    if line.starts_with('|') {
        cells.remove(0);
    }
    if cells.len() > 1 && cells.last().is_some_and(|last| last.trim().is_empty()) {
        cells.pop();
    }
    cells.into_iter().map(|cell| cell.trim().to_string()).collect()
}

/// Where each column stands, if `line` is the line under the first of a
/// table: nothing but `---` between `|`, with a `:` at the end the
/// column is set towards, or at both for the middle.
fn columns(line: &str) -> Option<Vec<Align>> {
    if !line.contains('|') || !line.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t')) {
        return None;
    }
    cells(line)
        .into_iter()
        .map(|cell| {
            let dashes = cell.trim_matches(':');
            let lined = !dashes.is_empty() && dashes.chars().all(|c| c == '-');
            lined.then_some(match (cell.starts_with(':'), cell.ends_with(':')) {
                (true, true) => Align::Middle,
                (false, true) => Align::Right,
                _ => Align::Left,
            })
        })
        .collect()
}

/// The table that starts at the first of `lines`, if one does, and how
/// many lines it is: a line of cells, the line that says where its
/// columns stand, and every line of cells after that.
fn table<'a>(lines: &[&'a str]) -> Option<(Table<Vec<Run>>, usize)> {
    let unbroken = |line: &&'a str| -> &'a str { line.strip_suffix('\r').unwrap_or(line) };
    let (first, under) = (lines.first().map(unbroken)?, lines.get(1).map(unbroken)?);
    if !first.contains('|') {
        return None;
    }
    let columns = columns(under)?;
    let head = cells(first);
    if head.len() != columns.len() {
        return None;
    }
    let read = |cell: String| {
        let mut runs = Runs::default();
        Inline::new(&cell).read(Style::default(), 0, &mut runs);
        runs.0
    };
    let mut rows = vec![head.into_iter().map(read).collect::<Vec<_>>()];
    let rest = lines.iter().skip(2).map(unbroken).take_while(|line| line.contains('|') && !line.trim().is_empty());
    for line in rest.take(TABLE_MAX_CELLS / columns.len().max(1)) {
        // A row has as many cells as there are columns: those it lacks
        // are empty, and those it has too many of are left out.
        let mut row: Vec<Vec<Run>> = cells(line).into_iter().take(columns.len()).map(read).collect();
        row.resize(columns.len(), Vec::new());
        rows.push(row);
    }
    let lines = rows.len() + 1;
    Some((Table { columns, rows }, lines))
}

/// One line of a note, outside any block, read into `runs`. `listed`, it
/// is an item of a list if it is written as one.
fn line(line: &str, listed: bool, runs: &mut Runs) {
    let plain = Style::default();
    let quiet = Style { quiet: true, ..plain };
    // A line may end in the `\r` of a line break written as two
    // characters, which is no part of what it says.
    let said = line.strip_suffix('\r').unwrap_or(line);
    if let Some((level, title)) = heading(said) {
        Inline::new(title).read(Style { bold: true, heading: level, ..plain }, 0, runs);
    } else if let Some(quoted) = quoted(said) {
        runs.text(QUOTE_BAR, quiet);
        Inline::new(quoted).read(quiet, 0, runs);
    } else if let Some((room, said)) = item(said).filter(|(_, said)| listed || to_do(said).is_some()) {
        runs.text(room, plain);
        let said = match to_do(said) {
            Some((done, said)) => {
                runs.text(if done { ITEM_DONE } else { ITEM_TO_DO }, plain);
                said
            }
            None => {
                runs.text(ITEM_DOT, plain);
                said
            }
        };
        Inline::new(said).read(plain, 0, runs);
    } else {
        Inline::new(line).read(plain, 0, runs);
    }
}

/// What `content` says, as runs: its text in the styles it marks, its
/// code, the links it labels, and what stands apart from its text. Of a
/// note that marks nothing, one run of text that is the note.
pub fn runs(content: &str) -> Vec<Run> {
    let lines: Vec<&str> = content.split('\n').collect();
    let unbroken = |at: usize| lines.get(at).map(|line| line.strip_suffix('\r').unwrap_or(line));
    let mut runs = Runs::default();
    // The fences that were looked for a closing one of and had none: no
    // fence of that character and as long or longer is closed further on.
    let mut unclosed: Vec<(char, usize)> = Vec::new();
    // Whether a line of text was the last thing read: a line break stands
    // between it and the next. None does on either side of a block, and
    // the empty lines there are left out: a block stands apart as it is.
    let mut after_text = false;
    let mut at = 0;
    while let Some(said) = unbroken(at) {
        let opened =
            fence(said, true).filter(|(mark, count)| !unclosed.iter().any(|(same, shorter)| same == mark && shorter <= count));
        if let Some((mark, count)) = opened {
            let closes = |line: &&str| {
                fence(line.trim_end_matches('\r'), false).is_some_and(|(same, longer)| same == mark && longer >= count)
            };
            match lines.iter().skip(at + 1).position(closes) {
                Some(lines_of_code) => {
                    let code: Vec<&str> =
                        lines.iter().skip(at + 1).take(lines_of_code).map(|line| line.trim_end_matches('\r')).collect();
                    runs.block(Block::Code(code.join("\n")));
                    (after_text, at) = (false, at + lines_of_code + 2);
                    continue;
                }
                None => unclosed.push((mark, count)),
            }
        }
        if let Some((table, lines_of_it)) = lines.get(at..).and_then(table) {
            runs.block(Block::Table(table));
            (after_text, at) = (false, at + lines_of_it);
            continue;
        }
        if is_rule(said) {
            runs.block(Block::Rule);
            (after_text, at) = (false, at + 1);
            continue;
        }
        let after_block = matches!(runs.0.last(), Some(Run::Block(_)));
        if after_block && !after_text && said.trim().is_empty() {
            at += 1;
            continue;
        }
        if after_text {
            runs.text("\n", Style::default());
        }
        after_text = true;
        // A line of text with `===` under it is a heading.
        let plainly_text = !said.trim().is_empty() && heading(said).is_none() && quoted(said).is_none() && item(said).is_none();
        if plainly_text && unbroken(at + 1).is_some_and(underlines) {
            Inline::new(said.trim()).read(Style { bold: true, heading: 1, ..Style::default() }, 0, &mut runs);
            at += 2;
            continue;
        }
        // An item of a list has another on the line before it or after.
        let is_item = |at: Option<usize>| at.and_then(unbroken).is_some_and(|line| item(line).is_some());
        let listed = is_item(at.checked_sub(1)) || is_item(Some(at + 1));
        line(lines.get(at).copied().unwrap_or_default(), listed, &mut runs);
        at += 1;
    }
    runs.0
}

#[cfg(test)]
mod tests {
    use super::{Align, Block, Run, Style, Table, runs};

    const PLAIN: Style = Style { bold: false, italic: false, strike: false, code: false, quiet: false, heading: 0 };
    const BOLD: Style = Style { bold: true, ..PLAIN };
    const ITALIC: Style = Style { italic: true, ..PLAIN };
    const STRUCK: Style = Style { strike: true, ..PLAIN };
    const CODE: Style = Style { code: true, ..PLAIN };
    const QUIET: Style = Style { quiet: true, ..PLAIN };

    fn text(text: &str, style: Style) -> Run {
        Run::Text { text: text.to_string(), style }
    }

    fn code(text: &str) -> Run {
        Run::Code { text: text.to_string(), style: CODE }
    }

    fn link(url: &str, label: &str) -> Run {
        Run::Link { url: url.parse().unwrap(), label: label.to_string() }
    }

    /// Lines of code that stand apart.
    fn lines_of_code(code: &str) -> Run {
        Run::Block(Block::Code(code.to_string()))
    }

    /// A heading of `level`, in the style it is set in.
    fn heading(level: u8) -> Style {
        Style { bold: true, heading: level, ..PLAIN }
    }

    /// What a note that marks nothing comes out as: itself.
    fn untouched(said: &str) {
        assert_eq!(runs(said), [text(said, PLAIN)], "{said:?}");
    }

    #[test]
    fn a_note_that_marks_nothing_is_one_run_of_itself() {
        untouched("Just a note.\n\nWith two paragraphs, a #hashtag and https://example.com/a_b_c/*d*.");
        assert_eq!(runs(""), []);
    }

    #[test]
    fn marks_inside_words_are_not_marks() {
        for said in [
            "2*3*4 = 24 and 5 * 3 * 7 = 105",
            "f**k this s**t",
            "snake_case_names and a_b__c__d",
            "about ~5 minutes, or ~10",
            "a footnote* and another*",
            "*unclosed and **unclosed too",
            "*not closed on\nthe line*",
            "**** and ***** and ~~~~",
        ] {
            untouched(said);
        }
    }

    #[test]
    fn faces_made_of_marks_stay_faces() {
        for said in [
            "¯\\_(ツ)_/¯",
            "¯_(ツ)_/¯",
            "^_^ and again ^_^",
            "(>_<) (>_<)",
            "o_O ... O_o",
            "*_* (_._) *-*",
            "*** !!! ***?",
            ">_< and\n>:( and\n>100 came",
        ] {
            untouched(said);
        }
    }

    #[test]
    fn a_backslash_before_a_mark_keeps_it_from_being_one_and_goes() {
        assert_eq!(runs("\\*not slanted\\* and \\`not code\\`"), [text("*not slanted* and `not code`", PLAIN)]);
        assert_eq!(runs("snake\\_case and \\_private\\_"), [text("snake_case and _private_", PLAIN)]);
        // But not the arm of a shrug, and not one before anything else.
        untouched("¯\\_(ツ)_/¯ in C:\\Users\\me, and a \\n");
    }

    #[test]
    fn what_is_marked_is_set_as_marked_without_its_marks() {
        assert_eq!(
            runs("Check out *Avestruz*! It has a ```--toggle``` flag, I have *not* **read** it ~~all~~."),
            [
                text("Check out ", PLAIN),
                text("Avestruz", ITALIC),
                text("! It has a ", PLAIN),
                code("--toggle"),
                text(" flag, I have ", PLAIN),
                text("not", ITALIC),
                text(" ", PLAIN),
                text("read", BOLD),
                text(" it ", PLAIN),
                text("all", STRUCK),
                text(".", PLAIN),
            ]
        );
        assert_eq!(
            runs("_one_ and (_two_)"),
            [text("one", ITALIC), text(" and (", PLAIN), text("two", ITALIC), text(")", PLAIN)]
        );
        let both = Style { bold: true, italic: true, ..PLAIN };
        assert_eq!(runs("***both*** and ___both___"), [text("both", both), text(" and ", PLAIN), text("both", both)]);
        // Two underscores are bold as two stars are: what is named so is
        // code, and written as code.
        assert_eq!(runs("__bold__ and `__init__`"), [text("bold", BOLD), text(" and ", PLAIN), code("__init__")]);
    }

    #[test]
    fn marks_inside_marks_add_up() {
        assert_eq!(
            runs("**bold with *slanted* and `code` in it**"),
            [
                text("bold with ", BOLD),
                text("slanted", Style { bold: true, italic: true, ..PLAIN }),
                text(" and ", BOLD),
                Run::Code { text: "code".into(), style: Style { bold: true, code: true, ..PLAIN } },
                text(" in it", BOLD),
            ]
        );
    }

    #[test]
    fn marks_beside_text_written_without_spaces_are_marks() {
        assert_eq!(runs("这是**重点**内容"), [text("这是", PLAIN), text("重点", BOLD), text("内容", PLAIN)]);
    }

    #[test]
    fn code_is_as_written_whatever_is_in_it() {
        assert_eq!(runs("run `a *b* _c_ #d` now"), [text("run ", PLAIN), code("a *b* _c_ #d"), text(" now", PLAIN)]);
        // As many backticks close it as opened it, so it can hold fewer.
        assert_eq!(runs("`` a ` b ``"), [code("a ` b")]);
        // One that is not closed is a backtick, and so is one in a word:
        // an apostrophe, on a keyboard without one.
        untouched("it's 5 o`clock somewhere, and `` there");
        untouched("I don`t think it`s fair, `tis not");
        // A mark is not closed by one inside code.
        assert_eq!(runs("*open `closed*` here"), [text("*open ", PLAIN), code("closed*"), text(" here", PLAIN)]);
    }

    #[test]
    fn what_is_between_two_fences_is_code_that_stands_apart() {
        // No line break is left on either side of it, nor an empty line.
        assert_eq!(
            runs("Look:\n\n```rust\nfn main() {\n    let *x* = 1;\n}\n```\n\ndone"),
            [text("Look:", PLAIN), lines_of_code("fn main() {\n    let *x* = 1;\n}"), text("done", PLAIN)]
        );
        assert_eq!(runs("~~~\na\n~~~"), [lines_of_code("a")]);
        // A longer fence holds a shorter one.
        assert_eq!(runs("````\n```\n````"), [lines_of_code("```")]);
        // One that nothing closes is what was written, and what follows
        // it is read as it would be without it.
        untouched("```\nnever closed");
        assert_eq!(runs("```\nopen\n*but* here"), [text("```\nopen\n", PLAIN), text("but", ITALIC), text(" here", PLAIN)]);
    }

    #[test]
    fn a_heading_is_bold_and_says_how_deep_and_a_hashtag_is_not_one() {
        assert_eq!(
            runs("## What *is* new ##\nthis"),
            [
                text("What ", heading(2)),
                text("is", Style { italic: true, ..heading(2) }),
                text(" new", heading(2)),
                text("\nthis", PLAIN)
            ]
        );
        assert_eq!(runs("# I write C#"), [text("I write C#", heading(1))]);
        assert_eq!(runs("###### six"), [text("six", heading(6))]);
        untouched("#nostr is not a heading\n####### neither are seven\n# ");
    }

    #[test]
    fn a_line_with_equals_signs_under_it_is_a_heading() {
        assert_eq!(runs("The title\n=========\nand so on"), [text("The title", heading(1)), text("\nand so on", PLAIN)]);
        // Not what is something else already, and not two of them.
        untouched("a == b\n==\nc");
        assert_eq!(runs("> said\n===="), [text("\u{258e} said", QUIET), text("\n====", PLAIN)]);
    }

    #[test]
    fn what_is_quoted_is_quiet_behind_a_bar() {
        assert_eq!(
            runs("> to be, or *not*\n> > to be\n>be me\nsaid he"),
            [
                text("\u{258e} to be, or ", QUIET),
                text("not", Style { italic: true, ..QUIET }),
                text("\n", PLAIN),
                text("\u{258e} to be", QUIET),
                text("\n", PLAIN),
                text("\u{258e} be me", QUIET),
                text("\nsaid he", PLAIN),
            ]
        );
        untouched("-> not this\n2 > 1");
    }

    #[test]
    fn items_of_a_list_get_a_dot_and_a_dash_alone_does_not() {
        assert_eq!(
            runs("- one\n  * *two*\n+ three"),
            [text("\u{2022} one\n  \u{2022} ", PLAIN), text("two", ITALIC), text("\n\u{2022} three", PLAIN)]
        );
        // One with no other beside it is a dash: a signature, a footnote.
        untouched("so it goes\n\n- signed, me");
        untouched("* terms apply\nto all of it");
        // Numbers read as what they are, and are left.
        untouched("1. one\n2. two\n3) three");
    }

    #[test]
    fn what_is_to_do_gets_a_box_ticked_or_not() {
        assert_eq!(runs("- [ ] write it\n- [x] ship it"), [text("\u{2610} write it\n\u{2611} ship it", PLAIN)]);
        // Alone too: it could be nothing else.
        assert_eq!(runs("* [X] done"), [text("\u{2611} done", PLAIN)]);
    }

    #[test]
    fn three_marks_alone_on_a_line_are_a_rule_that_stands_apart() {
        for marks in ["---", "***", "___", "- - -", "-----"] {
            assert_eq!(
                runs(&format!("above\n\n{marks}\nbelow")),
                [text("above", PLAIN), Run::Block(Block::Rule), text("below", PLAIN)],
                "{marks}"
            );
        }
        untouched("--\n-- \n--- not alone\n-*-");
    }

    #[test]
    fn lines_of_cells_with_a_line_of_dashes_under_the_first_are_a_table() {
        let cell = |said: &str| vec![text(said, PLAIN)];
        assert_eq!(
            runs(
                "Dams:\n|Category|Approximate count|\n|---|---|\n|Large dams in the U.S.|92,000+|\n|Large dams worldwide|~200,000|\n\nso many"
            ),
            [
                text("Dams:", PLAIN),
                Run::Block(Block::Table(Table {
                    columns: vec![Align::Left, Align::Left],
                    rows: vec![
                        vec![cell("Category"), cell("Approximate count")],
                        vec![cell("Large dams in the U.S."), cell("92,000+")],
                        vec![cell("Large dams worldwide"), cell("~200,000")],
                    ],
                })),
                text("so many", PLAIN),
            ]
        );
    }

    #[test]
    fn a_table_says_where_its_columns_stand_and_reads_its_cells_as_text() {
        let table = match runs("a | b | c\n:-- | :-: | --:\n**x** | `y \\| z` | [w](https://example.com)\nshort |\n1 | 2 | 3 | 4")
            .as_slice()
        {
            [Run::Block(Block::Table(table))] => table.clone(),
            other => panic!("not a table: {other:?}"),
        };
        assert_eq!(table.columns, [Align::Left, Align::Middle, Align::Right]);
        assert_eq!(
            table.rows.get(1),
            Some(&vec![vec![text("x", BOLD)], vec![code("y | z")], vec![link("https://example.com", "w")]])
        );
        // A row has as many cells as there are columns, whatever it has.
        assert_eq!(table.rows.get(2), Some(&vec![vec![text("short", PLAIN)], vec![], vec![]]));
        assert_eq!(table.rows.get(3).map(Vec::len), Some(3));
    }

    #[test]
    fn a_line_with_a_bar_in_it_is_not_a_table_by_itself() {
        untouched("this | that\nand the other");
        untouched("a | b\n--\nc");
        untouched("| a | b |\n| --- |\n| c |");
        untouched("yes | no\n-- maybe | so");
    }

    #[test]
    fn a_labelled_link_is_a_link_that_says_its_label() {
        assert_eq!(
            runs("Read [the **notes**](https://example.com/a_(b)) first."),
            [text("Read ", PLAIN), link("https://example.com/a_(b)", "the notes"), text(" first.", PLAIN)]
        );
        // A picture is a link too, until it is seen what is there.
        assert_eq!(runs("![a cat](https://example.com/cat.png \"Tom\")"), [link("https://example.com/cat.png", "a cat")]);
        // An address that is not one for the web is no link.
        untouched("[click](javascript:alert(1)) or [here](/local) or [x] (y) or [a [b](c)");
        // One for Nostr is read as it would be without the brackets.
        assert_eq!(runs("by [me](nostr:npub1xyz)"), [text("by nostr:npub1xyz", PLAIN)]);
    }

    #[test]
    fn an_address_between_angle_brackets_is_shown_without_them_and_a_br_is_a_line_break() {
        assert_eq!(runs("see <https://example.com/a>, or not"), [text("see https://example.com/a, or not", PLAIN)]);
        assert_eq!(runs("one<br>two<BR/>three"), [text("one\ntwo\nthree", PLAIN)]);
        // Nothing else between them is taken for anything.
        untouched("<b>not bold</b> <script>alert(1)</script> <3 a < b > c <mailto:a@b.c>");
    }

    #[test]
    fn what_is_in_a_web_address_is_not_a_mark() {
        untouched("see https://example.com/a_b_/c*d*/~~e~~ and _https://example.com/x");
        // But a mark that ends one closes what was opened before it.
        assert_eq!(runs("*see https://example.com/a*."), [text("see https://example.com/a", ITALIC), text(".", PLAIN)]);
    }

    #[test]
    fn line_breaks_written_as_two_characters_are_kept_where_nothing_is_marked() {
        untouched("one\r\ntwo\r\n");
        assert_eq!(runs("# Title\r\ntext"), [text("Title", heading(1)), text("\ntext", PLAIN)]);
        // And are no part of a table, a fence or a rule.
        assert!(
            matches!(runs("a|b\r\n-|-\r\nc|d\r\n").as_slice(), [Run::Block(Block::Table(table)), ..] if table.rows.len() == 2)
        );
        assert_eq!(runs("```\r\ncode\r\n```\r\n---\r"), [lines_of_code("code"), Run::Block(Block::Rule)]);
    }

    #[test]
    fn a_note_of_nothing_but_marks_is_read_in_the_time_any_other_is() {
        // Each of these would take minutes if every mark looked to the
        // end of the note for the one that closes it: a mark that was
        // looked for once and not found is not looked for again.
        let started = std::time::Instant::now();
        for said in [
            " *a".repeat(40_000),
            " `a".repeat(40_000),
            "*_".repeat(60_000),
            "```x\n".repeat(20_000),
            " **a `".repeat(20_000),
            "a|b\n".repeat(30_000),
            "|a\n|-\n".repeat(20_000),
            "|".repeat(60_000),
            "- a\n".repeat(30_000),
            "<a ".repeat(40_000),
        ] {
            let _ = runs(&said);
        }
        // A link's label and address are looked for the end of so far and
        // no further, however many open.
        let _ = runs(&"[a](".repeat(4_000));
        let _ = runs(&"<".repeat(4_000));
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "{:?}", started.elapsed());
    }

    #[test]
    fn marks_inside_marks_are_followed_so_deep_and_no_deeper() {
        // Nobody writes them so, and whoever does gets text for it.
        let deep = format!("{}deep{}", "*_".repeat(50), "_*".repeat(50));
        let said: String = runs(&deep)
            .into_iter()
            .map(|run| match run {
                Run::Text { text, .. } | Run::Code { text, .. } => text,
                Run::Link { label, .. } => label,
                Run::Block(_) => String::new(),
            })
            .collect();
        assert!(said.contains("deep") && said.contains('*'), "{said}");
    }
}
