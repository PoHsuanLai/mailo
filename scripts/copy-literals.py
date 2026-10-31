"""Count the visible string literals in the window's code outside copy/.

All the words the window shows live in crates/mail-ui-model/src/copy/, by kind. This finds the
string literals that still say something to a person elsewhere under crates/mail-ui/src/ui and
in the model's modules that word the window (not its settings files), so
scripts/check-copy.sh can hold their number to a ratchet (scripts/copy-literals.txt).

What is a visible literal is a heuristic, kept simple and deterministic:

- Not counted at all: comments, byte strings, the copy/ directory itself, test code (files named tests.rs or
  *_tests.rs, anything under a tests/ or fixtures/ directory, a file a `#[cfg(test)] mod x;`
  declares, and the item that follows any `#[cfg(test)]`).
- Not visible by where it stands: inside an `impl Debug for` block, inside an attribute
  (`#[path = "..."]`), inside a log, panic or
  assert macro or an `.expect(...)`, as the argument of a string test (`starts_with`, `contains`,
  `strip_prefix`, ...), as a match pattern (`"Escape" =>`, `"a" | "b"`), beside `==` or `!=`, or
  as the value of a markup property that is never read out (`class:`, `key:`, `id:`, `role:`,
  `href:`, `style:`, ...).
- Not visible by what it says: nothing of two letters together once `{...}` placeholders and
  escapes are gone; one token of lowercase, digits and `-_.:/#@=` (a class, a key, an id, a path,
  an event name, a search operator); lowercase words joined by `-` or `_` (a class list); one
  token in capitals or in camelCase (an environment variable, a region code, a command name); a
  CSS selector or CSS (`px`, `var(`, `:` with `;`); a URL; a locale code; a DOM key name; a font;
  a key chord as the keymap spells it (`Primary+Shift+F`, `Alt+Primary+C`, `Primary+]`).

Everything else is counted: a capitalised word ("Inbox"), any words with a space between them
("no mail from them"), anything with typographic marks ("Settings…").

Usage: copy-literals.py            print "<file> <count>" for every file with one, as the list
       copy-literals.py --show F   print each visible literal in F with its line
Run with python3 -I. Reads files only.
"""

import os
import re
import sys

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
UI = os.path.join("crates", "mail-ui", "src", "ui")
MODEL = os.path.join("crates", "mail-ui-model", "src")
COPY = os.path.join(MODEL, "copy")
# What is walked: the window's code, and the model beside it. The model's settings files are
# schema and file names, not the window's words, and its `damage` is the command line's too.
ROOTS = [UI, MODEL]
LEFT_OUT = [COPY, os.path.join(MODEL, "settings"), os.path.join(MODEL, "damage")]

# Calls whose string arguments are not shown in the window.
QUIET_CALLS = {
    "debug!", "info!", "warn!", "error!", "trace!", "log!", "panic!", "unreachable!", "todo!",
    "unimplemented!", "assert!", "assert_eq!", "assert_ne!", "debug_assert!",
    "debug_assert_eq!", "debug_assert_ne!", "println!", "eprintln!", "print!", "eprint!",
    "include_str!", "include_bytes!", "env!", "option_env!", "expect", "expect_err",
    "starts_with", "ends_with", "contains", "strip_prefix", "strip_suffix", "split",
    "split_once", "rsplit_once", "find", "rfind", "trim_start_matches", "trim_end_matches",
    "matches", "eq_ignore_ascii_case", "classed", "tagged", "get_attribute", "set_attribute", "var", "var_os",
    "or_log", "or_log_default",
}

# Markup and model properties whose value is never read out.
QUIET_PROPS = re.compile(
    r'(?:\b(?:class|key|id|role|href|src|style|r#type|r#for|lang|tabindex|autocomplete|'
    r'inputmode|draggable|spellcheck|dir|target|rel|for_id|class_name|test_id)|"data-[a-z-]+")'
    r'\s*:\s*$'
)

DOM_KEYS = {
    "Escape", "Enter", "Tab", "Backspace", "Delete", "Insert", "Home", "End", "PageUp",
    "PageDown", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Shift", "Control", "Alt",
    "Meta", "CapsLock", "Space", "Dead", "AltGraph", "Super", "Hyper", "NumLock", "ScrollLock",
    "Fn", "FnLock", "Symbol", "SymbolLock", "Unidentified", "Process", "ContextMenu", "OS",
}

ID_LIKE = re.compile(r"^[a-z0-9_\-.:/#@=+*?&%,()\[\]!<>]*$")
CLASS_LIST = re.compile(r"^[a-z0-9]+(?:[-_][a-z0-9]+)+(?:\s+[a-z0-9]+(?:[-_][a-z0-9]+)*)*$|^[a-z0-9]+(?:\s+[a-z0-9]+(?:[-_][a-z0-9]+)+)+$")
LOCALE = re.compile(r"^[a-z]{2,3}(?:[-_][A-Za-z]{2,4})*$")
CONSTANT = re.compile(r"^[A-Z0-9_]+$|^[a-z]+(?:[A-Z][a-z0-9]*)+$")
SELECTOR = re.compile(r"^[.#][a-z]|\[[a-z-]+=|^[a-z]+\[")
FONT = re.compile(
    r"^(?:Noto |PingFang|Hiragino|Microsoft (?:JhengHei|YaHei)|Yu (?:Gothic|Mincho)|SimSun|Songti|"
    r"P?MingLiU|Batang|Malgun|Apple ?(?:SD|Myungjo)|Georgia$|Times New Roman|Helvetica|Arial|"
    r"Segoe|Menlo|Consolas|Courier|DejaVu|Liberation|Cantarell|Inter$)"
)
CHORD = re.compile(
    r"^(?:(?:Primary|Shift|Alt|Ctrl|Control|Super|Meta|Cmd|Option)\+)+(?:[A-Za-z0-9]+|[^\sA-Za-z0-9])$"
)
# The head of an `impl Debug for T` block, up to its `{`: what it writes is for a developer.
DEBUG_IMPL = re.compile(r"\bimpl\b[^{};]*\bDebug\s+for\b[^{};]*$")
PLACEHOLDER = re.compile(r"\{[^{}]*\}")
ESCAPE = re.compile(r"\\(?:u\{[0-9a-fA-F]+\}|x[0-9a-fA-F]{2}|.)")


def visible_text(text):
    """Whether a literal's words are ones a person reads."""
    bare = PLACEHOLDER.sub("", text.replace("{{", "").replace("}}", ""))
    plain = ESCAPE.sub(" ", bare).strip()
    if not re.search(r"[^\W\d_]{2}", plain):
        return False
    if ID_LIKE.match(plain) or CLASS_LIST.match(plain) or LOCALE.match(plain):
        return False
    if CONSTANT.match(plain) or SELECTOR.search(plain) or FONT.match(plain):
        return False
    if plain in DOM_KEYS or CHORD.match(plain):
        return False
    if "://" in plain or plain.startswith(("mailto:", "data:")):
        return False
    if re.search(r"\d(px|em|rem|ms|%)\b|var\(--|calc\(", plain) or (":" in plain and ";" in plain):
        return False
    return True


class Lexer:
    """The string literals of one Rust file, with whether each stands where it is shown."""

    def __init__(self, src):
        self.s = src
        self.i = 0
        self.code = []  # code text seen so far, comments and literals left out
        self.frames = []  # (opener, callee) for each open bracket
        self.found = []  # (line, text)

    def line(self, at):
        return self.s.count("\n", 0, at) + 1

    def callee(self):
        tail = self.tail(80)
        m = re.search(r"([A-Za-z_][A-Za-z0-9_]*!?)\s*$", tail)
        return m.group(1) if m else ""

    def tail(self, n):
        out = []
        size = 0
        for piece in reversed(self.code):
            out.append(piece)
            size += len(piece)
            if size >= n:
                break
        return "".join(reversed(out))[-n:]

    def run(self):
        s = self.s
        n = len(s)
        skip_item = False  # a #[cfg(test)] was seen and its item has not started
        skipping = None  # bracket depth the skipped item started at
        while self.i < n:
            c = s[self.i]
            # Comments.
            if s.startswith("//", self.i):
                end = s.find("\n", self.i)
                self.i = n if end < 0 else end
                continue
            if s.startswith("/*", self.i):
                depth = 0
                while self.i < n:
                    if s.startswith("/*", self.i):
                        depth += 1
                        self.i += 2
                    elif s.startswith("*/", self.i):
                        depth -= 1
                        self.i += 2
                        if depth == 0:
                            break
                    else:
                        self.i += 1
                continue
            # Raw and byte strings, then plain strings.
            m = re.compile(r'(b?)r(#*)"').match(s, self.i)
            if m and (self.i == 0 or not (s[self.i - 1].isalnum() or s[self.i - 1] == "_")):
                hashes = m.group(2)
                start = m.end()
                end = s.find('"' + hashes, start)
                end = n if end < 0 else end
                if not m.group(1):
                    self.literal(self.i, s[start:end], end + 1 + len(hashes), skipping)
                self.i = end + 1 + len(hashes)
                continue
            if c == '"' or (c == "b" and s.startswith('b"', self.i) and not self.ident_before()):
                byte = c == "b"
                start = self.i + (2 if byte else 1)
                j = start
                while j < n and s[j] != '"':
                    j += 2 if s[j] == "\\" else 1
                if not byte:
                    self.literal(self.i, s[start:j], j + 1, skipping)
                self.i = j + 1
                continue
            if c == "'":
                # A char literal, or a lifetime or label.
                if s.startswith("\\", self.i + 1):
                    j = self.i + 2
                    while j < n and s[j] != "'":
                        j += 1
                    self.i = j + 1
                    continue
                if self.i + 2 < n and s[self.i + 2] == "'":
                    self.i += 3
                    continue
                self.code.append(c)
                self.i += 1
                continue
            # Attributes.
            if s.startswith("#[", self.i) or s.startswith("#![", self.i):
                open_at = s.index("[", self.i)
                depth = 0
                j = open_at
                while j < n:
                    if s[j] == '"':
                        j += 1
                        while j < n and s[j] != '"':
                            j += 2 if s[j] == "\\" else 1
                    elif s[j] == "[":
                        depth += 1
                    elif s[j] == "]":
                        depth -= 1
                        if depth == 0:
                            break
                    j += 1
                attr = re.sub(r"\s+", "", s[open_at + 1 : j])
                if skipping is None and re.match(r"cfg\((?:all\()?test\b", attr):
                    skip_item = True
                self.i = j + 1
                continue
            if c in "([{":
                if skip_item and skipping is None and c == "{":
                    skipping = len(self.frames)
                    skip_item = False
                elif skipping is None and c == "{" and DEBUG_IMPL.search(self.tail(200)):
                    skipping = len(self.frames)  # what a Debug impl writes is never shown
                self.frames.append((c, self.callee()))
            elif c in ")]}":
                if self.frames:
                    self.frames.pop()
                if skipping is not None and c == "}" and len(self.frames) == skipping:
                    skipping = None
            elif c == ";" and skip_item and skipping is None:
                skip_item = False  # `#[cfg(test)] mod x;` or a one-line item: nothing to skip
            self.code.append(c)
            self.i += 1
        return self.found

    def ident_before(self):
        return self.i > 0 and (self.s[self.i - 1].isalnum() or self.s[self.i - 1] == "_")

    def literal(self, at, text, after, skipping):
        if skipping is not None:
            return
        if any(callee in QUIET_CALLS for _, callee in self.frames):
            return
        before = self.tail(60)
        if QUIET_PROPS.search(before) or re.search(r"(==|!=)\s*&?$", before):
            return
        rest = self.s[after : after + 8].lstrip()
        if rest.startswith(("=>", "|", "==", "!=")) and not rest.startswith("||"):
            return
        if visible_text(text):
            self.found.append((self.line(at), text))


def test_files():
    """Files a `#[cfg(test)] mod name;` declares, which are test code whatever they are called."""
    out = set()
    pattern = re.compile(
        r"#\[cfg\(test\)\]\s*(?:#\[path\s*=\s*\"([^\"]+)\"\]\s*)?(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;"
    )
    for path in rust_files(include_tests=True):
        with open(os.path.join(ROOT, path), encoding="utf-8") as f:
            src = f.read()
        here = os.path.dirname(path)
        stem = os.path.splitext(os.path.basename(path))[0]
        owns = here if stem in ("mod", "lib", "main") else os.path.join(here, stem)
        for m in pattern.finditer(src):
            if m.group(1):
                out.add(os.path.normpath(os.path.join(here, m.group(1))))
            else:
                out.add(os.path.join(owns, m.group(2) + ".rs"))
                out.add(os.path.join(owns, m.group(2), "mod.rs"))
    return out


def rust_files(include_tests=False):
    for root in ROOTS:
        yield from rust_files_under(root, include_tests)


def rust_files_under(root, include_tests):
    for base, dirs, files in os.walk(os.path.join(ROOT, root)):
        rel = os.path.relpath(base, ROOT)
        dirs[:] = sorted(
            d for d in dirs
            if include_tests or d not in ("tests", "fixtures")
        )
        if any(rel == gone or rel.startswith(gone + os.sep) for gone in LEFT_OUT):
            continue
        for name in sorted(files):
            if not name.endswith(".rs"):
                continue
            if os.path.join(rel, name) == os.path.join(MODEL, "damage.rs"):
                continue
            if not include_tests and (name == "tests.rs" or name.endswith("_tests.rs")):
                continue
            yield os.path.join(rel, name)


def counted():
    tests = test_files()
    out = []
    for path in rust_files():
        if path in tests:
            continue
        with open(os.path.join(ROOT, path), encoding="utf-8") as f:
            found = Lexer(f.read()).run()
        if found:
            out.append((path, found))
    return out


def main(argv):
    if len(argv) == 3 and argv[1] == "--show":
        want = os.path.normpath(argv[2])
        for path, found in counted():
            if path == want:
                for line, text in found:
                    print(f"{path}:{line}: \"{text}\"")
        return 0
    if len(argv) != 1:
        print(__doc__, file=sys.stderr)
        return 2
    for path, found in counted():
        print(f"{path} {len(found)}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
