//! A JSON parser, because glTF is JSON and there is no third-party crate to
//! reach for.
//!
//! It is a small, strict parser: it accepts what the specification says and
//! nothing more, which is the right trade for reading asset files. Objects
//! keep their keys in the order they appeared rather than in a hash map,
//! because iteration order that changes between runs is exactly the sort of
//! thing that makes a "deterministic" pipeline produce two different files
//! from one input.

use std::fmt;

/// A parsed JSON value.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// Any number; JSON does not distinguish integers.
    Number(f64),
    /// A string, with escapes already resolved.
    Text(String),
    /// An array.
    Array(Vec<Json>),
    /// An object, in the order its keys appeared.
    Object(Vec<(String, Json)>),
}

/// Why some text is not JSON.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonError {
    /// Byte offset where the trouble was found.
    pub position: usize,
    /// What went wrong.
    pub reason: &'static str,
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.reason, self.position)
    }
}

impl std::error::Error for JsonError {}

/// How deeply values may nest.
///
/// A limit rather than recursion until the stack runs out: a file of ten
/// thousand open brackets is a crash otherwise, and asset files come from
/// wherever the artist got them.
const MAX_DEPTH: usize = 128;

impl Json {
    /// Parse a whole document.
    pub fn parse(text: &str) -> Result<Json, JsonError> {
        let mut parser = Parser {
            bytes: text.as_bytes(),
            position: 0,
            depth: 0,
        };
        parser.skip_whitespace();
        let value = parser.value()?;
        parser.skip_whitespace();
        if parser.position != parser.bytes.len() {
            return Err(parser.error("trailing characters after the value"));
        }
        Ok(value)
    }

    /// A member of an object.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(members) => members
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    /// An element of an array.
    pub fn at(&self, index: usize) -> Option<&Json> {
        match self {
            Json::Array(items) => items.get(index),
            _ => None,
        }
    }

    /// The value as a number.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Number(value) => Some(*value),
            _ => None,
        }
    }

    /// The value as a number, rounded toward zero.
    pub fn as_usize(&self) -> Option<usize> {
        match self {
            Json::Number(value) if *value >= 0.0 && value.is_finite() => Some(*value as usize),
            _ => None,
        }
    }

    /// The value as text.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Text(value) => Some(value),
            _ => None,
        }
    }

    /// The value as a boolean.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(value) => Some(*value),
            _ => None,
        }
    }

    /// The value as an array.
    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Array(items) => Some(items),
            _ => None,
        }
    }

    /// The members of an object.
    pub fn as_object(&self) -> Option<&[(String, Json)]> {
        match self {
            Json::Object(members) => Some(members),
            _ => None,
        }
    }

    /// Whether this is `null`.
    pub fn is_null(&self) -> bool {
        matches!(self, Json::Null)
    }

    /// Follow a path of object keys, as a convenience for reaching into a
    /// document: `document.path(&["meshes", "0"])` is not supported — use
    /// [`Json::at`] for arrays.
    pub fn path(&self, keys: &[&str]) -> Option<&Json> {
        let mut current = self;
        for key in keys {
            current = current.get(key)?;
        }
        Some(current)
    }
}

struct Parser<'a> {
    bytes: &'a [u8],
    position: usize,
    depth: usize,
}

impl<'a> Parser<'a> {
    fn error(&self, reason: &'static str) -> JsonError {
        JsonError {
            position: self.position,
            reason,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.position += 1;
        }
    }

    fn expect(&mut self, byte: u8, reason: &'static str) -> Result<(), JsonError> {
        if self.peek() == Some(byte) {
            self.position += 1;
            Ok(())
        } else {
            Err(self.error(reason))
        }
    }

    fn value(&mut self) -> Result<Json, JsonError> {
        if self.depth >= MAX_DEPTH {
            return Err(self.error("nested too deeply"));
        }
        match self.peek() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => Ok(Json::Text(self.string()?)),
            Some(b't') => self.literal("true", Json::Bool(true)),
            Some(b'f') => self.literal("false", Json::Bool(false)),
            Some(b'n') => self.literal("null", Json::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.error("expected a value")),
            None => Err(self.error("the document ended early")),
        }
    }

    fn literal(&mut self, word: &str, value: Json) -> Result<Json, JsonError> {
        if self.bytes[self.position..].starts_with(word.as_bytes()) {
            self.position += word.len();
            Ok(value)
        } else {
            Err(self.error("expected a value"))
        }
    }

    fn object(&mut self) -> Result<Json, JsonError> {
        self.expect(b'{', "expected an object")?;
        self.depth += 1;
        let mut members = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.position += 1;
            self.depth -= 1;
            return Ok(Json::Object(members));
        }
        loop {
            self.skip_whitespace();
            let key = self.string()?;
            self.skip_whitespace();
            self.expect(b':', "expected a colon after the key")?;
            self.skip_whitespace();
            let value = self.value()?;
            members.push((key, value));
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.position += 1,
                Some(b'}') => {
                    self.position += 1;
                    self.depth -= 1;
                    return Ok(Json::Object(members));
                }
                _ => return Err(self.error("expected a comma or a closing brace")),
            }
        }
    }

    fn array(&mut self) -> Result<Json, JsonError> {
        self.expect(b'[', "expected an array")?;
        self.depth += 1;
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.position += 1;
            self.depth -= 1;
            return Ok(Json::Array(items));
        }
        loop {
            self.skip_whitespace();
            items.push(self.value()?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.position += 1,
                Some(b']') => {
                    self.position += 1;
                    self.depth -= 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(self.error("expected a comma or a closing bracket")),
            }
        }
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.expect(b'"', "expected a string")?;
        let mut out = String::new();
        loop {
            let Some(byte) = self.peek() else {
                return Err(self.error("the string was never closed"));
            };
            self.position += 1;
            match byte {
                b'"' => return Ok(out),
                b'\\' => {
                    let Some(escape) = self.peek() else {
                        return Err(self.error("the string was never closed"));
                    };
                    self.position += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => out.push(self.unicode_escape()?),
                        _ => return Err(self.error("unknown escape")),
                    }
                }
                // Control characters must be escaped; a raw newline inside a
                // string is a malformed document, not a lenient one.
                0x00..=0x1f => return Err(self.error("unescaped control character")),
                _ => {
                    // Copy the whole UTF-8 sequence, which the input is
                    // already guaranteed to contain because it came from &str.
                    let start = self.position - 1;
                    let length = utf8_length(byte);
                    self.position = start + length;
                    match core::str::from_utf8(
                        &self.bytes[start..self.position.min(self.bytes.len())],
                    ) {
                        Ok(text) => out.push_str(text),
                        Err(_) => return Err(self.error("invalid UTF-8 in a string")),
                    }
                }
            }
        }
    }

    /// A `\uXXXX` escape, including the surrogate pairs that carry anything
    /// above the basic plane.
    fn unicode_escape(&mut self) -> Result<char, JsonError> {
        let first = self.hex4()?;
        if !(0xD800..0xDC00).contains(&first) {
            return char::from_u32(first).ok_or_else(|| self.error("invalid escape"));
        }
        // A high surrogate must be followed by its low partner.
        if self.peek() != Some(b'\\') {
            return Err(self.error("a lone surrogate escape"));
        }
        self.position += 1;
        self.expect(b'u', "a lone surrogate escape")?;
        let second = self.hex4()?;
        if !(0xDC00..0xE000).contains(&second) {
            return Err(self.error("a lone surrogate escape"));
        }
        let combined = 0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00);
        char::from_u32(combined).ok_or_else(|| self.error("invalid escape"))
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        if self.position + 4 > self.bytes.len() {
            return Err(self.error("a truncated escape"));
        }
        let mut value = 0u32;
        for _ in 0..4 {
            let byte = self.bytes[self.position];
            let digit = match byte {
                b'0'..=b'9' => u32::from(byte - b'0'),
                b'a'..=b'f' => u32::from(byte - b'a') + 10,
                b'A'..=b'F' => u32::from(byte - b'A') + 10,
                _ => return Err(self.error("a malformed escape")),
            };
            value = value * 16 + digit;
            self.position += 1;
        }
        Ok(value)
    }

    fn number(&mut self) -> Result<Json, JsonError> {
        let start = self.position;
        if self.peek() == Some(b'-') {
            self.position += 1;
        }
        // An integer part is required, and a leading zero may not be followed
        // by more digits.
        match self.peek() {
            Some(b'0') => self.position += 1,
            Some(b'1'..=b'9') => {
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.position += 1;
                }
            }
            _ => return Err(self.error("expected a number")),
        }
        if self.peek() == Some(b'.') {
            self.position += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.error("expected digits after the decimal point"));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.position += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.position += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.position += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.error("expected digits in the exponent"));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.position += 1;
            }
        }

        let text = core::str::from_utf8(&self.bytes[start..self.position])
            .map_err(|_| self.error("expected a number"))?;
        text.parse::<f64>()
            .map(Json::Number)
            .map_err(|_| self.error("expected a number"))
    }
}

/// Length in bytes of a UTF-8 sequence from its first byte.
fn utf8_length(first: u8) -> usize {
    match first {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shapes_of_json_all_parse() {
        assert_eq!(Json::parse("null").unwrap(), Json::Null);
        assert_eq!(Json::parse("true").unwrap(), Json::Bool(true));
        assert_eq!(Json::parse("false").unwrap(), Json::Bool(false));
        assert_eq!(Json::parse("42").unwrap(), Json::Number(42.0));
        assert_eq!(
            Json::parse("\"hello\"").unwrap(),
            Json::Text("hello".into())
        );
        assert_eq!(Json::parse("[]").unwrap(), Json::Array(vec![]));
        assert_eq!(Json::parse("{}").unwrap(), Json::Object(vec![]));
    }

    #[test]
    fn numbers_follow_the_specification() {
        let number = |text: &str| Json::parse(text).unwrap().as_f64().unwrap();
        assert_eq!(number("0"), 0.0);
        assert_eq!(number("-17"), -17.0);
        assert_eq!(number("3.5"), 3.5);
        assert_eq!(number("1e3"), 1000.0);
        assert_eq!(number("1.5E-2"), 0.015);
        assert_eq!(number("-0.0"), 0.0);

        // And what the specification does not allow.
        for bad in [
            "01", "+1", ".5", "1.", "1e", "1e+", "--1", "0x10", "Infinity", "NaN",
        ] {
            assert!(Json::parse(bad).is_err(), "{bad} should not parse");
        }
    }

    #[test]
    fn strings_handle_their_escapes() {
        let text = |json: &str| Json::parse(json).unwrap().as_str().unwrap().to_string();
        assert_eq!(text(r#""a\nb""#), "a\nb");
        assert_eq!(
            text(r#""quote: \" backslash: \\ slash: \/""#),
            "quote: \" backslash: \\ slash: /"
        );
        assert_eq!(text(r#""\u0041\u00e9""#), "Aé");
        // A surrogate pair, which is how anything above the basic plane is
        // written in JSON.
        assert_eq!(text(r#""\ud83d\ude00""#), "😀");
        // And non-ASCII that is simply there, unescaped.
        assert_eq!(text("\"деревня\""), "деревня");

        for bad in [
            r#""\u00""#,         // truncated
            r#""\q""#,           // unknown escape
            r#""\ud800""#,       // a lone high surrogate
            r#""\ud800\u0041""#, // followed by something that is not its pair
            "\"unclosed",
            "\"raw\nnewline\"",
        ] {
            assert!(Json::parse(bad).is_err(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn objects_keep_their_order() {
        // Not a hash map: iteration order that changes between runs is how a
        // "deterministic" asset pipeline produces two files from one input.
        let document = Json::parse(r#"{"z":1,"a":2,"m":3}"#).unwrap();
        let keys: Vec<&str> = document
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, _)| k.as_str())
            .collect();
        assert_eq!(keys, vec!["z", "a", "m"]);
        assert_eq!(document.get("a").unwrap().as_f64(), Some(2.0));
        assert_eq!(document.get("missing"), None);
    }

    #[test]
    fn nesting_works_and_can_be_walked() {
        let document = Json::parse(
            r#"{
                "asset": {"version": "2.0"},
                "meshes": [
                    {"name": "cart", "primitives": [{"indices": 3}]}
                ]
            }"#,
        )
        .unwrap();

        assert_eq!(
            document.path(&["asset", "version"]).unwrap().as_str(),
            Some("2.0")
        );
        let mesh = document.get("meshes").unwrap().at(0).unwrap();
        assert_eq!(mesh.get("name").unwrap().as_str(), Some("cart"));
        assert_eq!(
            mesh.path(&["primitives"])
                .unwrap()
                .at(0)
                .unwrap()
                .get("indices")
                .unwrap()
                .as_usize(),
            Some(3)
        );
        assert_eq!(document.at(0), None, "an object is not an array");
    }

    #[test]
    fn whitespace_anywhere_is_fine() {
        let document = Json::parse("  {\n\t\"a\" : [ 1 , 2 ]\r\n}  ").unwrap();
        assert_eq!(document.get("a").unwrap().as_array().unwrap().len(), 2);
    }

    #[test]
    fn malformed_documents_are_refused_with_a_position() {
        let cases = [
            ("", "the document ended early"),
            ("{", "expected a string"),
            ("{\"a\"}", "expected a colon after the key"),
            ("{\"a\":1,}", "expected a string"),
            ("[1,2", "expected a comma or a closing bracket"),
            ("[1 2]", "expected a comma or a closing bracket"),
            ("nul", "expected a value"),
            ("{} extra", "trailing characters after the value"),
        ];
        for (text, reason) in cases {
            let error = Json::parse(text).unwrap_err();
            assert_eq!(error.reason, reason, "parsing {text:?}");
            assert!(error.position <= text.len());
        }
    }

    #[test]
    fn a_hostile_document_does_not_blow_the_stack() {
        // Ten thousand open brackets is a crash for a naive recursive parser,
        // and asset files come from wherever the artist got them.
        let deep = "[".repeat(10_000);
        let error = Json::parse(&deep).unwrap_err();
        assert_eq!(error.reason, "nested too deeply");

        let deep = format!("{}{}", "[".repeat(10_000), "]".repeat(10_000));
        assert!(Json::parse(&deep).is_err());

        // Just inside the limit still works.
        let shallow = format!("{}1{}", "[".repeat(100), "]".repeat(100));
        assert!(Json::parse(&shallow).is_ok());
    }

    #[test]
    fn random_rubbish_never_panics() {
        let mut rng = runity_math::Rng::named(1, "json fuzz");
        let alphabet = b"{}[]\",:0123456789.eE+-tfnul \\u\n";
        for _ in 0..20_000 {
            let length = rng.below(40) as usize;
            let text: String = (0..length)
                .map(|_| alphabet[rng.below(alphabet.len() as u32) as usize] as char)
                .collect();
            let _ = Json::parse(&text);
        }
    }

    #[test]
    fn every_prefix_of_a_real_document_is_refused_rather_than_misread() {
        let document = r#"{"asset":{"version":"2.0"},"nodes":[{"mesh":0,"name":"a"}]}"#;
        for length in 0..document.len() {
            let prefix = &document[..length];
            if let Ok(value) = Json::parse(prefix) {
                panic!("the prefix {prefix:?} parsed as {value:?}");
            }
        }
        assert!(Json::parse(document).is_ok());
    }

    #[test]
    fn accessors_return_nothing_for_the_wrong_type() {
        let document = Json::parse(r#"{"a": "text", "b": 1, "c": null}"#).unwrap();
        assert_eq!(document.get("a").unwrap().as_f64(), None);
        assert_eq!(document.get("b").unwrap().as_str(), None);
        assert_eq!(document.get("b").unwrap().as_bool(), None);
        assert!(document.get("c").unwrap().is_null());
        assert_eq!(
            Json::parse("-3").unwrap().as_usize(),
            None,
            "negative is not an index"
        );
    }
}
