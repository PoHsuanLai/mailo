//! The quoted tail of a reply, folded: what a message quotes of the one it answers is put behind
//! a small "···" toggle in the frame's document, so a conversation reads as what each person
//! wrote. Opening the toggle is the document's own (`<details>`, which Blitz opens and closes
//! on a click of its `<summary>`), so the reader keeps no state for it.
//!
//! Only a tail is folded: the last thing a message holds, when it is a quote (a `<blockquote>`,
//! a mail client's quote container, an Outlook reply header and what follows it, or a run of
//! `>` lines), with the line that introduces it ("On … wrote:") folded with it. A quote with
//! the reply written under or between its parts is not a tail and is left as it is, and so is a
//! message that is nothing but a quote: folding it would leave nothing to read.
//!
//! Pure functions of the text, so every case is a table test.

/// The toggle's face: three middle dots, as the composer's quote marker reads.
const TOGGLE: &str = "\u{b7}\u{b7}\u{b7}";

/// The toggle's tooltip in the frame's document, which cannot reach the window's copy.
const TOGGLE_TIP: &str = "Show Quoted Text";

/// What opens the fold, placed where the quoted tail starts.
fn opening() -> String {
    format!(
        "<details class=\"mailo-quote\"><summary title=\"{TOGGLE_TIP}\" \
         aria-label=\"{TOGGLE_TIP}\">{TOGGLE}</summary><div class=\"mailo-quoted\">"
    )
}

/// What closes the fold, after the quoted tail. The tail sits in an element of its own: a closed
/// `<details>` hides the elements after its `<summary>`, never bare text, on Blitz as in the spec's
/// rendering rules, so a plain message's quote would otherwise stay in view.
const CLOSING: &str = "</div></details>";

/// The rules for the toggle, in the frame's own sheet: a small quiet pill on its own line, and
/// the quote under it once open. `ink` is the soft ink of the sheet it joins.
pub(super) fn sheet(ink: &str) -> String {
    format!(
        "details.mailo-quote {{ margin: 0; }} \
         details.mailo-quote > summary {{ display: inline-block; list-style: none; cursor: pointer; \
         margin: 6px 0; padding: 0 7px; border-radius: 6px; background: rgba(127,127,127,.16); \
         color: {ink}; font-size: 11px; line-height: 15px; letter-spacing: 1px; }} \
         details.mailo-quote[open] > summary {{ margin-bottom: 10px; }}"
    )
}

/// Words a line that introduces a quote ends in, before its colon, in the languages mail
/// clients write them in: "On … wrote:", "Am … schrieb:", "Le … a écrit :".
const WROTE: [&str; 12] = [
    "wrote",
    "schrieb",
    "écrit",
    "escribió",
    "scritto",
    "skrev",
    "schreef",
    "napisał",
    "escreveu",
    "kirjoitti",
    "写道",
    "寫道",
];

/// Whether `line` introduces a quote: it ends in a colon (either width) after one of [`WROTE`],
/// or after German's "schrieb" and the name German puts after it ("Am … schrieb Ada:"), and is
/// short enough to be a line rather than a paragraph.
fn introduces(line: &str) -> bool {
    let line = line.trim();
    let Some(head) = line.strip_suffix(':').or_else(|| line.strip_suffix('：')) else {
        return false;
    };
    let head = head.trim_end().to_lowercase();
    line.chars().count() <= 400
        && (WROTE.iter().any(|word| head.ends_with(word)) || head.contains(" schrieb "))
}

/// Whether `line` is Outlook's divider over the message it quotes.
fn outlook_divider(line: &str) -> bool {
    let line = line.trim();
    let inner = line.trim_matches('-').trim();
    line.starts_with("--") && line.ends_with("--") && inner.eq_ignore_ascii_case("original message")
}

/// Where the quoted tail of plain `text` starts, a byte offset at the start of a line: a run of
/// `>` lines at the end (blank lines among and after them), with the line or two before it that
/// introduce it, or Outlook's divider and everything under it. `None` when there is no such tail
/// or nothing above it.
pub(super) fn plain_tail(text: &str) -> Option<usize> {
    let lines: Vec<(usize, &str)> = text
        .split_inclusive('\n')
        .scan(0, |at, line| {
            let start = *at;
            *at += line.len();
            Some((start, line))
        })
        .collect();
    let blank = |line: &str| line.trim().is_empty();
    let quoted = |line: &str| line.trim_start().starts_with('>');
    let written_above = |index: usize| lines[..index].iter().any(|(_, line)| !blank(line));

    // A run of `>` lines at the end.
    let mut first = lines.len();
    let mut any = false;
    for (index, (_, line)) in lines.iter().enumerate().rev() {
        if quoted(line) {
            first = index;
            any = true;
        } else if !blank(line) {
            break;
        }
    }
    let marked = any.then(|| {
        // The line or two that introduce it ("On Mon, 3 Jun 2026, Ada Lovelace\n<ada@x> wrote:").
        let mut above = first;
        while above > 0 && blank(lines[above - 1].1) {
            above -= 1;
        }
        let Some(one) = above.checked_sub(1) else {
            return first;
        };
        if !introduces(lines[one].1) {
            return first;
        }
        // A client that wraps the line leaves its first half above, unfinished (no full stop,
        // no colon, as "On Mon, 3 Jun 2026 at 10:00, Ada Lovelace" is), and the second half
        // starts mid-sentence: with the address's `<`, or a small letter.
        let continues = lines[one]
            .1
            .trim_start()
            .starts_with(|c: char| c == '<' || c.is_lowercase());
        let wrapped = continues && one > 0 && {
            let half = lines[one - 1].1.trim();
            !half.is_empty() && !half.ends_with(['.', '!', '?', ':', '：'])
        };
        if wrapped { one - 1 } else { one }
    });
    let divider = lines.iter().position(|(_, line)| outlook_divider(line));
    let start = match (marked, divider) {
        (Some(a), Some(b)) => a.min(b),
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (None, None) => return None,
    };
    written_above(start).then(|| lines[start].0)
}

/// Plain `text` as the frame's markup, escaped by `escape`, with its quoted tail folded.
pub(super) fn fold_plain(text: &str, escape: impl Fn(&str) -> String) -> String {
    match plain_tail(text) {
        Some(at) => format!(
            "{}{}{}{CLOSING}",
            escape(&text[..at]),
            opening(),
            escape(&text[at..])
        ),
        None => escape(text),
    }
}

/// `html` (sanitized markup, as the frame shows it) with its quoted tail folded.
pub(super) fn fold_html(html: &str) -> String {
    match html_tail(html) {
        Some((start, end)) => format!(
            "{}{}{}{CLOSING}{}",
            &html[..start],
            opening(),
            &html[start..end],
            &html[end..]
        ),
        None => html.to_owned(),
    }
}

/// One node of sanitized markup: where it is, and for an element its name, its opening tag and
/// its children.
#[derive(Debug)]
struct Node {
    /// Where it starts: its opening tag, or its first character.
    start: usize,
    /// Where it ends: after its closing tag, or its last character.
    end: usize,
    element: Option<Element>,
    children: Vec<usize>,
}

#[derive(Debug)]
struct Element {
    /// Its name, in lowercase.
    name: String,
    /// Its opening tag, whole.
    tag: std::ops::Range<usize>,
}

/// Elements with no content and no closing tag.
const VOID: [&str; 13] = [
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr",
];

/// Elements whose content is text up to their closing tag, never markup.
const RAW: [&str; 4] = ["style", "script", "textarea", "title"];

/// Elements a fold may be placed inside: blocks that hold blocks. Never a paragraph or a phrase,
/// where a `<details>` would be moved out by the parser and the fold come apart.
const CONTAINERS: [&str; 7] = [
    "body", "div", "section", "article", "main", "center", "font",
];

/// Elements that say nothing on their own when they hold nothing: a line break, or a block whose
/// content is all blank.
const SPACERS: [&str; 5] = ["div", "p", "span", "font", "center"];

/// The nodes of `html`, and the top level's. Sanitized markup is well formed: every element is
/// closed or void, every attribute quoted, and no comment is left; what is not (an unclosed
/// element) ends where the markup does.
fn parse(html: &str) -> (Vec<Node>, Vec<usize>) {
    let bytes = html.as_bytes();
    let mut nodes: Vec<Node> = Vec::new();
    let mut top: Vec<usize> = Vec::new();
    let mut open: Vec<usize> = Vec::new();
    let mut at = 0;
    let push = |nodes: &mut Vec<Node>, top: &mut Vec<usize>, open: &[usize], node: Node| {
        let index = nodes.len();
        nodes.push(node);
        match open.last() {
            Some(parent) => nodes[*parent].children.push(index),
            None => top.push(index),
        }
        index
    };
    while at < bytes.len() {
        if bytes[at] != b'<' {
            let end = html[at..].find('<').map_or(html.len(), |next| at + next);
            push(
                &mut nodes,
                &mut top,
                &open,
                Node {
                    start: at,
                    end,
                    element: None,
                    children: Vec::new(),
                },
            );
            at = end;
            continue;
        }
        let rest = &html[at..];
        if rest.starts_with("<!--") {
            at = rest.find("-->").map_or(html.len(), |end| at + end + 3);
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            at = rest.find('>').map_or(html.len(), |end| at + end + 1);
            continue;
        }
        if let Some(closing) = rest.strip_prefix("</") {
            let name = tag_name(closing);
            let end = rest.find('>').map_or(html.len(), |end| at + end + 1);
            if let Some(depth) = open.iter().rposition(|index| {
                nodes[*index]
                    .element
                    .as_ref()
                    .is_some_and(|el| el.name == name)
            }) {
                for index in open.drain(depth..) {
                    nodes[index].end = end;
                }
            }
            at = end;
            continue;
        }
        if !rest[1..].starts_with(|c: char| c.is_ascii_alphabetic()) {
            // A `<` that opens nothing is text.
            let end = rest[1..].find('<').map_or(html.len(), |next| at + 1 + next);
            push(
                &mut nodes,
                &mut top,
                &open,
                Node {
                    start: at,
                    end,
                    element: None,
                    children: Vec::new(),
                },
            );
            at = end;
            continue;
        }
        let name = tag_name(&rest[1..]);
        let tag_end = at + tag_length(rest);
        let index = push(
            &mut nodes,
            &mut top,
            &open,
            Node {
                start: at,
                end: tag_end,
                element: Some(Element {
                    name: name.clone(),
                    tag: at..tag_end,
                }),
                children: Vec::new(),
            },
        );
        let closed_by_itself = html[at..tag_end].ends_with("/>");
        if VOID.contains(&name.as_str()) || closed_by_itself {
            at = tag_end;
        } else if RAW.contains(&name.as_str()) {
            let close = format!("</{name}");
            let end =
                html[tag_end..]
                    .to_ascii_lowercase()
                    .find(&close)
                    .map_or(html.len(), |found| {
                        let from = tag_end + found;
                        html[from..]
                            .find('>')
                            .map_or(html.len(), |gt| from + gt + 1)
                    });
            nodes[index].end = end;
            at = end;
        } else {
            open.push(index);
            at = tag_end;
        }
    }
    for index in open {
        nodes[index].end = html.len();
    }
    (nodes, top)
}

/// The name a tag starts with, in lowercase.
fn tag_name(after_bracket: &str) -> String {
    after_bracket
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == ':')
        .collect::<String>()
        .to_ascii_lowercase()
}

/// How long the tag `rest` starts with is, through its `>`, past any `>` inside a quoted value.
fn tag_length(rest: &str) -> usize {
    let mut quote = None;
    for (at, c) in rest.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(open), c) if c == open => quote = None,
            (None, '>') => return at + 1,
            _ => {}
        }
    }
    rest.len()
}

/// The value of attribute `name` in `tag`, as sanitized markup writes it (`name="value"`).
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!(" {name}=\"");
    let start = tag.find(&needle)? + needle.len();
    tag[start..].split('"').next()
}

/// The text a node holds, its entities for spaces and the markup's own characters read.
fn text_of(html: &str, nodes: &[Node], index: usize) -> String {
    let node = &nodes[index];
    match &node.element {
        None => html[node.start..node.end]
            .replace("&nbsp;", " ")
            .replace("&#160;", " ")
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&#39;", "'"),
        Some(element) if element.name == "br" => "\n".to_owned(),
        Some(_) => node
            .children
            .iter()
            .map(|child| text_of(html, nodes, *child))
            .collect(),
    }
}

/// Whether a node says nothing: blank text, a line break, or a spacer holding only such.
fn blank(html: &str, nodes: &[Node], index: usize) -> bool {
    let node = &nodes[index];
    match &node.element {
        None => text_of(html, nodes, index).trim().is_empty(),
        Some(element) if matches!(element.name.as_str(), "br" | "wbr") => true,
        Some(element) if SPACERS.contains(&element.name.as_str()) => {
            node.children.iter().all(|child| blank(html, nodes, *child))
        }
        Some(_) => false,
    }
}

/// The classes mail clients put round the message a reply quotes.
const QUOTE_CLASSES: [&str; 4] = [
    "gmail_quote",
    "yahoo_quoted",
    "protonmail_quote",
    "moz-forward-container",
];

/// The classes mail clients put on the line that introduces a quote.
const ATTRIBUTION_CLASSES: [&str; 2] = ["gmail_attr", "moz-cite-prefix"];

/// The ids Outlook puts on the header over the message a reply quotes, which the quoted message
/// follows as siblings rather than inside.
const OUTLOOK_HEADERS: [&str; 2] = ["divRplyFwdMsg", "appendonsend"];

fn classes<'a>(html: &'a str, element: &Element) -> impl Iterator<Item = &'a str> {
    attribute(&html[element.tag.clone()], "class")
        .unwrap_or("")
        .split_whitespace()
}

/// Whether a node is a quote: a `<blockquote>`, or a block a mail client marks as one.
fn is_quote(html: &str, nodes: &[Node], index: usize) -> bool {
    let Some(element) = &nodes[index].element else {
        return false;
    };
    element.name == "blockquote"
        || classes(html, element).any(|class| QUOTE_CLASSES.contains(&class))
}

/// Whether a node introduces the quote after it: marked so by its client, or a short line ending
/// in "wrote:" or its kin.
fn is_attribution(html: &str, nodes: &[Node], index: usize) -> bool {
    if let Some(element) = &nodes[index].element
        && classes(html, element).any(|class| ATTRIBUTION_CLASSES.contains(&class))
    {
        return true;
    }
    introduces(&text_of(html, nodes, index))
}

/// Whether a node is Outlook's header over the message it quotes.
fn is_outlook_header(html: &str, nodes: &[Node], index: usize) -> bool {
    nodes[index].element.as_ref().is_some_and(|element| {
        attribute(&html[element.tag.clone()], "id").is_some_and(|id| OUTLOOK_HEADERS.contains(&id))
    })
}

/// Where the quoted tail of `html` is, as the byte range to fold: from the quote, or the line
/// introducing it, to the end of what holds it.
fn html_tail(html: &str) -> Option<(usize, usize)> {
    let (nodes, top) = parse(html);
    tail_in(html, &nodes, &top, false)
}

/// The tail among `children`, where `before` says whether something was written before them
/// further out. The last thing said is followed down through the blocks that hold it.
fn tail_in(html: &str, nodes: &[Node], children: &[usize], before: bool) -> Option<(usize, usize)> {
    let said: Vec<usize> = children
        .iter()
        .copied()
        .filter(|child| !blank(html, nodes, *child))
        .collect();
    let end = nodes[*children.last()?].end;
    // Outlook's header, and everything after it.
    if let Some(at) = said
        .iter()
        .position(|child| is_outlook_header(html, nodes, *child))
    {
        let mut first = at;
        if at > 0
            && nodes[said[at - 1]]
                .element
                .as_ref()
                .is_some_and(|element| element.name == "hr")
        {
            first = at - 1;
        }
        return (before || first > 0).then(|| (nodes[said[first]].start, end));
    }
    let (at, &last) = said.iter().enumerate().next_back()?;
    if is_quote(html, nodes, last) {
        let first = if at > 0 && is_attribution(html, nodes, said[at - 1]) {
            at - 1
        } else {
            at
        };
        return (before || first > 0).then(|| (nodes[said[first]].start, end));
    }
    let element = nodes[last].element.as_ref()?;
    if !CONTAINERS.contains(&element.name.as_str()) {
        return None;
    }
    tail_in(html, nodes, &nodes[last].children, before || at > 0)
}

#[cfg(test)]
mod tests {
    use super::{fold_html, fold_plain, html_tail, plain_tail};

    /// Where each plain text's fold starts, as the text from there.
    #[test]
    fn a_plain_reply_folds_its_quoted_tail_and_the_line_that_introduces_it() {
        let cases: &[(&str, &str, Option<&str>)] = &[
            ("no quote", "Thanks, see you then.\n", None),
            (
                "a quoted tail",
                "Sounds good.\n\n> Shall we meet at ten?\n> Ada\n",
                Some("> Shall we meet at ten?\n> Ada\n"),
            ),
            (
                "introduced",
                "Sounds good.\n\nOn Mon, 3 Jun 2026, Ada <ada@example.test> wrote:\n> Ten?\n",
                Some("On Mon, 3 Jun 2026, Ada <ada@example.test> wrote:\n> Ten?\n"),
            ),
            (
                "introduced over two lines",
                "Yes.\n\nOn Mon, 3 Jun 2026 at 10:00, Ada Lovelace\n<ada@example.test> wrote:\n\n> Ten?\n\n",
                Some(
                    "On Mon, 3 Jun 2026 at 10:00, Ada Lovelace\n<ada@example.test> wrote:\n\n> Ten?\n\n",
                ),
            ),
            (
                "in German",
                "Ja.\nAm 3. Juni 2026 schrieb Ada:\n> Zehn?\n",
                Some("Am 3. Juni 2026 schrieb Ada:\n> Zehn?\n"),
            ),
            (
                "answered between the quotes: not a tail",
                "> Ten?\nYes, ten.\n> And lunch?\nNo.\n",
                None,
            ),
            ("nothing but a quote", "> Ten?\n> Ada\n", None),
            (
                "Outlook's divider",
                "Fine by me.\n\n-----Original Message-----\nFrom: Ada\nSubject: Ten?\n\nTen?\n",
                Some("-----Original Message-----\nFrom: Ada\nSubject: Ten?\n\nTen?\n"),
            ),
            (
                "a colon that is not an introduction",
                "The plan:\n> step one\n",
                Some("> step one\n"),
            ),
        ];
        for (case, text, want) in cases {
            let got = plain_tail(text).map(|at| &text[at..]);
            assert_eq!(got, *want, "{case}");
        }
    }

    /// Where each html reply's fold starts and ends, as the markup it folds.
    #[test]
    fn an_html_reply_folds_its_quoted_tail_wherever_its_client_put_it() {
        let cases: &[(&str, &str, Option<&str>)] = &[
            ("no quote", "<p>Thanks.</p>", None),
            (
                "gmail",
                "<div dir=\"ltr\">Sounds good.</div><br><div class=\"gmail_quote\"><div class=\"gmail_attr\">On Mon, Ada wrote:<br></div><blockquote>Ten?</blockquote></div>",
                Some(
                    "<div class=\"gmail_quote\"><div class=\"gmail_attr\">On Mon, Ada wrote:<br></div><blockquote>Ten?</blockquote></div>",
                ),
            ),
            (
                "apple, nested in a block",
                "<div>Sounds good.<div><br><blockquote type=\"cite\"><div>On 3 Jun, Ada wrote:</div><div>Ten?</div></blockquote></div></div>",
                Some(
                    "<blockquote type=\"cite\"><div>On 3 Jun, Ada wrote:</div><div>Ten?</div></blockquote>",
                ),
            ),
            (
                "thunderbird, introduced by its prefix",
                "<p>Yes.</p><div class=\"moz-cite-prefix\">On 3/6/26 Ada wrote:<br></div><blockquote type=\"cite\"><p>Ten?</p></blockquote>",
                Some(
                    "<div class=\"moz-cite-prefix\">On 3/6/26 Ada wrote:<br></div><blockquote type=\"cite\"><p>Ten?</p></blockquote>",
                ),
            ),
            (
                "introduced by a line of text",
                "Yes.<br><br>On Mon, Ada &lt;ada@example.test&gt; wrote:<br><blockquote>Ten?</blockquote>\n",
                Some(
                    "On Mon, Ada &lt;ada@example.test&gt; wrote:<br><blockquote>Ten?</blockquote>\n",
                ),
            ),
            (
                "outlook",
                "<div>Fine by me.</div><hr><div id=\"divRplyFwdMsg\"><b>From:</b> Ada</div><div>Ten?</div>",
                Some("<hr><div id=\"divRplyFwdMsg\"><b>From:</b> Ada</div><div>Ten?</div>"),
            ),
            (
                "written under the quote: not a tail",
                "<blockquote>Ten?</blockquote><p>Yes, ten.</p>",
                None,
            ),
            ("nothing but a quote", "<blockquote>Ten?</blockquote>", None),
            (
                "a quote inside a paragraph is left alone",
                "<p>Yes <span><blockquote>Ten?</blockquote></span></p>",
                None,
            ),
            (
                "a style sheet's text is not markup",
                "<style>a > b { color: red }</style><p>Yes.</p><blockquote>Ten?</blockquote>",
                Some("<blockquote>Ten?</blockquote>"),
            ),
        ];
        for (case, html, want) in cases {
            let got = html_tail(html).map(|(start, end)| &html[start..end]);
            assert_eq!(got, *want, "{case}");
        }
    }

    /// The fold wraps exactly the tail, and a message without one is left as it was.
    #[test]
    fn the_fold_is_a_details_around_the_tail() {
        let html = "<p>Yes.</p><blockquote>Ten?</blockquote>";
        let folded = fold_html(html);
        assert!(
            folded.starts_with("<p>Yes.</p><details class=\"mailo-quote\"><summary")
                && folded.ends_with("<blockquote>Ten?</blockquote></div></details>"),
            "{folded}"
        );
        assert_eq!(fold_html("<p>Thanks.</p>"), "<p>Thanks.</p>");
        let plain = fold_plain("Yes.\n> Ten?\n", |text| text.replace('>', "&gt;"));
        assert!(
            plain.starts_with("Yes.\n<details class=\"mailo-quote\">")
                && plain.ends_with("&gt; Ten?\n</div></details>"),
            "{plain}"
        );
    }
}
