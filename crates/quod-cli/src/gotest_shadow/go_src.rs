//! Just enough Go lexing for `quod gotest`: a file's header (what `go list`
//! reads of it: build constraints, package clause, imports, `//go:embed`),
//! its shape (everything outside function bodies, and each top-level
//! function's body), and test-name normalization. Anything this lexer does
//! not understand is `None`, and the caller takes the slow, exact path.

#[derive(Debug, Clone, Copy, PartialEq)]
enum Tk {
    Ident,
    Lit,
    Op,
    Comment,
}

#[derive(Debug, Clone, Copy)]
struct Tok {
    kind: Tk,
    start: usize,
    end: usize,
    line: u32,
    col: u32,
    /// A newline (or a comment holding one) separates it from the token before.
    nl: bool,
}

fn lex(src: &[u8]) -> Option<Vec<Tok>> {
    let mut v = Vec::new();
    let mut i = 0;
    let mut line = 1u32;
    let mut ls = 0usize;
    let mut nl = false;
    while i < src.len() {
        let c = src[i];
        if c == b'\n' {
            line += 1;
            ls = i + 1;
            nl = true;
            i += 1;
            continue;
        }
        if c == b' ' || c == b'\t' || c == b'\r' || c == 0x0c {
            i += 1;
            continue;
        }
        let start = i;
        let (tl, tc) = (line, u32::try_from(i - ls + 1).ok()?);
        let mut nl_after = false;
        let kind = if c == b'/' && src.get(i + 1) == Some(&b'/') {
            while i < src.len() && src[i] != b'\n' {
                i += 1;
            }
            Tk::Comment
        } else if c == b'/' && src.get(i + 1) == Some(&b'*') {
            i += 2;
            loop {
                if i + 1 >= src.len() {
                    return None;
                }
                if src[i] == b'*' && src[i + 1] == b'/' {
                    i += 2;
                    break;
                }
                if src[i] == b'\n' {
                    line += 1;
                    ls = i + 1;
                    nl_after = true;
                }
                i += 1;
            }
            Tk::Comment
        } else if c == b'"' || c == b'\'' {
            i += 1;
            while i < src.len() && src[i] != c {
                if src[i] == b'\n' {
                    return None;
                }
                if src[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            if i >= src.len() {
                return None;
            }
            i += 1;
            Tk::Lit
        } else if c == b'`' {
            i += 1;
            while i < src.len() && src[i] != b'`' {
                if src[i] == b'\n' {
                    line += 1;
                    ls = i + 1;
                }
                i += 1;
            }
            if i >= src.len() {
                return None;
            }
            i += 1;
            Tk::Lit
        } else if c.is_ascii_digit()
            || (c == b'.' && src.get(i + 1).is_some_and(u8::is_ascii_digit))
        {
            i += 1;
            while i < src.len()
                && (src[i].is_ascii_alphanumeric()
                    || src[i] == b'_'
                    || src[i] == b'.'
                    || (matches!(src[i], b'+' | b'-')
                        && matches!(src[i - 1], b'e' | b'E' | b'p' | b'P')))
            {
                i += 1;
            }
            Tk::Lit
        } else if c.is_ascii_alphabetic() || c == b'_' || c >= 0x80 {
            while i < src.len()
                && (src[i].is_ascii_alphanumeric() || src[i] == b'_' || src[i] >= 0x80)
            {
                i += 1;
            }
            Tk::Ident
        } else {
            i += 1;
            Tk::Op
        };
        v.push(Tok {
            kind,
            start,
            end: i,
            line: tl,
            col: tc,
            nl,
        });
        nl = nl_after;
    }
    Some(v)
}

fn text<'s>(src: &'s [u8], t: &Tok) -> &'s [u8] {
    &src[t.start..t.end]
}

fn is(src: &[u8], t: &Tok, s: &str) -> bool {
    t.kind != Tk::Comment && text(src, t) == s.as_bytes()
}

/// A comment the compiler or go list reads.
fn directive(src: &[u8], t: &Tok) -> bool {
    let s = text(src, t);
    t.kind == Tk::Comment
        && (s.starts_with(b"//go:")
            || s.starts_with(b"//line ")
            || s.starts_with(b"/*line ")
            || s.starts_with(b"//export ")
            || s.starts_with(b"// +build"))
}

fn hash(parts: &[&[u8]]) -> String {
    let mut h = blake3::Hasher::new();
    for p in parts {
        h.update(&(p.len() as u64).to_le_bytes());
        h.update(p);
    }
    h.finalize().to_hex()[..32].to_string()
}

/// What `go list` reads of a .go file, in two parts.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) struct Header {
    /// Everything up to the package clause's end (build constraints and
    /// other directives), every `//go:embed` line, and in a cgo file the
    /// whole header (its preamble).
    pub fixed: String,
    /// The import paths.
    pub imports: Vec<String>,
}

/// A .go file's `Header`. None: not a file this lexer can read that far.
pub(super) fn header(src: &[u8]) -> Option<Header> {
    let toks = lex(src)?;
    let code: Vec<&Tok> = toks.iter().filter(|t| t.kind != Tk::Comment).collect();
    if code.len() < 2 || !is(src, code[0], "package") || code[1].kind != Tk::Ident {
        return None;
    }
    let pkg_end = code[1].end;
    let mut end = pkg_end;
    let mut imports: Vec<String> = Vec::new();
    let mut path = |t: &Tok| -> Option<()> {
        if t.kind == Tk::Lit {
            let s = std::str::from_utf8(text(src, t)).ok()?;
            let inner = s.get(1..s.len().checked_sub(1)?)?;
            if inner.contains('\\') || !(s.starts_with('"') || s.starts_with('`')) {
                return None;
            }
            imports.push(inner.to_string());
        }
        Some(())
    };
    let mut k = 2;
    while k < code.len() && is(src, code[k], "import") {
        k += 1;
        if k < code.len() && is(src, code[k], "(") {
            let mut depth = 0;
            loop {
                let t = code.get(k)?;
                path(t)?;
                if is(src, t, "(") {
                    depth += 1;
                } else if is(src, t, ")") {
                    depth -= 1;
                    if depth == 0 {
                        end = t.end;
                        k += 1;
                        break;
                    }
                }
                k += 1;
            }
        } else {
            // One spec: up to the end of its line.
            let first = k;
            while k < code.len() && (k == first || !code[k].nl) && !is(src, code[k], ";") {
                path(code[k])?;
                end = code[k].end;
                k += 1;
            }
            if k < code.len() && is(src, code[k], ";") {
                end = code[k].end;
                k += 1;
            }
        }
        while k < code.len() && is(src, code[k], ";") {
            end = code[k].end;
            k += 1;
        }
    }
    let cgo = imports.iter().any(|i| i == "C");
    let mut parts: Vec<&[u8]> = vec![if cgo { &src[..end] } else { &src[..pkg_end] }];
    for t in &toks {
        if t.kind == Tk::Comment && text(src, t).starts_with(b"//go:embed") {
            parts.push(text(src, t));
        }
    }
    imports.sort();
    imports.dedup();
    Some(Header {
        fixed: hash(&parts),
        imports,
    })
}

/// A top-level function or method.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Func {
    /// Its signature's tokens (an ordinal appended to a repeat, `init`).
    pub id: String,
    /// The hash of its body's exact bytes.
    pub body: String,
    /// The line its `func` is on.
    pub line: u32,
    /// Its body's `{` and `}`: (line, column), columns in bytes from 1.
    pub open: (u32, u32),
    pub close: (u32, u32),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Shape {
    /// Everything outside function bodies, comments but directives dropped,
    /// newlines kept as separators (they end statements).
    pub skeleton: String,
    pub funcs: Vec<Func>,
}

/// A file's skeleton and functions. None: not a file this lexer can split.
pub(super) fn shape(src: &[u8]) -> Option<Shape> {
    let toks = lex(src)?;
    let mut skel: Vec<&[u8]> = Vec::new();
    let mut funcs: Vec<Func> = Vec::new();
    let mut depth = 0i32;
    let mut i = 0;
    while i < toks.len() {
        let t = &toks[i];
        if t.kind == Tk::Comment {
            if directive(src, t) {
                push(&mut skel, t);
                skel.push(text(src, t));
            }
            i += 1;
            continue;
        }
        if depth == 0 && is(src, t, "func") {
            // The signature, up to the body's `{` (struct{} and interface{}
            // in it are types) or the end of a body-less declaration.
            let mut sig: Vec<&[u8]> = Vec::new();
            let mut j = i;
            let mut sd = 0i32;
            let mut prev: Option<&Tok> = None;
            let mut body: Option<usize> = None;
            while j < toks.len() {
                let u = &toks[j];
                if u.kind == Tk::Comment {
                    if directive(src, u) {
                        return None;
                    }
                    j += 1;
                    continue;
                }
                if j > i
                    && sd == 0
                    && ((u.nl && prev.is_some_and(|p| ends_statement(src, p))) || is(src, u, ";"))
                {
                    break;
                }
                if is(src, u, "(") || is(src, u, "[") {
                    sd += 1;
                } else if is(src, u, ")") || is(src, u, "]") {
                    sd -= 1;
                } else if is(src, u, "{") {
                    if prev.is_some_and(|p| is(src, p, "struct") || is(src, p, "interface")) {
                        sd += 1;
                    } else if sd == 0 {
                        body = Some(j);
                        break;
                    } else {
                        sd += 1;
                    }
                } else if is(src, u, "}") {
                    sd -= 1;
                }
                if sd < 0 {
                    return None;
                }
                push(&mut skel, u);
                skel.push(text(src, u));
                sig.push(text(src, u));
                prev = Some(u);
                j += 1;
            }
            let Some(open) = body else {
                i = j;
                continue;
            };
            let mut bd = 0i32;
            let mut k = open;
            let close = loop {
                let u = toks.get(k)?;
                if u.kind != Tk::Comment {
                    if is(src, u, "{") {
                        bd += 1;
                    } else if is(src, u, "}") {
                        bd -= 1;
                        if bd == 0 {
                            break k;
                        }
                    }
                }
                k += 1;
            };
            let (o, c) = (&toks[open], &toks[close]);
            skel.push(b"{body}");
            let mut id = String::from_utf8_lossy(&sig.join(&b' ')).into_owned();
            let n = funcs
                .iter()
                .filter(|f| f.id == id || f.id.starts_with(&format!("{id}#")))
                .count();
            if n > 0 {
                id = format!("{id}#{n}");
            }
            funcs.push(Func {
                id,
                body: hash(&[&src[o.start..c.end]]),
                line: t.line,
                open: (o.line, o.col),
                close: (c.line, c.col),
            });
            i = close + 1;
            continue;
        }
        if is(src, t, "(") || is(src, t, "[") || is(src, t, "{") {
            depth += 1;
        } else if is(src, t, ")") || is(src, t, "]") || is(src, t, "}") {
            depth -= 1;
            if depth < 0 {
                return None;
            }
        }
        push(&mut skel, t);
        skel.push(text(src, t));
        i += 1;
    }
    if depth != 0 {
        return None;
    }
    Some(Shape {
        skeleton: hash(&skel),
        funcs,
    })
}

fn push(skel: &mut Vec<&[u8]>, t: &Tok) {
    if t.nl {
        skel.push(b"\n");
    }
}

/// Whether Go would end a statement after this token at a newline.
fn ends_statement(src: &[u8], t: &Tok) -> bool {
    match t.kind {
        Tk::Ident => !matches!(
            text(src, t),
            b"func" | b"chan" | b"map" | b"struct" | b"interface" | b"var" | b"const" | b"type"
        ),
        Tk::Lit => true,
        Tk::Op => matches!(text(src, t), b")" | b"]" | b"}"),
        Tk::Comment => false,
    }
}

/// The function whose body holds (line, col), if any.
pub(super) fn owner(funcs: &[Func], line: u32, col: u32) -> Option<usize> {
    funcs
        .iter()
        .position(|f| f.open <= (line, col) && (line, col) <= f.close)
}

/// A test name with what changes run to run (hex addresses, RFC 3339
/// timestamps, `time.Date(...)` literals) replaced, for matching results
/// only.
pub(super) fn norm_name(n: &str) -> String {
    let b = n.as_bytes();
    let mut out = String::with_capacity(n.len());
    let mut i = 0;
    while i < b.len() {
        if b[i..].starts_with(b"0x") && b.get(i + 2).is_some_and(u8::is_ascii_hexdigit) {
            i += 2;
            while i < b.len() && b[i].is_ascii_hexdigit() {
                i += 1;
            }
            out.push_str("0x?");
            continue;
        }
        if b[i..].starts_with(b"time.Date(")
            && let Some(e) = n[i..].find(')')
        {
            out.push_str("time.Date(?)");
            i += e + 1;
            continue;
        }
        if let Some(len) = rfc3339(&b[i..]) {
            out.push_str("<time>");
            i += len;
            continue;
        }
        let ch = n[i..].chars().next().unwrap_or('?');
        out.push(ch);
        i += ch.len_utf8().max(1);
    }
    out
}

/// The length of an RFC 3339 timestamp at the start of `b`.
fn rfc3339(b: &[u8]) -> Option<usize> {
    let shape = b"dddd-dd-ddTdd:dd:dd";
    if b.len() < shape.len() {
        return None;
    }
    for (x, s) in b.iter().zip(shape) {
        let ok = if *s == b'd' {
            x.is_ascii_digit()
        } else {
            x == s || (*s == b'T' && (*x == b' ' || *x == b'_'))
        };
        if !ok {
            return None;
        }
    }
    let mut i = shape.len();
    if b.get(i) == Some(&b'.') {
        i += 1;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
    }
    match b.get(i) {
        Some(b'Z') => i += 1,
        Some(b'+' | b'-')
            if b.len() >= i + 6
                && b[i + 1].is_ascii_digit()
                && b[i + 2].is_ascii_digit()
                && b[i + 3] == b':' =>
        {
            i += 6;
        }
        _ => {}
    }
    Some(i)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "//go:build linux\n\n// Package p.\npackage p\n\nimport (\n\t\"fmt\"\n\t\"os\"\n)\n\nimport \"strings\"\n\nvar X = 1\n\n// F does f.\nfunc F(s struct{ a int }) interface{ M() } {\n\tif true {\n\t\tfmt.Println(\"}\")\n\t}\n\treturn nil\n}\n\nfunc (r *T) G() {\n\t_ = `{`\n}\n\nfunc init() {}\nfunc init() { os.Exit(0) }\n";

    #[test]
    fn headers_ignore_bodies_but_not_imports() {
        let h = header(A.as_bytes()).unwrap();
        assert_eq!(h.imports, vec!["fmt", "os", "strings"]);
        let body = A.replace("return nil", "return nil // changed");
        assert_eq!(header(body.as_bytes()).unwrap(), h);
        let imp = header(A.replace("import \"strings\"", "import \"bytes\"").as_bytes()).unwrap();
        assert_eq!(imp.fixed, h.fixed);
        assert_eq!(imp.imports, vec!["bytes", "fmt", "os"]);
        let tag = A.replace("linux", "darwin");
        assert_ne!(header(tag.as_bytes()).unwrap().fixed, h.fixed);
        let emb = A.replace("var X = 1", "//go:embed x.txt\nvar X string");
        assert_ne!(header(emb.as_bytes()).unwrap().fixed, h.fixed);
        assert_eq!(header(b"x := 1"), None);
    }

    #[test]
    fn shapes_split_bodies_from_the_rest() {
        let s = shape(A.as_bytes()).unwrap();
        let ids: Vec<&str> = s.funcs.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "func F ( s struct { a int } ) interface { M ( ) }",
                "func ( r * T ) G ( )",
                "func init ( )",
                "func init ( )#1",
            ]
        );
        assert_eq!(s.funcs[0].line, 16);
        assert_eq!(s.funcs[0].open, (16, 44));
        assert_eq!(s.funcs[0].close, (21, 1));
        // A body edit keeps the skeleton and the other bodies.
        let b = shape(A.replace("return nil", "return New()").as_bytes()).unwrap();
        assert_eq!(b.skeleton, s.skeleton);
        assert_ne!(b.funcs[0].body, s.funcs[0].body);
        assert_eq!(b.funcs[1], s.funcs[1]);
        // A doc comment does not change the skeleton; a declaration does.
        let c = shape(A.replace("// F does f.", "// F does f, really.").as_bytes()).unwrap();
        assert_eq!(c.skeleton, s.skeleton);
        let d = shape(A.replace("var X = 1", "var X = 2").as_bytes()).unwrap();
        assert_ne!(d.skeleton, s.skeleton);
        let e = shape(A.replace("func (r *T) G()", "func (r T) G()").as_bytes()).unwrap();
        assert_ne!(e.skeleton, s.skeleton);
        assert_eq!(owner(&s.funcs, 18, 3), Some(0));
        assert_eq!(owner(&s.funcs, 13, 1), None);
        assert_eq!(shape(b"package p\nfunc f() {"), None);
    }

    #[test]
    fn names_normalize_what_varies() {
        assert_eq!(norm_name("T/S{p:0x1e9505145ca0}"), "T/S{p:0x?}");
        assert_eq!(
            norm_name("T/time.Date(2026,_time.November,_3,_4,_42,_31,_303363787,_time.Local)_x"),
            "T/time.Date(?)_x"
        );
        assert_eq!(norm_name("T/at_2026-11-03T04:42:31.303Z"), "T/at_<time>");
        assert_eq!(norm_name("TestA/sub_1"), "TestA/sub_1");
    }
}
