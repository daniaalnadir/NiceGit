//! A small JSON reader for the output of `gh` and `git lfs`. It accepts standard JSON and refuses
//! anything else, so a malformed response becomes an error rather than a partial value.

/// A parsed JSON value. Only what `gh` and `git lfs` output needs is supported, which is all of
/// the standard grammar.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    pub(crate) fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(fields) => fields.iter().find(|(name, _)| name == key).map(|(_, value)| value),
            _ => None,
        }
    }

    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(text) => Some(text),
            _ => None,
        }
    }

    pub(crate) fn as_number(&self) -> Option<f64> {
        match self {
            Json::Number(number) => Some(*number),
            _ => None,
        }
    }

    pub(crate) fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(value) => Some(*value),
            _ => None,
        }
    }
}

const MAX_DEPTH: usize = 64;

pub(crate) fn parse_json(text: &str) -> std::result::Result<Json, String> {
    let mut parser = JsonParser { bytes: text.as_bytes(), position: 0 };
    let value = parser.value(0)?;
    parser.skip_whitespace();
    if parser.position != parser.bytes.len() {
        return Err("Unexpected text after the JSON value.".to_string());
    }
    Ok(value)
}

struct JsonParser<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl JsonParser<'_> {
    fn skip_whitespace(&mut self) {
        while let Some(byte) = self.bytes.get(self.position) {
            if matches!(byte, b' ' | b'\t' | b'\n' | b'\r') {
                self.position += 1;
            } else {
                break;
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    fn expect_literal(&mut self, literal: &str) -> std::result::Result<(), String> {
        if self.bytes[self.position..].starts_with(literal.as_bytes()) {
            self.position += literal.len();
            Ok(())
        } else {
            Err(format!("Expected {literal}."))
        }
    }

    fn value(&mut self, depth: usize) -> std::result::Result<Json, String> {
        if depth > MAX_DEPTH {
            return Err("JSON is nested too deeply.".to_string());
        }
        self.skip_whitespace();
        match self.peek().ok_or("Unexpected end of JSON.")? {
            b'{' => self.object(depth),
            b'[' => self.array(depth),
            b'"' => self.string().map(Json::String),
            b't' => self.expect_literal("true").map(|_| Json::Bool(true)),
            b'f' => self.expect_literal("false").map(|_| Json::Bool(false)),
            b'n' => self.expect_literal("null").map(|_| Json::Null),
            b'-' | b'0'..=b'9' => self.number(),
            _ => Err("Unexpected character in JSON.".to_string()),
        }
    }

    fn object(&mut self, depth: usize) -> std::result::Result<Json, String> {
        self.position += 1;
        let mut fields = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.position += 1;
            return Ok(Json::Object(fields));
        }
        loop {
            self.skip_whitespace();
            if self.peek() != Some(b'"') {
                return Err("Expected an object key.".to_string());
            }
            let key = self.string()?;
            self.skip_whitespace();
            if self.peek() != Some(b':') {
                return Err("Expected ':' in an object.".to_string());
            }
            self.position += 1;
            let value = self.value(depth + 1)?;
            fields.push((key, value));
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.position += 1,
                Some(b'}') => {
                    self.position += 1;
                    return Ok(Json::Object(fields));
                }
                _ => return Err("Expected ',' or '}' in an object.".to_string()),
            }
        }
    }

    fn array(&mut self, depth: usize) -> std::result::Result<Json, String> {
        self.position += 1;
        let mut values = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.position += 1;
            return Ok(Json::Array(values));
        }
        loop {
            values.push(self.value(depth + 1)?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.position += 1,
                Some(b']') => {
                    self.position += 1;
                    return Ok(Json::Array(values));
                }
                _ => return Err("Expected ',' or ']' in an array.".to_string()),
            }
        }
    }

    fn number(&mut self) -> std::result::Result<Json, String> {
        let start = self.position;
        while let Some(byte) = self.peek() {
            if byte.is_ascii_digit() || matches!(byte, b'-' | b'+' | b'.' | b'e' | b'E') {
                self.position += 1;
            } else {
                break;
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.position]).map_err(|_| "Invalid number.".to_string())?;
        text.parse::<f64>().map(Json::Number).map_err(|_| "Invalid number.".to_string())
    }

    fn hex4(&mut self) -> std::result::Result<u32, String> {
        let digits = self.bytes.get(self.position..self.position + 4).ok_or("Invalid unicode escape.")?;
        let text = std::str::from_utf8(digits).map_err(|_| "Invalid unicode escape.".to_string())?;
        let value = u32::from_str_radix(text, 16).map_err(|_| "Invalid unicode escape.".to_string())?;
        self.position += 4;
        Ok(value)
    }

    fn string(&mut self) -> std::result::Result<String, String> {
        self.position += 1;
        let mut buffer: Vec<u8> = Vec::new();
        loop {
            let byte = self.peek().ok_or("Unterminated string in JSON.")?;
            self.position += 1;
            match byte {
                b'"' => break,
                b'\\' => {
                    let escape = self.peek().ok_or("Unterminated escape in JSON.")?;
                    self.position += 1;
                    let character = match escape {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let high = self.hex4()?;
                            if (0xD800..0xDC00).contains(&high) && self.bytes[self.position..].starts_with(b"\\u") {
                                self.position += 2;
                                let low = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&low) {
                                    return Err("Invalid surrogate pair in JSON.".to_string());
                                }
                                let code = 0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00);
                                char::from_u32(code).ok_or("Invalid surrogate pair in JSON.")?
                            } else {
                                char::from_u32(high).unwrap_or('\u{FFFD}')
                            }
                        }
                        _ => return Err("Invalid escape in JSON.".to_string()),
                    };
                    let mut encoded = [0u8; 4];
                    buffer.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
                }
                // Control characters must be escaped in JSON strings.
                0..=0x1f => return Err("Unescaped control character in JSON.".to_string()),
                other => buffer.push(other),
            }
        }
        String::from_utf8(buffer).map_err(|_| "Invalid UTF-8 in JSON.".to_string())
    }
}
