//! A small, dependency-free syntax highlighter for code blocks in replies. It knows the
//! keywords, comments, strings and numbers of the languages models write most, which is enough
//! for a chat and keeps the phone app small. Every shell colours the same [`TokenKind`]s.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenKind {
    Plain,
    Keyword,
    /// `true`, `false`, `null`, `None`, `self`, …
    Literal,
    String,
    Number,
    Comment,
    /// A capitalised name in languages where that means a type.
    Type,
    /// A name followed by `(`.
    Function,
    /// `diff`: an added or a removed line.
    Added,
    Removed,
}

/// A piece of a code block. The tokens of a block join back to its exact text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
    pub text: String,
    pub kind: TokenKind,
}

struct Lang {
    line_comments: &'static [&'static str],
    block_comment: Option<(&'static str, &'static str)>,
    /// Quote characters that open strings.
    quotes: &'static str,
    /// `'` only quotes a short char literal (Rust, C-likes): `'a` stays plain.
    char_quote: bool,
    triple_quotes: bool,
    keywords: &'static [&'static str],
    literals: &'static [&'static str],
    capital_types: bool,
}

const C_COMMENT: Option<(&str, &str)> = Some(("/*", "*/"));

const RUST: Lang = Lang {
    line_comments: &["//"],
    block_comment: C_COMMENT,
    quotes: "\"",
    char_quote: true,
    triple_quotes: false,
    keywords: &[
        "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "fn", "for", "if", "impl", "in", "let",
        "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "static", "struct", "super", "trait", "type", "unsafe", "use", "where",
        "while",
    ],
    literals: &["true", "false", "self", "Self", "None", "Some", "Ok", "Err"],
    capital_types: true,
};

const PYTHON: Lang = Lang {
    line_comments: &["#"],
    block_comment: None,
    quotes: "\"'",
    char_quote: false,
    triple_quotes: true,
    keywords: &[
        "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except", "finally", "for", "from",
        "global", "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with", "yield", "match",
        "case",
    ],
    literals: &["True", "False", "None", "self", "cls"],
    capital_types: true,
};

const JS: Lang = Lang {
    line_comments: &["//"],
    block_comment: C_COMMENT,
    quotes: "\"'`",
    char_quote: false,
    triple_quotes: false,
    keywords: &[
        "async",
        "await",
        "break",
        "case",
        "catch",
        "class",
        "const",
        "continue",
        "default",
        "delete",
        "do",
        "else",
        "export",
        "extends",
        "finally",
        "for",
        "from",
        "function",
        "if",
        "import",
        "in",
        "instanceof",
        "interface",
        "let",
        "new",
        "of",
        "return",
        "static",
        "switch",
        "throw",
        "try",
        "type",
        "typeof",
        "var",
        "void",
        "while",
        "yield",
        "enum",
        "implements",
        "private",
        "protected",
        "public",
        "readonly",
        "as",
    ],
    literals: &["true", "false", "null", "undefined", "this", "NaN", "Infinity"],
    capital_types: true,
};

const GO: Lang = Lang {
    line_comments: &["//"],
    block_comment: C_COMMENT,
    quotes: "\"`",
    char_quote: true,
    triple_quotes: false,
    keywords: &[
        "break",
        "case",
        "chan",
        "const",
        "continue",
        "default",
        "defer",
        "else",
        "fallthrough",
        "for",
        "func",
        "go",
        "goto",
        "if",
        "import",
        "interface",
        "map",
        "package",
        "range",
        "return",
        "select",
        "struct",
        "switch",
        "type",
        "var",
    ],
    literals: &["true", "false", "nil", "iota"],
    capital_types: false,
};

const C_LIKE: Lang = Lang {
    line_comments: &["//"],
    block_comment: C_COMMENT,
    quotes: "\"",
    char_quote: true,
    triple_quotes: false,
    keywords: &[
        "abstract",
        "auto",
        "break",
        "case",
        "catch",
        "class",
        "const",
        "continue",
        "default",
        "delete",
        "do",
        "else",
        "enum",
        "extends",
        "extern",
        "final",
        "finally",
        "for",
        "fun",
        "func",
        "goto",
        "guard",
        "if",
        "implements",
        "import",
        "in",
        "inline",
        "interface",
        "is",
        "let",
        "namespace",
        "new",
        "object",
        "override",
        "package",
        "private",
        "protected",
        "public",
        "return",
        "sealed",
        "static",
        "struct",
        "switch",
        "template",
        "throw",
        "throws",
        "try",
        "typedef",
        "typename",
        "union",
        "using",
        "val",
        "var",
        "virtual",
        "void",
        "volatile",
        "when",
        "while",
        "async",
        "await",
        "late",
        "required",
        "dynamic",
        "int",
        "char",
        "float",
        "double",
        "long",
        "short",
        "unsigned",
        "signed",
        "bool",
        "boolean",
        "byte",
        "string",
        "#include",
        "#define",
        "#ifdef",
        "#ifndef",
        "#endif",
        "#if",
        "#else",
        "#pragma",
    ],
    literals: &["true", "false", "null", "nullptr", "NULL", "this", "self", "super", "nil"],
    capital_types: true,
};

const SHELL: Lang = Lang {
    line_comments: &["#"],
    block_comment: None,
    quotes: "\"'",
    char_quote: false,
    triple_quotes: false,
    keywords: &[
        "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac", "in", "function", "return", "export", "local",
        "readonly", "set", "unset", "source", "echo", "cd", "exit", "sudo",
    ],
    literals: &["true", "false"],
    capital_types: false,
};

const DATA: Lang = Lang {
    line_comments: &["#"],
    block_comment: None,
    quotes: "\"'",
    char_quote: false,
    triple_quotes: true,
    keywords: &[],
    literals: &["true", "false", "null", "yes", "no", "on", "off", "~"],
    capital_types: false,
};

const JSON: Lang = Lang {
    line_comments: &["//"],
    block_comment: C_COMMENT,
    quotes: "\"",
    char_quote: false,
    triple_quotes: false,
    keywords: &[],
    literals: &["true", "false", "null"],
    capital_types: false,
};

const SQL: Lang = Lang {
    line_comments: &["--"],
    block_comment: C_COMMENT,
    quotes: "'\"",
    char_quote: false,
    triple_quotes: false,
    keywords: &[
        "select",
        "from",
        "where",
        "insert",
        "into",
        "values",
        "update",
        "set",
        "delete",
        "create",
        "table",
        "drop",
        "alter",
        "add",
        "index",
        "join",
        "left",
        "right",
        "inner",
        "outer",
        "on",
        "group",
        "by",
        "order",
        "having",
        "limit",
        "offset",
        "as",
        "and",
        "or",
        "not",
        "in",
        "is",
        "like",
        "primary",
        "key",
        "foreign",
        "references",
        "distinct",
        "union",
        "all",
        "case",
        "when",
        "then",
        "else",
        "end",
        "with",
        "returning",
        "default",
    ],
    literals: &["null", "true", "false"],
    capital_types: false,
};

const RUBY_LUA: Lang = Lang {
    line_comments: &["#", "--"],
    block_comment: None,
    quotes: "\"'",
    char_quote: false,
    triple_quotes: false,
    keywords: &[
        "def", "end", "class", "module", "if", "elsif", "elseif", "else", "unless", "while", "until", "for", "in", "do", "then", "return", "yield",
        "begin", "rescue", "ensure", "require", "local", "function", "and", "or", "not", "repeat",
    ],
    literals: &["true", "false", "nil", "self"],
    capital_types: true,
};

const CSS: Lang = Lang {
    line_comments: &[],
    block_comment: C_COMMENT,
    quotes: "\"'",
    char_quote: false,
    triple_quotes: false,
    keywords: &["@media", "@import", "@keyframes", "!important"],
    literals: &[],
    capital_types: false,
};

const MARKUP: Lang = Lang {
    line_comments: &[],
    block_comment: Some(("<!--", "-->")),
    quotes: "\"'",
    char_quote: false,
    triple_quotes: false,
    keywords: &[],
    literals: &[],
    capital_types: false,
};

fn lang(name: &str) -> Option<&'static Lang> {
    Some(match name.to_ascii_lowercase().as_str() {
        "rust" | "rs" => &RUST,
        "python" | "py" | "python3" => &PYTHON,
        "javascript" | "js" | "jsx" | "mjs" | "cjs" | "typescript" | "ts" | "tsx" => &JS,
        "go" | "golang" => &GO,
        "c" | "h" | "cpp" | "c++" | "cc" | "cxx" | "hpp" | "java" | "kotlin" | "kt" | "swift" | "dart" | "csharp" | "cs" | "c#" | "scala" | "zig"
        | "objc" | "php" => &C_LIKE,
        "sh" | "bash" | "shell" | "zsh" | "fish" | "console" | "ps1" | "powershell" => &SHELL,
        "toml" | "yaml" | "yml" | "ini" | "conf" | "nix" | "dockerfile" | "make" | "makefile" => &DATA,
        "json" | "jsonc" | "json5" => &JSON,
        "sql" => &SQL,
        "ruby" | "rb" | "lua" => &RUBY_LUA,
        "css" | "scss" | "less" => &CSS,
        "html" | "xml" | "svg" | "vue" => &MARKUP,
        _ => return None,
    })
}

/// Splits `code` into tokens for `lang` (a fence's language). An unknown or missing language is
/// one plain token.
pub fn highlight(lang_name: Option<&str>, code: &str) -> Vec<Token> {
    let name = lang_name.unwrap_or("").to_ascii_lowercase();
    if name == "diff" || name == "patch" {
        return diff(code);
    }
    match lang(&name) {
        Some(l) => Lexer { l, src: code, out: Vec::new() }.run(),
        None if code.is_empty() => Vec::new(),
        None => vec![Token { text: code.to_string(), kind: TokenKind::Plain }],
    }
}

fn diff(code: &str) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::new();
    for line in code.split_inclusive('\n') {
        let kind = if line.starts_with("+++") || line.starts_with("---") {
            TokenKind::Keyword
        } else if line.starts_with('+') {
            TokenKind::Added
        } else if line.starts_with('-') {
            TokenKind::Removed
        } else if line.starts_with("@@") {
            TokenKind::Comment
        } else {
            TokenKind::Plain
        };
        push(&mut out, line, kind);
    }
    out
}

fn push(out: &mut Vec<Token>, text: &str, kind: TokenKind) {
    if text.is_empty() {
        return;
    }
    match out.last_mut() {
        Some(t) if t.kind == kind => t.text.push_str(text),
        _ => out.push(Token { text: text.to_string(), kind }),
    }
}

struct Lexer<'a> {
    l: &'static Lang,
    src: &'a str,
    out: Vec<Token>,
}

impl Lexer<'_> {
    fn run(mut self) -> Vec<Token> {
        let s = self.src;
        let mut i = 0;
        while i < s.len() {
            let rest = &s[i..];
            let c = rest.chars().next().unwrap_or(' ');
            // Comments.
            if self.l.line_comments.iter().any(|p| rest.starts_with(p)) {
                let end = rest.find('\n').unwrap_or(rest.len());
                push(&mut self.out, &rest[..end], TokenKind::Comment);
                i += end;
                continue;
            }
            if let Some((open, close)) = self.l.block_comment
                && rest.starts_with(open)
            {
                let end = rest[open.len()..].find(close).map(|e| open.len() + e + close.len()).unwrap_or(rest.len());
                push(&mut self.out, &rest[..end], TokenKind::Comment);
                i += end;
                continue;
            }
            // Strings.
            if self.l.triple_quotes && (rest.starts_with("\"\"\"") || rest.starts_with("'''")) {
                let q = &rest[..3];
                let end = rest[3..].find(q).map(|e| 3 + e + 3).unwrap_or(rest.len());
                push(&mut self.out, &rest[..end], TokenKind::String);
                i += end;
                continue;
            }
            if self.l.quotes.contains(c) || (c == '\'' && self.l.char_quote) {
                if c == '\'' && self.l.char_quote && !self.l.quotes.contains('\'') {
                    // A char literal: 'a', '\n', '\u{1F600}'. Otherwise ('a lifetime) plain.
                    let body = &rest[1..];
                    let len = if let Some(esc) = body.strip_prefix('\\') {
                        esc.find('\'').map(|e| e + 2)
                    } else {
                        body.chars().next().map(|ch| ch.len_utf8())
                    };
                    if let Some(len) = len
                        && body[len.min(body.len())..].starts_with('\'')
                        && len <= 12
                    {
                        push(&mut self.out, &rest[..len + 2], TokenKind::String);
                        i += len + 2;
                        continue;
                    }
                    push(&mut self.out, "'", TokenKind::Plain);
                    i += 1;
                    continue;
                }
                let end = string_end(rest, c);
                push(&mut self.out, &rest[..end], TokenKind::String);
                i += end;
                continue;
            }
            // Numbers.
            if c.is_ascii_digit() {
                let end = rest.find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '.' || ch == '_')).unwrap_or(rest.len());
                push(&mut self.out, &rest[..end], TokenKind::Number);
                i += end;
                continue;
            }
            // Words.
            if c.is_alphabetic() || c == '_' || c == '@' || c == '#' || (c == '!' && rest.starts_with("!important")) {
                let first = c.len_utf8();
                let end = rest[first..]
                    .find(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '-' && self.is_css()))
                    .map(|e| e + first)
                    .unwrap_or(rest.len());
                let word = &rest[..end];
                let lower = word.to_ascii_lowercase();
                let kind = if self.l.keywords.contains(&word) || (std::ptr::eq(self.l, &SQL) && self.l.keywords.contains(&lower.as_str())) {
                    TokenKind::Keyword
                } else if self.l.literals.contains(&word) {
                    TokenKind::Literal
                } else if rest[end..].starts_with('(') && !std::ptr::eq(self.l, &SQL) {
                    TokenKind::Function
                } else if self.l.capital_types && word.chars().next().is_some_and(|ch| ch.is_uppercase()) && word.chars().any(|ch| ch.is_lowercase())
                {
                    TokenKind::Type
                } else {
                    TokenKind::Plain
                };
                push(&mut self.out, word, kind);
                i += end;
                continue;
            }
            push(&mut self.out, &rest[..c.len_utf8()], TokenKind::Plain);
            i += c.len_utf8();
        }
        self.out
    }

    fn is_css(&self) -> bool {
        std::ptr::eq(self.l, &CSS)
    }
}

/// The end of a string starting at `rest[0] == q`: after the closing quote, honouring
/// backslashes; at the line's end for an unclosed `"`/`'` (a backtick string may span lines).
fn string_end(rest: &str, q: char) -> usize {
    let mut escaped = false;
    for (i, ch) in rest.char_indices().skip(1) {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '\n' if q != '`' => return i,
            c if c == q => return i + c.len_utf8(),
            _ => {}
        }
    }
    rest.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(lang: &str, code: &str) -> Vec<(String, TokenKind)> {
        highlight(Some(lang), code).into_iter().filter(|t| t.kind != TokenKind::Plain).map(|t| (t.text, t.kind)).collect()
    }

    #[test]
    fn rust() {
        let code = "fn main() {\n    let s: &'static str = \"hi\"; // greet\n    println!(\"{}\", 'x');\n}";
        let toks = highlight(Some("rust"), code);
        assert_eq!(toks.iter().map(|t| t.text.as_str()).collect::<String>(), code);
        let k = kinds("rust", code);
        assert!(k.contains(&("fn".into(), TokenKind::Keyword)));
        assert!(k.contains(&("main".into(), TokenKind::Function)));
        assert!(k.contains(&("\"hi\"".into(), TokenKind::String)));
        assert!(k.contains(&("// greet".into(), TokenKind::Comment)));
        assert!(k.contains(&("'x'".into(), TokenKind::String)));
        assert!(!k.iter().any(|(t, _)| t.starts_with("'static")), "a lifetime isn't a string: {k:?}");
    }

    #[test]
    fn python_and_unknown() {
        let k = kinds("py", "def f(x=1):\n    \"\"\"doc\"\"\"\n    return None  # no");
        assert!(k.contains(&("def".into(), TokenKind::Keyword)));
        assert!(k.contains(&("\"\"\"doc\"\"\"".into(), TokenKind::String)));
        assert!(k.contains(&("1".into(), TokenKind::Number)));
        assert!(k.contains(&("None".into(), TokenKind::Literal)));
        assert!(k.contains(&("# no".into(), TokenKind::Comment)));
        assert_eq!(highlight(Some("brainfuck"), "+-<>"), vec![Token { text: "+-<>".into(), kind: TokenKind::Plain }]);
        assert_eq!(highlight(None, ""), vec![]);
    }

    #[test]
    fn diff_lines() {
        let k = kinds("diff", "@@ -1 +1 @@\n-old\n+new\n same");
        assert_eq!(k[1], ("-old\n".into(), TokenKind::Removed));
        assert_eq!(k[2], ("+new\n".into(), TokenKind::Added));
    }

    #[test]
    fn unclosed_things_end_cleanly() {
        for (l, c) in [("js", "const a = `multi\nline"), ("c", "/* open"), ("py", "'''open"), ("rust", "let c = '")] {
            let toks = highlight(Some(l), c);
            assert_eq!(toks.iter().map(|t| t.text.as_str()).collect::<String>(), c);
        }
    }
}
