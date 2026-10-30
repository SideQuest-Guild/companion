//! A bounded, data-only parser for the subset of Lua that WoW's `SavedVariables` serializer emits (never executes
//! anything: no functions, metatables, or expressions beyond literals), plus [`stage_inbox`] for writing the
//! companion's staged `SQI1:` import back into a `SideQuest.lua` file per the inbox contract.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};

use crate::config::atomic_write;
use crate::state::PendingCounts;

/// Refuse to parse anything larger than this; a legitimate SavedVariables file is nowhere near it.
pub const MAX_SIZE_BYTES: usize = 64 * 1024 * 1024;
/// Refuse table nesting deeper than this (protects against malicious or corrupt input causing stack blowups).
pub const MAX_DEPTH: usize = 64;

const INBOX_BEGIN: &str =
    "-- >>> SideQuest Companion inbox (applied and removed by the addon at login)";
const INBOX_END: &str = "-- <<< SideQuest Companion inbox";

/// A parsed Lua value: WoW's serializer only ever emits these.
#[derive(Debug, Clone, PartialEq)]
pub enum LuaValue {
    Nil,
    Bool(bool),
    Number(f64),
    Str(String),
    Table(Vec<(LuaKey, LuaValue)>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LuaKey {
    Index(i64),
    Str(String),
}

impl LuaValue {
    /// Look up a string-keyed field in a table; `None` for any other value or a missing key.
    pub fn get(&self, key: &str) -> Option<&LuaValue> {
        match self {
            LuaValue::Table(entries) => entries.iter().find_map(|(k, v)| match k {
                LuaKey::Str(s) if s == key => Some(v),
                _ => None,
            }),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            LuaValue::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            LuaValue::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// Count of entries with an integer key, i.e. this table's length when used as an array.
    pub fn len_array(&self) -> usize {
        match self {
            LuaValue::Table(entries) => {
                entries.iter().filter(|(k, _)| matches!(k, LuaKey::Index(_))).count()
            }
            _ => 0,
        }
    }

    pub fn is_nil(&self) -> bool {
        matches!(self, LuaValue::Nil)
    }
}

/// Parses `text` as a sequence of top-level Lua assignment statements (`Name = {...}` or `Name[k1][k2] = expr`),
/// applying them in order so a later statement overwrites an earlier whole-variable assignment or a single nested
/// key within it (e.g. `SideQuestDB["inbox"] = {...}` appended after the main `SideQuestDB = {...}` table).
/// Bounded by [`MAX_SIZE_BYTES`] and [`MAX_DEPTH`]; returns `Err` rather than panicking on malformed input.
pub fn parse_document(text: &str) -> Result<HashMap<String, LuaValue>> {
    if text.len() > MAX_SIZE_BYTES {
        bail!("SavedVariables text exceeds the {MAX_SIZE_BYTES}-byte size cap");
    }
    let mut parser = Parser::new(text);
    let mut doc: HashMap<String, LuaValue> = HashMap::new();
    loop {
        parser.skip_trivia();
        if parser.peek().is_none() {
            break;
        }
        parser.parse_statement(&mut doc)?;
    }
    Ok(doc)
}

/// What the engine needs out of a `SideQuestDB` table.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedSavedVariables {
    pub last_export: Option<String>,
    pub pending: Option<PendingCounts>,
    pub standings_generated_at: Option<DateTime<Utc>>,
    pub inbox_staged: bool,
}

/// Best-effort extraction from a SavedVariables file's full text. Always returns something usable: if structured
/// parsing fails, or `SideQuestDB` or `lastExport` is missing, falls back to the last `SQX1:...` token found
/// anywhere in the text (legacy files may contain only that).
pub fn extract(text: &str) -> ParsedSavedVariables {
    let regex_fallback = extract_last_sqx1(text);
    let Ok(doc) = parse_document(text) else {
        return ParsedSavedVariables { last_export: regex_fallback, ..Default::default() };
    };
    let Some(sidequest_db) = doc.get("SideQuestDB") else {
        return ParsedSavedVariables { last_export: regex_fallback, ..Default::default() };
    };
    let last_export = sidequest_db
        .get("lastExport")
        .and_then(LuaValue::as_str)
        .map(str::to_owned)
        .or(regex_fallback);
    let pending = extract_pending(sidequest_db);
    let standings_generated_at = sidequest_db
        .get("standings")
        .and_then(|s| s.get("generated_at"))
        .and_then(LuaValue::as_f64)
        .and_then(|secs| DateTime::<Utc>::from_timestamp(secs as i64, 0));
    let inbox_staged = sidequest_db.get("inbox").is_some_and(|v| !v.is_nil());
    ParsedSavedVariables { last_export, pending, standings_generated_at, inbox_staged }
}

fn extract_pending(sidequest_db: &LuaValue) -> Option<PendingCounts> {
    let pending = sidequest_db.get("pending")?;
    let count = |field: &str| pending.get(field).map(LuaValue::len_array).unwrap_or(0);
    Some(PendingCounts {
        sessions: count("sessions"),
        encounters: count("encounters"),
        loot: count("loot"),
        awards: count("awards"),
        roster: pending.get("roster").is_some_and(|v| !v.is_nil()),
        professions: count("professions"),
        signups: count("signups"),
        bank: count("bank"),
        announcements: count("announcements"),
        recruits: count("recruits"),
        member_notes: count("member_notes"),
    })
}

fn is_base64_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'='
}

/// The last `SQX1:...` token anywhere in `text`, or `None`.
fn extract_last_sqx1(text: &str) -> Option<String> {
    const PREFIX: &str = "SQX1:";
    let bytes = text.as_bytes();
    let mut result = None;
    let mut search_from = 0usize;
    while let Some(rel) = text.get(search_from..).and_then(|rest| rest.find(PREFIX)) {
        let start = search_from + rel;
        let mut end = start + PREFIX.len();
        while end < bytes.len() && is_base64_byte(bytes[end]) {
            end += 1;
        }
        result = Some(text[start..end].to_owned());
        search_from = start + PREFIX.len();
    }
    result
}

// --- Recursive-descent parser -----------------------------------------------------------------------------------

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Self {
        Self { bytes: text.as_bytes(), pos: 0 }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.peek();
        if b.is_some() {
            self.pos += 1;
        }
        b
    }

    fn expect(&mut self, want: u8) -> Result<()> {
        match self.bump() {
            Some(b) if b == want => Ok(()),
            _ => bail!("expected '{}' near byte {}", want as char, self.pos),
        }
    }

    fn skip_trivia(&mut self) {
        loop {
            while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
                self.pos += 1;
            }
            if self.bytes[self.pos..].starts_with(b"--") {
                self.pos += 2;
                if self.bytes[self.pos..].starts_with(b"[[") {
                    self.pos += 2;
                    match find_subslice(&self.bytes[self.pos..], b"]]") {
                        Some(rel) => self.pos += rel + 2,
                        None => self.pos = self.bytes.len(),
                    }
                } else {
                    while !matches!(self.peek(), None | Some(b'\n')) {
                        self.pos += 1;
                    }
                }
                continue;
            }
            break;
        }
    }

    fn is_identifier_start(&self) -> bool {
        matches!(self.peek(), Some(c) if c.is_ascii_alphabetic() || c == b'_')
    }

    fn parse_identifier(&mut self) -> Result<String> {
        if !self.is_identifier_start() {
            bail!("expected identifier near byte {}", self.pos);
        }
        let start = self.pos;
        self.pos += 1;
        while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || c == b'_') {
            self.pos += 1;
        }
        Ok(String::from_utf8_lossy(&self.bytes[start..self.pos]).into_owned())
    }

    fn parse_number(&mut self) -> Result<f64> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        let mut saw_digit = false;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.pos += 1;
            saw_digit = true;
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
                saw_digit = true;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            let save = self.pos;
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                    self.pos += 1;
                }
            } else {
                self.pos = save;
            }
        }
        if !saw_digit {
            bail!("expected a number near byte {start}");
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos]).unwrap_or_default();
        text.parse::<f64>()
            .map_err(|_| anyhow!("invalid number literal '{text}' near byte {start}"))
    }

    fn parse_string(&mut self) -> Result<String> {
        let quote = self.bump().context("expected a string literal")?;
        if quote != b'"' && quote != b'\'' {
            bail!("expected a string literal near byte {}", self.pos);
        }
        let mut buf: Vec<u8> = Vec::new();
        loop {
            match self.bump() {
                None => bail!("unterminated string literal"),
                Some(b) if b == quote => break,
                Some(b'\\') => self.parse_escape(&mut buf)?,
                Some(b) => buf.push(b),
            }
        }
        Ok(String::from_utf8_lossy(&buf).into_owned())
    }

    fn parse_escape(&mut self, buf: &mut Vec<u8>) -> Result<()> {
        match self.bump() {
            None => bail!("unterminated escape sequence"),
            Some(b'n') => buf.push(b'\n'),
            Some(b't') => buf.push(b'\t'),
            Some(b'r') => buf.push(b'\r'),
            Some(b'a') => buf.push(0x07),
            Some(b'b') => buf.push(0x08),
            Some(b'f') => buf.push(0x0C),
            Some(b'v') => buf.push(0x0B),
            Some(b'\\') => buf.push(b'\\'),
            Some(b'"') => buf.push(b'"'),
            Some(b'\'') => buf.push(b'\''),
            Some(b'\n') => buf.push(b'\n'),
            Some(c) if c.is_ascii_digit() => {
                let mut n = u32::from(c - b'0');
                for _ in 0..2 {
                    match self.peek() {
                        Some(d) if d.is_ascii_digit() => {
                            n = n * 10 + u32::from(self.bump().expect("peeked digit") - b'0');
                        }
                        _ => break,
                    }
                }
                if n > 255 {
                    bail!("decimal escape \\{n} out of byte range near byte {}", self.pos);
                }
                buf.push(n as u8);
            }
            Some(c) => buf.push(c),
        }
        Ok(())
    }

    fn parse_key(&mut self) -> Result<LuaKey> {
        match self.peek() {
            Some(b'"' | b'\'') => Ok(LuaKey::Str(self.parse_string()?)),
            Some(c) if c == b'-' || c.is_ascii_digit() => {
                Ok(LuaKey::Index(self.parse_number()? as i64))
            }
            _ => bail!("expected a string or number table key near byte {}", self.pos),
        }
    }

    fn parse_expr(&mut self, depth: usize) -> Result<LuaValue> {
        if depth > MAX_DEPTH {
            bail!("table nesting exceeds the depth cap of {MAX_DEPTH}");
        }
        self.skip_trivia();
        match self.peek() {
            Some(b'{') => self.parse_table(depth),
            Some(b'"' | b'\'') => Ok(LuaValue::Str(self.parse_string()?)),
            Some(c) if c == b'-' || c.is_ascii_digit() => {
                Ok(LuaValue::Number(self.parse_number()?))
            }
            Some(_) => {
                let word = self
                    .parse_identifier()
                    .context("expected a value (table, string, number, nil, true or false)")?;
                match word.as_str() {
                    "nil" => Ok(LuaValue::Nil),
                    "true" => Ok(LuaValue::Bool(true)),
                    "false" => Ok(LuaValue::Bool(false)),
                    other => bail!("unexpected token '{other}' near byte {}", self.pos),
                }
            }
            None => bail!("unexpected end of input while parsing a value"),
        }
    }

    /// Lookahead: does the table entry starting here look like `identifier =`? Restores position either way.
    fn peek_is_bare_key_assignment(&mut self) -> bool {
        let saved = self.pos;
        let result = self
            .parse_identifier()
            .ok()
            .map(|_| {
                self.skip_trivia();
                self.peek() == Some(b'=')
            })
            .unwrap_or(false);
        self.pos = saved;
        result
    }

    fn parse_table(&mut self, depth: usize) -> Result<LuaValue> {
        self.expect(b'{')?;
        let mut entries: Vec<(LuaKey, LuaValue)> = Vec::new();
        let mut next_index: i64 = 1;
        loop {
            self.skip_trivia();
            match self.peek() {
                Some(b'}') => {
                    self.bump();
                    break;
                }
                None => bail!("unterminated table"),
                Some(b'[') => {
                    self.bump();
                    self.skip_trivia();
                    let key = self.parse_key()?;
                    self.skip_trivia();
                    self.expect(b']')?;
                    self.skip_trivia();
                    self.expect(b'=')?;
                    self.skip_trivia();
                    let value = self.parse_expr(depth + 1)?;
                    entries.push((key, value));
                }
                Some(_) if self.is_identifier_start() && self.peek_is_bare_key_assignment() => {
                    let name = self.parse_identifier()?;
                    self.skip_trivia();
                    self.expect(b'=')?;
                    self.skip_trivia();
                    let value = self.parse_expr(depth + 1)?;
                    entries.push((LuaKey::Str(name), value));
                }
                Some(_) => {
                    let value = self.parse_expr(depth + 1)?;
                    entries.push((LuaKey::Index(next_index), value));
                    next_index += 1;
                }
            }
            self.skip_trivia();
            match self.peek() {
                Some(b',' | b';') => {
                    self.bump();
                }
                Some(b'}') => {}
                _ => bail!("expected ',' or '}}' near byte {}", self.pos),
            }
        }
        Ok(LuaValue::Table(entries))
    }

    fn parse_statement(&mut self, doc: &mut HashMap<String, LuaValue>) -> Result<()> {
        let name = self.parse_identifier()?;
        self.skip_trivia();
        let mut path: Vec<LuaKey> = Vec::new();
        while self.peek() == Some(b'[') {
            self.bump();
            self.skip_trivia();
            let key = self.parse_key()?;
            self.skip_trivia();
            self.expect(b']')?;
            self.skip_trivia();
            path.push(key);
        }
        self.expect(b'=')?;
        self.skip_trivia();
        let value = self.parse_expr(0)?;
        self.skip_trivia();
        if matches!(self.peek(), Some(b',' | b';')) {
            self.bump();
        }
        if path.is_empty() {
            doc.insert(name, value);
        } else {
            let entry = doc.entry(name).or_insert(LuaValue::Table(Vec::new()));
            set_path(entry, &path, value);
        }
        Ok(())
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Overwrite (or create) the value at `path` within `value`, turning non-table ancestors into tables as needed.
fn set_path(value: &mut LuaValue, path: &[LuaKey], new_value: LuaValue) {
    if !matches!(value, LuaValue::Table(_)) {
        *value = LuaValue::Table(Vec::new());
    }
    let LuaValue::Table(entries) = value else { unreachable!("just ensured this is a table") };
    let (first, rest) =
        path.split_first().expect("caller only calls set_path with a non-empty path");
    if let Some(existing) = entries.iter_mut().find(|(k, _)| k == first) {
        if rest.is_empty() {
            existing.1 = new_value;
        } else {
            set_path(&mut existing.1, rest, new_value);
        }
    } else if rest.is_empty() {
        entries.push((first.clone(), new_value));
    } else {
        let mut child = LuaValue::Table(Vec::new());
        set_path(&mut child, rest, new_value);
        entries.push((first.clone(), child));
    }
}

// --- Inbox staging (contract 7) ---------------------------------------------------------------------------------

fn lua_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out
}

/// Whether `text` has a top-level `name = {` assignment (allowing leading whitespace and whitespace around `=`),
/// i.e. whether it's safe to append/replace our inbox block.
fn has_top_level_table(text: &str, name: &str) -> bool {
    text.lines().any(|line| {
        let Some(rest) = line.trim_start().strip_prefix(name) else { return false };
        let Some(rest) = rest.trim_start().strip_prefix('=') else { return false };
        rest.trim_start().starts_with('{')
    })
}

/// Replace the `begin..end` marker block (inclusive) with `block`, or append `block` at the end of the file if no
/// such block exists yet. Idempotent: calling this twice with a different `block` leaves only the latest one.
fn replace_or_append_block(original: &str, begin: &str, end: &str, block: &str) -> String {
    if let Some(start_idx) = original.find(begin)
        && let Some(end_rel) = original[start_idx..].find(end)
    {
        let end_idx = start_idx + end_rel + end.len();
        let tail_start = if original[end_idx..].starts_with('\n') { end_idx + 1 } else { end_idx };
        let mut result = String::with_capacity(original.len() + block.len());
        result.push_str(&original[..start_idx]);
        result.push_str(block);
        result.push_str(&original[tail_start..]);
        return result;
    }
    let mut result = original.to_owned();
    if !result.ends_with('\n') {
        result.push('\n');
    }
    result.push('\n');
    result.push_str(block);
    result
}

/// Stage an `SQI1:` game import into a `SideQuest.lua` file per contract 7: refuses if the file has no top-level
/// `SideQuestDB = {` table, or if `generated_at` is older than the file's current `standings.generated_at`.
/// Writes atomically and keeps the previous content as `<path>.companion-bak`. Idempotent: calling it again with a
/// newer import replaces the marker block rather than duplicating it. Callers must only call this while WoW is not
/// running.
pub fn stage_inbox(
    path: &Path,
    payload: &str,
    generated_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<()> {
    let original =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    if !has_top_level_table(&original, "SideQuestDB") {
        bail!(
            "refusing to stage an import: {} has no top-level 'SideQuestDB = {{' table",
            path.display()
        );
    }
    if let Some(standings_at) = extract(&original).standings_generated_at
        && generated_at < standings_at
    {
        bail!(
            "refusing to stage an import generated at {generated_at} because it's older than the file's current standings ({standings_at})"
        );
    }
    let block = format!(
        "{INBOX_BEGIN}\nSideQuestDB[\"inbox\"] = {{ [\"payload\"] = \"{payload}\", [\"generated_at\"] = {gen_ts}, [\"staged_at\"] = {staged_ts} }}\n{INBOX_END}\n",
        payload = lua_escape(payload),
        gen_ts = generated_at.timestamp(),
        staged_ts = now.timestamp(),
    );
    let replaced = replace_or_append_block(&original, INBOX_BEGIN, INBOX_END, &block);

    let backup_path = backup_path_for(path);
    std::fs::write(&backup_path, &original)
        .with_context(|| format!("writing backup {}", backup_path.display()))?;

    atomic_write(path, replaced.as_bytes())
}

fn backup_path_for(path: &Path) -> std::path::PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".companion-bak");
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use tempfile::tempdir;

    use super::*;

    const FIXTURE: &str = r#"
SideQuestDB = {
    ["lastExport"] = "SQX1:AbC123+/==",
    ["pending"] = {
        ["sessions"] = {
            [1] = { ["id"] = "s1" },
            [2] = { ["id"] = "s2" },
        },
        ["loot"] = {},
        ["roster"] = {
            ["updated"] = true,
        },
    },
    ["standings"] = {
        ["generated_at"] = 1700000000,
    },
    ["notes"] = "a \"quoted\" line\nwith an escape \065\066\067",
    ["inbox"] = nil,
}
-- a trailing comment
SideQuestDB["inbox"] = {
    ["payload"] = "SQI1:zzz",
}
"#;

    #[test]
    fn extracts_last_export_pending_counts_standings_and_inbox() {
        let parsed = extract(FIXTURE);
        assert_eq!(parsed.last_export.as_deref(), Some("SQX1:AbC123+/=="));
        let pending = parsed.pending.expect("pending present");
        assert_eq!(pending.sessions, 2);
        assert_eq!(pending.loot, 0);
        assert!(pending.roster);
        assert_eq!(parsed.standings_generated_at, Utc.timestamp_opt(1_700_000_000, 0).single());
        // The later `SideQuestDB["inbox"] = {...}` statement overrides the earlier `nil`.
        assert!(parsed.inbox_staged);
    }

    #[test]
    fn decodes_decimal_and_backslash_string_escapes() {
        let doc = parse_document(FIXTURE).unwrap();
        let sqdb = doc.get("SideQuestDB").unwrap();
        assert_eq!(
            sqdb.get("notes").and_then(LuaValue::as_str),
            Some("a \"quoted\" line\nwith an escape ABC")
        );
    }

    #[test]
    fn legacy_file_falls_back_to_regex_match() {
        let text = "this is not valid lua at all but it has SQX1:deadBEEF42== in it somewhere";
        let parsed = extract(text);
        assert_eq!(parsed.last_export.as_deref(), Some("SQX1:deadBEEF42=="));
        assert!(parsed.pending.is_none());
    }

    #[test]
    fn takes_the_last_sqx1_token_when_several_are_present() {
        let text = "SQX1:first SQX1:second==";
        assert_eq!(extract(text).last_export.as_deref(), Some("SQX1:second=="));
    }

    #[test]
    fn later_top_level_assignment_wins_over_the_earlier_one() {
        let text = r#"
Foo = { ["a"] = 1 }
Foo = { ["a"] = 2 }
"#;
        let doc = parse_document(text).unwrap();
        assert_eq!(doc.get("Foo").unwrap().get("a").and_then(LuaValue::as_f64), Some(2.0));
    }

    #[test]
    fn later_key_assignment_overrides_a_single_field_without_touching_siblings() {
        let text = r#"
Foo = { ["a"] = 1, ["b"] = 2 }
Foo["a"] = 99
"#;
        let doc = parse_document(text).unwrap();
        let foo = doc.get("Foo").unwrap();
        assert_eq!(foo.get("a").and_then(LuaValue::as_f64), Some(99.0));
        assert_eq!(foo.get("b").and_then(LuaValue::as_f64), Some(2.0));
    }

    #[test]
    fn malformed_input_errors_without_panicking() {
        for bad in [
            "Foo = {",
            "Foo = { [1] = }",
            "Foo = \"unterminated",
            "Foo =",
            "= 1",
            "Foo = { 1 2 3 }",
        ] {
            assert!(parse_document(bad).is_err(), "expected '{bad}' to fail to parse");
        }
    }

    #[test]
    fn refuses_input_over_the_size_cap() {
        let huge = "x".repeat(MAX_SIZE_BYTES + 1);
        assert!(parse_document(&huge).is_err());
    }

    #[test]
    fn refuses_tables_nested_past_the_depth_cap() {
        let mut text = String::from("Foo = ");
        text.push_str(&"{ [1] = ".repeat(MAX_DEPTH + 5));
        text.push('1');
        text.push_str(&"}".repeat(MAX_DEPTH + 5));
        assert!(parse_document(&text).is_err());
    }

    fn write_savedvariables(dir: &Path, contents: &str) -> std::path::PathBuf {
        let path = dir.join("SideQuest.lua");
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn stage_inbox_appends_block_to_a_file_without_one() {
        let dir = tempdir().unwrap();
        let path = write_savedvariables(
            dir.path(),
            "SideQuestDB = {\n  [\"standings\"] = { [\"generated_at\"] = 100 },\n}\n",
        );
        let generated_at = Utc.timestamp_opt(200, 0).unwrap();
        let now = Utc.timestamp_opt(300, 0).unwrap();

        stage_inbox(&path, "SQI1:abc", generated_at, now).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(INBOX_BEGIN));
        assert!(text.contains("SQI1:abc"));
        assert!(text.contains("200"));
        assert!(text.contains("300"));
        assert!(extract(&text).inbox_staged);
    }

    #[test]
    fn stage_inbox_is_idempotent_and_replaces_the_previous_block() {
        let dir = tempdir().unwrap();
        let path = write_savedvariables(
            dir.path(),
            "SideQuestDB = {\n  [\"standings\"] = { [\"generated_at\"] = 100 },\n}\n",
        );
        let now = Utc.timestamp_opt(300, 0).unwrap();

        stage_inbox(&path, "SQI1:first", Utc.timestamp_opt(200, 0).unwrap(), now).unwrap();
        stage_inbox(&path, "SQI1:second", Utc.timestamp_opt(250, 0).unwrap(), now).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches(INBOX_BEGIN).count(), 1);
        assert!(!text.contains("SQI1:first"));
        assert!(text.contains("SQI1:second"));
    }

    #[test]
    fn stage_inbox_refuses_without_a_top_level_sidequestdb_table() {
        let dir = tempdir().unwrap();
        let path = write_savedvariables(dir.path(), "SomeOtherAddonDB = {}\n");
        let err = stage_inbox(
            &path,
            "SQI1:abc",
            Utc.timestamp_opt(1, 0).unwrap(),
            Utc.timestamp_opt(2, 0).unwrap(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("SideQuestDB"));
    }

    #[test]
    fn stage_inbox_refuses_an_import_older_than_current_standings() {
        let dir = tempdir().unwrap();
        let path = write_savedvariables(
            dir.path(),
            "SideQuestDB = {\n  [\"standings\"] = { [\"generated_at\"] = 1000 },\n}\n",
        );
        let err = stage_inbox(
            &path,
            "SQI1:abc",
            Utc.timestamp_opt(500, 0).unwrap(),
            Utc.timestamp_opt(600, 0).unwrap(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("older"));
    }

    #[test]
    fn stage_inbox_writes_a_backup_of_the_previous_content() {
        let dir = tempdir().unwrap();
        let original = "SideQuestDB = {\n  [\"standings\"] = { [\"generated_at\"] = 1 },\n}\n";
        let path = write_savedvariables(dir.path(), original);
        stage_inbox(
            &path,
            "SQI1:abc",
            Utc.timestamp_opt(2, 0).unwrap(),
            Utc.timestamp_opt(3, 0).unwrap(),
        )
        .unwrap();
        let backup = backup_path_for(&path);
        assert_eq!(std::fs::read_to_string(backup).unwrap(), original);
    }
}
