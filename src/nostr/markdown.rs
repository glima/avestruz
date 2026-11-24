// SPDX-License-Identifier: MIT

//! Markdown, as far as notes are written in it: what a note marks as
//! bold, slanted, struck out or code, its headings, what it quotes, and
//! the links it names by something other than their address.
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
//!     kind -->|"# "| heading["A heading: bold"]
//!     kind -->|"> "| quoted["Quoted: quiet, behind a bar"]
//!     kind -->|"* "| item["An item: behind a dot"]
//!     kind -->|"--- alone"| rule["A rule"]
//!     kind -->|"anything else"| text["Text"]
//!     heading --> inline["Within the line"]
//!     quoted --> inline
//!     item --> inline
//!     text --> inline
//!     inline --> runs["Runs: text in one style, code, links"]
//!     block --> runs
//!     rule --> runs
//! ```
//!
//! What is taken for Markdown, and what is not:
//!
//! | Written | Shown as | Left as written when |
//! |---|---|---|
//! | `*slanted*`, `_slanted_` | Slanted | The marks are inside a word (`2*3*4`, `snake_case`), have a space on their inside, are not closed on the line, have a `\` before them, or have no letter or figure between them (`*_*`) |
//! | `**bold**`, `***both***` | Bold | Likewise (`f**k`) |
//! | `~~struck~~` | Struck out | Likewise; one `~` is never a mark |
//! | `` `code` ``, with any number of backticks | Code | The same number does not close it on the line, or the backticks are inside a word (`don`t`) |
//! | A fence of three backticks or tildes, alone at the start of a line, and what follows down to another | Code | No fence closes it |
//! | `# Heading`, with up to six `#` | Bold | There is no space after the `#`: that is a hashtag |
//! | `> quoted` | Quiet, behind a bar | There is no space after the `>` |
//! | `* item` | The item, behind a dot | |
//! | `---`, `***` or `___` alone on a line | A rule | |
//! | `[label](https://…)` | A link that reads as its label | The address is not one for the web |
//! | `![label](https://…)` | The picture or the video there, or a link | Likewise |
//!
//! Left as written altogether: `__double underscores__`, which more often
//! name something (`__init__`) than stress it; lists that start with `-`,
//! `+` or a number, which read as lists already; tables; and a `\` before a
//! mark, which stays, since a shrug (`¯\_(ツ)_/¯`) is written more often
//! than a mark is escaped.
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
    /// Quieter than the text around it: what is quoted, and a rule.
    pub quiet: bool,
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
}

/// What stands before what a note quotes.
const QUOTE_BAR: &str = "\u{258e} ";

/// What stands before an item of a list.
const ITEM_DOT: &str = "\u{2022} ";

/// A rule between two parts of a note.
const RULE: &str = "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}";

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

/// Whether `c` is one of the characters marks are made of.
fn is_mark(c: char) -> bool {
    matches!(c, '*' | '_' | '~' | '`' | '[')
}

/// What the mark of `count` times `c` adds to a style, if it is a mark.
fn stress(c: char, count: usize, style: Style) -> Option<Style> {
    match (c, count) {
        ('*' | '_', 1) => Some(Style { italic: true, ..style }),
        ('*', 2) => Some(Style { bold: true, ..style }),
        ('*', 3) => Some(Style { bold: true, italic: true, ..style }),
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

    /// All of it as text, without its styles: what a label says.
    fn said(self) -> String {
        self.0
            .into_iter()
            .map(|run| match run {
                Run::Text { text, .. } | Run::Code { text, .. } => text,
                Run::Link { label, .. } => label,
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
                    at += 2;
                    continue;
                }
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
                        link @ Run::Link { .. } => runs.0.push(link),
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

/// What a heading says, if `line` is one: up to six `#`, a space, and
/// something after it, less the `#` some close a heading with, a space
/// after what it says.
fn heading(line: &str) -> Option<&str> {
    let said = line.trim_start_matches('#');
    let marks = line.len() - said.len();
    if !(1..=6).contains(&marks) || !said.starts_with([' ', '\t']) {
        return None;
    }
    let said = said.trim();
    let open = said.trim_end_matches('#');
    let said = if open.ends_with(' ') { open.trim_end() } else { said };
    (!said.is_empty()).then_some(said)
}

/// What is quoted, if `line` quotes: a `>` and a space, or a `>` alone,
/// and the same again for what is quoted in what is quoted.
fn quoted(line: &str) -> Option<&str> {
    let mut said = None;
    let mut rest = line;
    while let Some(inner) = rest.strip_prefix('>').filter(|inner| inner.is_empty() || inner.starts_with(' ')) {
        rest = inner.strip_prefix(' ').unwrap_or(inner);
        said = Some(rest);
    }
    said
}

/// The room before it and what it says, if `line` is an item of a list
/// written with `*`.
fn item(line: &str) -> Option<(&str, &str)> {
    let said = line.trim_start_matches(' ');
    let room = line.get(..line.len() - said.len())?;
    let said = said.strip_prefix("* ")?.trim_start();
    (!said.is_empty()).then_some((room, said))
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

/// One line of a note, outside any code, read into `runs`.
fn line(line: &str, runs: &mut Runs) {
    let plain = Style::default();
    let quiet = Style { quiet: true, ..plain };
    // A line may end in the `\r` of a line break written as two
    // characters, which is no part of what it says.
    let said = line.strip_suffix('\r').unwrap_or(line);
    if is_rule(said) {
        runs.text(RULE, quiet);
    } else if let Some(title) = heading(said) {
        Inline::new(title).read(Style { bold: true, ..plain }, 0, runs);
    } else if let Some(quoted) = quoted(said) {
        runs.text(QUOTE_BAR, quiet);
        Inline::new(quoted).read(quiet, 0, runs);
    } else if let Some((room, said)) = item(said) {
        runs.text(room, plain);
        runs.text(ITEM_DOT, plain);
        Inline::new(said).read(plain, 0, runs);
    } else {
        Inline::new(line).read(plain, 0, runs);
    }
}

/// What `content` says, as runs: its text in the styles it marks, its
/// code, and the links it labels. Of a note that marks nothing, one run
/// of text that is the note.
pub fn runs(content: &str) -> Vec<Run> {
    let lines: Vec<&str> = content.split('\n').collect();
    let mut runs = Runs::default();
    // The fences that were looked for a closing one of and had none: no
    // fence of that character and as long or longer is closed further on.
    let mut unclosed: Vec<(char, usize)> = Vec::new();
    let mut at = 0;
    while let Some(said) = lines.get(at) {
        if at > 0 {
            runs.text("\n", Style::default());
        }
        let opened = fence(said.trim_end_matches('\r'), true)
            .filter(|(mark, count)| !unclosed.iter().any(|(same, shorter)| same == mark && shorter <= count));
        if let Some((mark, count)) = opened {
            let closes = |line: &&str| {
                fence(line.trim_end_matches('\r'), false).is_some_and(|(same, longer)| same == mark && longer >= count)
            };
            match lines.iter().skip(at + 1).position(closes) {
                Some(lines_of_code) => {
                    let code: Vec<&str> =
                        lines.iter().skip(at + 1).take(lines_of_code).map(|line| line.trim_end_matches('\r')).collect();
                    runs.code(&code.join("\n"), Style::default());
                    at += lines_of_code + 2;
                    continue;
                }
                None => unclosed.push((mark, count)),
            }
        }
        line(said, &mut runs);
        at += 1;
    }
    runs.0
}

#[cfg(test)]
mod tests {
    use super::{Run, Style, runs};

    const PLAIN: Style = Style { bold: false, italic: false, strike: false, code: false, quiet: false };
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
            "snake_case_names and _private_var here",
            "__init__.py and __main__",
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
        for said in
            ["¯\\_(ツ)_/¯", "¯_(ツ)_/¯", "^_^ and again ^_^", "(>_<) (>_<)", "o_O ... O_o", "*_* (_._) *-*", "*** !!! ***?"]
        {
            untouched(said);
        }
        // A `\` before a mark keeps it from being one, and stays.
        untouched("\\*not slanted\\* and \\`not code\\`");
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
        assert_eq!(runs("***both***"), [text("both", Style { bold: true, italic: true, ..PLAIN })]);
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
    fn what_is_between_two_fences_is_code() {
        assert_eq!(
            runs("Look:\n```rust\nfn main() {\n    let *x* = 1;\n}\n```\ndone"),
            [text("Look:\n", PLAIN), code("fn main() {\n    let *x* = 1;\n}"), text("\ndone", PLAIN)]
        );
        assert_eq!(runs("~~~\na\n~~~"), [code("a")]);
        // A longer fence holds a shorter one.
        assert_eq!(runs("````\n```\n````"), [code("```")]);
        // One that nothing closes is what was written, and what follows
        // it is read as it would be without it.
        untouched("```\nnever closed");
        assert_eq!(runs("```\nopen\n*but* here"), [text("```\nopen\n", PLAIN), text("but", ITALIC), text(" here", PLAIN)]);
    }

    #[test]
    fn a_heading_is_bold_and_a_hashtag_is_not_one() {
        assert_eq!(
            runs("## What *is* new ##\nthis"),
            [
                text("What ", BOLD),
                text("is", Style { bold: true, italic: true, ..PLAIN }),
                text(" new", BOLD),
                text("\nthis", PLAIN)
            ]
        );
        assert_eq!(runs("# I write C#"), [text("I write C#", BOLD)]);
        untouched("#nostr is not a heading\n####### neither are seven\n# ");
    }

    #[test]
    fn what_is_quoted_is_quiet_behind_a_bar() {
        assert_eq!(
            runs("> to be, or *not*\n> > to be\nsaid he"),
            [
                text("\u{258e} to be, or ", QUIET),
                text("not", Style { italic: true, ..QUIET }),
                text("\n", PLAIN),
                text("\u{258e} to be", QUIET),
                text("\nsaid he", PLAIN),
            ]
        );
        untouched(">be me\n-> not this either");
    }

    #[test]
    fn items_get_a_dot_and_other_lists_are_left() {
        assert_eq!(runs("* one\n  * *two*"), [text("\u{2022} one\n  \u{2022} ", PLAIN), text("two", ITALIC)]);
        untouched("- one\n- two\n+ three\n1. four\n- signed, me");
    }

    #[test]
    fn three_marks_alone_on_a_line_are_a_rule() {
        let rule = "\u{2500}".repeat(12);
        for marks in ["---", "***", "___", "- - -", "-----"] {
            assert_eq!(
                runs(&format!("above\n{marks}\nbelow")),
                [text("above\n", PLAIN), text(&rule, QUIET), text("\nbelow", PLAIN)],
                "{marks}"
            );
        }
        untouched("--\n-- \n--- not alone\n-*-");
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
    fn what_is_in_a_web_address_is_not_a_mark() {
        untouched("see https://example.com/a_b_/c*d*/~~e~~ and _https://example.com/x");
        // But a mark that ends one closes what was opened before it.
        assert_eq!(runs("*see https://example.com/a*."), [text("see https://example.com/a", ITALIC), text(".", PLAIN)]);
    }

    #[test]
    fn line_breaks_written_as_two_characters_are_kept_where_nothing_is_marked() {
        untouched("one\r\ntwo\r\n");
        assert_eq!(runs("# Title\r\ntext"), [text("Title", BOLD), text("\ntext", PLAIN)]);
    }

    #[test]
    fn a_note_of_nothing_but_marks_is_read_in_the_time_any_other_is() {
        // Each of these would take minutes if every mark looked to the
        // end of the note for the one that closes it: a mark that was
        // looked for once and not found is not looked for again.
        let started = std::time::Instant::now();
        for said in
            [" *a".repeat(40_000), " `a".repeat(40_000), "*_".repeat(60_000), "```x\n".repeat(20_000), " **a `".repeat(20_000)]
        {
            let _ = runs(&said);
        }
        // A link's label and address are looked for the end of so far and
        // no further, however many open.
        let _ = runs(&"[a](".repeat(4_000));
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
            })
            .collect();
        assert!(said.contains("deep") && said.contains('*'), "{said}");
    }
}
