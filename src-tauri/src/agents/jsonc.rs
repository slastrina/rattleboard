//! Small, targeted edits to JSON and JSON-with-comments files (VS Code's mcp.json,
//! opencode.jsonc, Gemini's settings.json): add or replace one object member, append one array
//! item, or take either out again, leaving every other byte of the file as it was: comments,
//! key order, indentation, trailing commas and line endings.
//!
//! Adding something and then removing it gives back the original text exactly.

use serde_json::Value;

/// A value to write, with its keys in the order they should appear in the file.
#[derive(Clone, Debug, PartialEq)]
pub enum J {
    S(String),
    B(bool),
    N(i64),
    A(Vec<J>),
    O(Vec<(String, J)>),
}

impl J {
    pub fn s(v: impl Into<String>) -> J {
        J::S(v.into())
    }
    pub fn o(pairs: Vec<(&str, J)>) -> J {
        J::O(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }
    pub fn strs(v: &[&str]) -> J {
        J::A(v.iter().map(|s| J::s(*s)).collect())
    }
    pub fn to_value(&self) -> Value {
        match self {
            J::S(s) => Value::String(s.clone()),
            J::B(b) => Value::Bool(*b),
            J::N(n) => Value::from(*n),
            J::A(a) => Value::Array(a.iter().map(J::to_value).collect()),
            J::O(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), v.to_value())).collect()),
        }
    }
    /// Pretty at indent `ind`, or all on one line (None) to match a one-line container.
    fn layout(&self, unit: &str, ind: Option<&str>, nl: &str) -> String {
        match ind {
            Some(i) => self.render(unit, i, nl),
            None => self.compact(),
        }
    }
    fn compact(&self) -> String {
        match self {
            J::A(a) => format!("[{}]", a.iter().map(J::compact).collect::<Vec<_>>().join(", ")),
            J::O(o) => format!("{{{}}}", o.iter().map(|(k, v)| format!("{}: {}", Value::String(k.clone()), v.compact())).collect::<Vec<_>>().join(", ")),
            v => v.render("", "", ""),
        }
    }
    fn scalar(&self) -> bool {
        matches!(self, J::S(_) | J::B(_) | J::N(_))
    }
    /// Pretty-printed, nested lines indented by `base` plus one `unit` per level. Arrays of
    /// plain values stay on one line, as people write `"args": ["--mcp"]`.
    fn render(&self, unit: &str, base: &str, nl: &str) -> String {
        match self {
            J::S(s) => Value::String(s.clone()).to_string(),
            J::B(b) => b.to_string(),
            J::N(n) => n.to_string(),
            J::A(a) if a.is_empty() => "[]".into(),
            J::A(a) if a.iter().all(J::scalar) => format!("[{}]", a.iter().map(|v| v.render(unit, base, nl)).collect::<Vec<_>>().join(", ")),
            J::A(a) => {
                let inner = format!("{base}{unit}");
                let items: Vec<String> = a.iter().map(|v| format!("{inner}{}", v.render(unit, &inner, nl))).collect();
                format!("[{nl}{}{nl}{base}]", items.join(&format!(",{nl}")))
            }
            J::O(o) if o.is_empty() => "{}".into(),
            J::O(o) => {
                let inner = format!("{base}{unit}");
                let items: Vec<String> =
                    o.iter().map(|(k, v)| format!("{inner}{}: {}", Value::String(k.clone()), v.render(unit, &inner, nl))).collect();
                format!("{{{nl}{}{nl}{base}}}", items.join(&format!(",{nl}")))
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum Node {
    Obj { open: usize, close: usize, members: Vec<(String, usize, Node)> },
    Arr { open: usize, close: usize, items: Vec<Node> },
    Leaf { start: usize, end: usize },
}

impl Node {
    fn start(&self) -> usize {
        match self {
            Node::Obj { open, .. } | Node::Arr { open, .. } => *open,
            Node::Leaf { start, .. } => *start,
        }
    }
    /// Just past the value
    fn end(&self) -> usize {
        match self {
            Node::Obj { close, .. } | Node::Arr { close, .. } => close + 1,
            Node::Leaf { end, .. } => *end,
        }
    }
    fn member(&self, key: &str) -> Option<(usize, &Node)> {
        match self {
            Node::Obj { members, .. } => members.iter().position(|m| m.0 == key).map(|i| (i, &members[i].2)),
            _ => None,
        }
    }
    /// Each member's or item's span, from its key (or value) to the end of its value
    fn entries(&self) -> Vec<(usize, usize)> {
        match self {
            Node::Obj { members, .. } => members.iter().map(|(_, k, v)| (*k, v.end())).collect(),
            Node::Arr { items, .. } => items.iter().map(|v| (v.start(), v.end())).collect(),
            Node::Leaf { .. } => vec![],
        }
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn fail<T>(&self, what: &str) -> Result<T, String> {
        let line = self.s[..self.i.min(self.s.len())].iter().filter(|&&c| c == b'\n').count() + 1;
        Err(format!("{what} on line {line}"))
    }

    fn ws(&mut self) -> Result<(), String> {
        loop {
            while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
                self.i += 1;
            }
            if self.s[self.i..].starts_with(b"//") {
                while self.i < self.s.len() && self.s[self.i] != b'\n' {
                    self.i += 1;
                }
            } else if self.s[self.i..].starts_with(b"/*") {
                match self.s[self.i + 2..].windows(2).position(|w| w == b"*/") {
                    Some(p) => self.i += p + 4,
                    None => return self.fail("Unclosed comment"),
                }
            } else {
                return Ok(());
            }
        }
    }

    fn string(&mut self) -> Result<(usize, usize), String> {
        let start = self.i;
        self.i += 1;
        while self.i < self.s.len() {
            match self.s[self.i] {
                b'\\' => self.i += 2,
                b'"' => {
                    self.i += 1;
                    return Ok((start, self.i));
                }
                _ => self.i += 1,
            }
        }
        self.fail("Unclosed string")
    }

    fn value(&mut self) -> Result<Node, String> {
        self.ws()?;
        match self.s.get(self.i) {
            Some(b'{') => {
                let open = self.i;
                self.i += 1;
                let mut members = vec![];
                loop {
                    self.ws()?;
                    match self.s.get(self.i) {
                        Some(b'}') => break,
                        Some(b'"') => {}
                        _ => return self.fail("Expected a key or }"),
                    }
                    let (ks, ke) = self.string()?;
                    let key: String = serde_json::from_slice(&self.s[ks..ke]).map_err(|e| e.to_string())?;
                    self.ws()?;
                    if self.s.get(self.i) != Some(&b':') {
                        return self.fail("Expected :");
                    }
                    self.i += 1;
                    let v = self.value()?;
                    members.push((key, ks, v));
                    self.ws()?;
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => break,
                        _ => return self.fail("Expected , or }"),
                    }
                }
                let close = self.i;
                self.i += 1;
                Ok(Node::Obj { open, close, members })
            }
            Some(b'[') => {
                let open = self.i;
                self.i += 1;
                let mut items = vec![];
                loop {
                    self.ws()?;
                    if self.s.get(self.i) == Some(&b']') {
                        break;
                    }
                    items.push(self.value()?);
                    self.ws()?;
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => break,
                        _ => return self.fail("Expected , or ]"),
                    }
                }
                let close = self.i;
                self.i += 1;
                Ok(Node::Arr { open, close, items })
            }
            Some(b'"') => {
                let (start, end) = self.string()?;
                Ok(Node::Leaf { start, end })
            }
            Some(_) => {
                let start = self.i;
                while self.i < self.s.len() && (self.s[self.i].is_ascii_alphanumeric() || matches!(self.s[self.i], b'-' | b'+' | b'.')) {
                    self.i += 1;
                }
                if start == self.i {
                    return self.fail("Unexpected character");
                }
                serde_json::from_slice::<Value>(&self.s[start..self.i]).map_err(|_| format!("Not a JSON value: {}", String::from_utf8_lossy(&self.s[start..self.i])))?;
                Ok(Node::Leaf { start, end: self.i })
            }
            None => self.fail("Unexpected end of file"),
        }
    }
}

/// The file's top-level value. Comments and trailing commas are allowed.
pub fn parse(text: &str) -> Result<Node, String> {
    let mut p = Parser { s: text.as_bytes(), i: 0 };
    let root = p.value()?;
    p.ws()?;
    if p.i != text.len() {
        return p.fail("Unexpected text after the end");
    }
    Ok(root)
}

/// A parsed node's value, with comments dropped.
pub fn value_of(text: &str, n: &Node) -> Value {
    match n {
        Node::Leaf { start, end } => serde_json::from_str(&text[*start..*end]).unwrap_or(Value::Null),
        Node::Arr { items, .. } => Value::Array(items.iter().map(|i| value_of(text, i)).collect()),
        Node::Obj { members, .. } => Value::Object(members.iter().map(|(k, _, v)| (k.clone(), value_of(text, v))).collect()),
    }
}

fn find<'a>(root: &'a Node, path: &[String]) -> Option<&'a Node> {
    path.iter().try_fold(root, |n, k| n.member(k).map(|m| m.1))
}

fn newline(text: &str) -> &'static str {
    if text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

/// The whitespace a line starts with.
fn line_indent(text: &str, pos: usize) -> &str {
    let start = text[..pos].rfind('\n').map(|p| p + 1).unwrap_or(0);
    let rest = &text[start..];
    &rest[..rest.len() - rest.trim_start_matches([' ', '\t']).len()]
}

/// True when only spaces or tabs come between the start of `pos`'s line and `pos`.
fn starts_line(text: &str, pos: usize) -> bool {
    let start = text[..pos].rfind('\n').map(|p| p + 1).unwrap_or(0);
    start > 0 && text[start..pos].bytes().all(|c| c == b' ' || c == b'\t')
}

/// One level of indentation, as the file already uses it.
fn unit(text: &str, root: &Node) -> String {
    if let Some((s, _)) = root.entries().first() {
        if starts_line(text, *s) {
            let ind = line_indent(text, *s);
            if !ind.is_empty() {
                return ind.to_string();
            }
        }
    }
    "  ".into()
}

/// Past spaces, tabs and comments (but not line breaks) from `p`.
fn skip_inline(text: &str, mut p: usize) -> usize {
    let b = text.as_bytes();
    loop {
        while p < b.len() && (b[p] == b' ' || b[p] == b'\t') {
            p += 1;
        }
        if text[p..].starts_with("/*") {
            match text[p + 2..].find("*/") {
                Some(e) if !text[p..p + 2 + e].contains('\n') => p += e + 4,
                _ => return p,
            }
        } else {
            return p;
        }
    }
}

/// Where the line holding `p` ends (before its line break), when only spaces and comments
/// follow `p` on it.
fn end_of_line(text: &str, p: usize) -> Option<usize> {
    let q = skip_inline(text, p);
    let rest = &text[q..];
    if rest.starts_with("//") {
        let e = rest.find('\n').map(|e| q + e).unwrap_or(text.len());
        return Some(if e > 0 && text.as_bytes()[e - 1] == b'\r' { e - 1 } else { e });
    }
    if rest.is_empty() || rest.starts_with('\n') {
        return Some(q);
    }
    if rest.starts_with("\r\n") {
        return Some(q);
    }
    None
}

/// The position of the comma after `p`, past whitespace and comments.
fn comma_after(text: &str, p: usize) -> Option<usize> {
    let mut parser = Parser { s: text.as_bytes(), i: p };
    parser.ws().ok()?;
    (text.as_bytes().get(parser.i) == Some(&b',')).then_some(parser.i)
}

/// Adds `entry` (a member `"key": value` or an array item) at the end of container `c`.
fn insert(text: &str, c: &Node, entry: impl Fn(Option<&str>) -> String, unit: &str) -> String {
    let nl = newline(text);
    let (open, close) = match c {
        Node::Obj { open, close, .. } | Node::Arr { open, close, .. } => (*open, *close),
        Node::Leaf { .. } => unreachable!(),
    };
    let entries = c.entries();
    let mut out = text.to_string();
    let Some(&(first, _)) = entries.first() else {
        let base = line_indent(text, open).to_string();
        let ind = format!("{base}{unit}");
        let inner = &text[open + 1..close];
        let piece = format!("{nl}{ind}{}{nl}{base}", entry(Some(&ind)));
        if inner.trim().is_empty() {
            out.replace_range(open + 1..close, &piece);
        } else {
            out.insert_str(close, &piece);
        }
        return out;
    };
    let (_, last_end) = *entries.last().unwrap();
    let trailing = comma_after(text, last_end);
    if !starts_line(text, first) {
        // All on one line: {"a": 1} -> {"a": 1, "b": 2}
        match trailing {
            Some(cm) => out.insert_str(cm + 1, &format!(" {},", entry(None))),
            None => out.insert_str(last_end, &format!(", {}", entry(None))),
        }
        return out;
    }
    let ind = line_indent(text, first).to_string();
    match trailing {
        Some(cm) => {
            let at = end_of_line(text, cm + 1).unwrap_or(cm + 1);
            out.insert_str(at, &format!("{nl}{ind}{},", entry(Some(&ind))));
        }
        None => {
            let at = end_of_line(text, last_end).unwrap_or(last_end);
            out.insert_str(at, &format!("{nl}{ind}{}", entry(Some(&ind))));
            out.insert(last_end, ',');
        }
    }
    out
}

/// Takes entry `idx` out of container `c`, the reverse of `insert`.
fn remove(text: &str, c: &Node, idx: usize) -> String {
    let entries = c.entries();
    let (s, e) = entries[idx];
    let (open, close) = match c {
        Node::Obj { open, close, .. } | Node::Arr { open, close, .. } => (*open, *close),
        Node::Leaf { .. } => unreachable!(),
    };
    let mut out = text.to_string();
    let own_comma = comma_after(text, e);
    if entries.len() == 1 {
        let end = own_comma.map(|c| c + 1).unwrap_or(e);
        let rest_blank = text[open + 1..s].trim().is_empty() && text[end..close].trim().is_empty();
        if rest_blank {
            out.replace_range(open + 1..close, "");
        } else {
            out.replace_range(s..end, "");
        }
        return out;
    }
    // From the line break before the entry, when it starts a line
    let line_start = |p: usize| {
        let mut q = p;
        while q > 0 && matches!(text.as_bytes()[q - 1], b' ' | b'\t') {
            q -= 1;
        }
        if q > 0 && text.as_bytes()[q - 1] == b'\n' {
            q -= 1;
            if q > 0 && text.as_bytes()[q - 1] == b'\r' {
                q -= 1;
            }
        }
        q
    };
    if idx + 1 < entries.len() {
        let (ns, _) = entries[idx + 1];
        if starts_line(text, s) && starts_line(text, ns) {
            out.replace_range(line_start(s)..line_start(ns), "");
        } else {
            out.replace_range(s..ns, "");
        }
        return out;
    }
    // The last entry: the one before it loses its comma unless the file uses trailing commas.
    let (_, pe) = entries[idx - 1];
    if starts_line(text, s) {
        match own_comma {
            Some(cm) => out.replace_range(line_start(s)..cm + 1, ""),
            None => {
                out.replace_range(line_start(s)..e, "");
                if let Some(pc) = comma_after(text, pe) {
                    out.remove(pc);
                }
            }
        }
    } else {
        match own_comma {
            Some(cm) => {
                let pc = comma_after(text, pe).unwrap_or(pe);
                out.replace_range(pc + 1..cm + 1, "");
            }
            None => out.replace_range(pe..e, ""),
        }
    }
    out
}

/// Text to start from: an empty or missing file is an empty object.
fn base(text: &str) -> String {
    if text.trim().is_empty() {
        "{}\n".into()
    } else {
        text.to_string()
    }
}

/// Makes sure each container on `path` exists, creating it (objects, and the last one an array
/// if `array`) where missing. Returns the text and the paths it created.
fn ensure(mut text: String, path: &[String], array: bool) -> Result<(String, Vec<Vec<String>>), String> {
    let mut created = vec![];
    'again: loop {
        let root = parse(&text)?;
        let u = unit(&text, &root);
        let mut n = &root;
        if !matches!(n, Node::Obj { .. }) {
            return Err("The file doesn't hold a JSON object".into());
        }
        for (i, k) in path.iter().enumerate() {
            let last = i + 1 == path.len();
            match n.member(k) {
                Some((_, child)) => {
                    let ok = if last && array { matches!(child, Node::Arr { .. }) } else { matches!(child, Node::Obj { .. }) };
                    if !ok {
                        return Err(format!("\"{}\" isn't {} in this file", path[..=i].join("."), if last && array { "a list" } else { "an object" }));
                    }
                    n = child;
                }
                None => {
                    let empty = if last && array { "[]" } else { "{}" };
                    let key = Value::String(k.clone()).to_string();
                    text = insert(&text, n, |_| format!("{key}: {empty}"), &u);
                    created.push(path[..=i].to_vec());
                    continue 'again;
                }
            }
        }
        return Ok((text, created));
    }
}

pub fn path(p: &[&str]) -> Vec<String> {
    p.iter().map(|s| s.to_string()).collect()
}

/// Sets member `key` of the object at `at` to `value`, creating the objects on the way. A
/// member that already holds the same value is left alone.
pub fn set_member(text: &str, at: &[String], key: &str, value: &J) -> Result<(String, Vec<Vec<String>>), String> {
    let (text, created) = ensure(base(text), at, false)?;
    let root = parse(&text)?;
    let u = unit(&text, &root);
    let obj = find(&root, at).ok_or("missing object")?;
    let nl = newline(&text);
    match obj.member(key) {
        Some((_, v)) if value_of(&text, v) == value.to_value() => Ok((text, created)),
        Some((i, v)) => {
            let Node::Obj { members, .. } = obj else { unreachable!() };
            let ind = line_indent(&text, members[i].1).to_string();
            let mut out = text.clone();
            out.replace_range(v.start()..v.end(), &value.render(&u, &ind, nl));
            Ok((out, created))
        }
        None => {
            let k = Value::String(key.to_string()).to_string();
            Ok((insert(&text, obj, |ind| format!("{k}: {}", value.layout(&u, ind, nl)), &u), created))
        }
    }
}

/// Removes member `key` of the object at `at`, then any of `created` left empty.
pub fn remove_member(text: &str, at: &[String], key: &str, created: &[Vec<String>]) -> Result<String, String> {
    if text.trim().is_empty() {
        return Ok(text.to_string());
    }
    let root = parse(text)?;
    let mut out = text.to_string();
    if let Some((i, _)) = find(&root, at).and_then(|o| o.member(key)) {
        out = remove(text, find(&root, at).unwrap(), i);
    }
    prune(out, created)
}

/// Appends `value` to the array at `at` (creating it), unless an item `ours` already says
/// is ours: that one is replaced if it differs, so connecting twice adds nothing.
pub fn add_item(text: &str, at: &[String], value: &J, ours: &dyn Fn(&Value) -> bool) -> Result<(String, Vec<Vec<String>>), String> {
    let (text, created) = ensure(base(text), at, true)?;
    let root = parse(&text)?;
    let u = unit(&text, &root);
    let nl = newline(&text);
    let arr = find(&root, at).ok_or("missing list")?;
    let Node::Arr { items, .. } = arr else { unreachable!() };
    if let Some(it) = items.iter().find(|it| ours(&value_of(&text, it))) {
        if value_of(&text, it) == value.to_value() {
            return Ok((text, created));
        }
        let ind = line_indent(&text, it.start()).to_string();
        let mut out = text.clone();
        out.replace_range(it.start()..it.end(), &value.render(&u, &ind, nl));
        return Ok((out, created));
    }
    Ok((insert(&text, arr, |ind| value.layout(&u, ind, nl), &u), created))
}

/// Removes every item of the array at `at` that `ours` recognises, then any of `created` left
/// empty.
pub fn remove_items(text: &str, at: &[String], ours: &dyn Fn(&Value) -> bool, created: &[Vec<String>]) -> Result<String, String> {
    if text.trim().is_empty() {
        return Ok(text.to_string());
    }
    let mut out = text.to_string();
    loop {
        let root = parse(&out)?;
        let Some(arr @ Node::Arr { items, .. }) = find(&root, at) else { break };
        let Some(i) = items.iter().position(|it| ours(&value_of(&out, it))) else { break };
        out = remove(&out, arr, i);
    }
    prune(out, created)
}

/// Removes the containers this app created (deepest first) once nothing is left in them.
fn prune(mut text: String, created: &[Vec<String>]) -> Result<String, String> {
    let mut paths = created.to_vec();
    paths.sort_by_key(|p| std::cmp::Reverse(p.len()));
    for p in paths {
        let Some((key, parent)) = p.split_last() else { continue };
        let root = parse(&text)?;
        let Some(par) = find(&root, parent) else { continue };
        if let Some((i, child)) = par.member(key) {
            if child.entries().is_empty() && !matches!(child, Node::Leaf { .. }) {
                text = remove(&text, par, i);
            }
        }
    }
    Ok(text)
}

/// Whether the text is an object with nothing in it (so a file this app created can go).
pub fn is_empty_object(text: &str) -> bool {
    matches!(parse(text), Ok(Node::Obj { ref members, .. }) if members.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn server() -> J {
        J::o(vec![("type", J::s("stdio")), ("command", J::s("/Apps/rb")), ("args", J::strs(&["--mcp"]))])
    }

    fn round_trip(before: &str, at: &[&str]) -> String {
        let at = path(at);
        let (after, created) = set_member(before, &at, "rattleboard", &server()).unwrap();
        let (again, created2) = set_member(&after, &at, "rattleboard", &server()).unwrap();
        assert_eq!(after, again, "adding twice changes nothing");
        assert!(created2.is_empty());
        parse(&after).unwrap();
        let back = remove_member(&after, &at, "rattleboard", &created).unwrap();
        assert_eq!(back, before, "removing restores the original");
        after
    }

    #[test]
    fn members() {
        let after = round_trip("{\n  \"theme\": \"dark\",\n  \"mcpServers\": {\n    \"other\": {\"command\": \"x\"}\n  }\n}\n", &["mcpServers"]);
        assert_eq!(
            after,
            "{\n  \"theme\": \"dark\",\n  \"mcpServers\": {\n    \"other\": {\"command\": \"x\"},\n    \"rattleboard\": {\n      \"type\": \"stdio\",\n      \"command\": \"/Apps/rb\",\n      \"args\": [\"--mcp\"]\n    }\n  }\n}\n"
        );
        // no mcpServers yet, tabs, CRLF
        let after = round_trip("{\r\n\t\"theme\": \"dark\"\r\n}\r\n", &["mcpServers"]);
        assert!(after.contains("\r\n\t\"mcpServers\": {\r\n\t\t\"rattleboard\": {\r\n\t\t\t\"type\""), "{after:?}");
        // empty object, empty file, single line
        round_trip("{}", &["mcpServers"]);
        round_trip("{\"a\": 1}", &["mcpServers"]);
        round_trip("{\"mcpServers\": {}}\n", &["mcpServers"]);
        assert_eq!(set_member("", &path(&["mcpServers"]), "rattleboard", &server()).unwrap().0.matches("rattleboard").count(), 1);
    }

    #[test]
    fn jsonc() {
        let before = "// VS Code user MCP servers\n{\n  /* mine */\n  \"servers\": {\n    \"git\": { \"command\": \"git-mcp\" }, // keep me\n  },\n  \"inputs\": [],\n}\n";
        let after = round_trip(before, &["servers"]);
        assert!(after.contains("\"git\": { \"command\": \"git-mcp\" }, // keep me\n    \"rattleboard\": {"), "{after}");
        assert!(after.contains("/* mine */") && after.starts_with("// VS Code"));
        let before = "{\n  \"servers\": {\n    \"git\": {} // keep me\n  }\n}";
        let after = round_trip(before, &["servers"]);
        assert!(after.contains("\"git\": {}, // keep me\n    \"rattleboard\""), "{after}");
        // replaced in place when it differs
        let old = "{\n  \"servers\": {\n    \"rattleboard\": {\"command\": \"/old/path\"},\n    \"z\": 1\n  }\n}";
        let (new, _) = set_member(old, &path(&["servers"]), "rattleboard", &server()).unwrap();
        assert_eq!(value_of(&new, &parse(&new).unwrap())["servers"]["rattleboard"]["command"], "/Apps/rb");
        assert!(new.contains("\"z\": 1"));
        // a member in the middle comes out cleanly
        let mid = "{\n  \"a\": 1,\n  \"rattleboard\": 2, // ours\n  \"b\": 3\n}";
        assert_eq!(remove_member(mid, &[], "rattleboard", &[]).unwrap(), "{\n  \"a\": 1,\n  \"b\": 3\n}");
        assert!(parse("{\"a\": /* unclosed").is_err());
        assert!(parse("[1, 2] x").is_err());
    }

    #[test]
    fn items() {
        let hook = J::o(vec![("hooks", J::A(vec![J::o(vec![("type", J::s("command")), ("command", J::s("rb --notify done"))])]))]);
        let ours = |v: &Value| v.to_string().contains("--notify");
        let at = path(&["hooks", "Stop"]);
        for before in [
            "{}\n",
            "{\n  \"model\": \"opus\"\n}\n",
            "{\n  \"hooks\": {\n    \"Stop\": [\n      {\n        \"hooks\": [{\"type\": \"command\", \"command\": \"say hi\"}]\n      }\n    ]\n  }\n}\n",
            "{\"hooks\": {\"Stop\": [{\"hooks\": []}]}}",
        ] {
            let (after, created) = add_item(before, &at, &hook, &ours).unwrap();
            let v = value_of(&after, &parse(&after).unwrap());
            assert!(v["hooks"]["Stop"].as_array().unwrap().iter().any(|h| h == &hook.to_value()), "{after}");
            let (again, _) = add_item(&after, &at, &hook, &ours).unwrap();
            assert_eq!(after, again);
            assert_eq!(remove_items(&after, &at, &ours, &created).unwrap(), before);
        }
        let (a, _) = add_item("{\"x\": [1]}", &path(&["x"]), &J::s("y"), &|_| false).unwrap();
        assert_eq!(value_of(&a, &parse(&a).unwrap()), json!({"x": [1, "y"]}));
        assert!(add_item("{\"hooks\": 3}", &at, &hook, &ours).is_err());
        assert!(is_empty_object(" { } ") && !is_empty_object("{\"a\":1}"));
    }
}
