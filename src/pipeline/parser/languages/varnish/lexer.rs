#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Token {
    Ident(String),
    DottedPath(Vec<String>),
    ShortString(String),
    LongString(String),
    Integer(i64),
    Real(f64),
    Duration(f64, DurationUnit),
    Bytes(u64, ByteUnit),
    AttrName(String),
    Macro(String),
    Punct(char),
    Op(String),
    Comment(String),
    Eof,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum DurationUnit {
    Ms,
    S,
    M,
    H,
    D,
    W,
    Y,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ByteUnit {
    B,
    Kb,
    Mb,
    Gb,
    Tb,
}

/// Tokenize VCL source code.
pub(crate) fn tokenize(source: &str) -> Vec<Token> {
    Lexer::new(source).tokenize()
}

struct Lexer {
    chars: Vec<char>,
    pos: usize,
    in_attr_block: bool,
    /// When true, `#` starts a line comment (VTC context).
    hash_comments: bool,
}

impl Lexer {
    fn new(source: &str) -> Self {
        Self {
            chars: source.chars().collect(),
            pos: 0,
            in_attr_block: false,
            hash_comments: false,
        }
    }

    #[cfg(test)]
    fn new_hash_comments(source: &str) -> Self {
        Self {
            chars: source.chars().collect(),
            pos: 0,
            in_attr_block: false,
            hash_comments: true,
        }
    }

    fn tokenize(&mut self) -> Vec<Token> {
        let mut tokens = Vec::new();
        while self.pos < self.chars.len() {
            let c = self.chars[self.pos];
            if c.is_whitespace() {
                self.pos += 1;
                continue;
            }
            let tok = self.scan_token();
            tokens.push(tok);
        }
        tokens.push(Token::Eof);
        tokens
    }

    fn peek(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    fn advance(&mut self) {
        self.pos += 1;
    }

    fn scan_token(&mut self) -> Token {
        let c = self.chars[self.pos];

        if self.hash_comments && c == '#' {
            return self.scan_hash_comment();
        }

        match c {
            '/' => {
                if self.peek(1) == Some('/') {
                    self.scan_slash_comment()
                } else if self.peek(1) == Some('*') {
                    self.scan_block_comment()
                } else {
                    self.advance();
                    Token::Punct('/')
                }
            }
            '#' => {
                self.advance();
                Token::Punct('#')
            }
            '"' => {
                if self.peek(1) == Some('"') && self.peek(2) == Some('"') {
                    self.scan_triple_string()
                } else {
                    self.scan_short_string()
                }
            }
            '{' => {
                if self.peek(1) == Some('"') {
                    // gotcha 8: long string {"..."}
                    self.scan_long_string()
                } else {
                    self.advance();
                    Token::Punct('{')
                }
            }
            '$' if self.peek(1) == Some('{') => self.scan_macro(),
            '\'' => {
                self.advance();
                Token::Punct('\'')
            }
            c if c.is_ascii_digit() => self.scan_number(),
            c if c == '.' && self.peek(1).is_some_and(|n| n.is_ascii_digit()) => self.scan_number(),
            c if c == '.' && self.in_attr_block => self.scan_attr_name(),
            c if c.is_ascii_alphabetic() || c == '_' || c == '-' => self.scan_ident_or_path(),
            c if "+-*!~<>=&|".contains(c) => self.scan_operator_or_punct(),
            _ => {
                self.advance();
                Token::Punct(c)
            }
        }
    }

    fn scan_short_string(&mut self) -> Token {
        // gotcha 15: no escape processing — everything between quotes is verbatim
        self.advance(); // opening "
        let start = self.pos;
        while self.pos < self.chars.len() && self.chars[self.pos] != '"' {
            self.pos += 1;
        }
        let content: String = self.chars[start..self.pos].iter().collect();
        if self.pos < self.chars.len() {
            self.advance(); // closing "
        }
        Token::ShortString(content)
    }

    fn scan_triple_string(&mut self) -> Token {
        self.pos += 3; // skip """
        let start = self.pos;
        while self.pos + 2 < self.chars.len()
            && !(self.chars[self.pos] == '"'
                && self.chars[self.pos + 1] == '"'
                && self.chars[self.pos + 2] == '"')
        {
            self.pos += 1;
        }
        let content: String = self.chars[start..self.pos].iter().collect();
        if self.pos + 2 < self.chars.len() {
            self.pos += 3; // skip closing """
        }
        Token::LongString(content)
    }

    fn scan_long_string(&mut self) -> Token {
        // gotcha 8: {"..."} long string
        self.pos += 2; // skip {"
        let start = self.pos;
        while self.pos + 1 < self.chars.len()
            && !(self.chars[self.pos] == '"' && self.chars[self.pos + 1] == '}')
        {
            self.pos += 1;
        }
        let content: String = self.chars[start..self.pos].iter().collect();
        if self.pos + 1 < self.chars.len() {
            self.pos += 2; // skip "}
        }
        Token::LongString(content)
    }

    fn scan_slash_comment(&mut self) -> Token {
        self.pos += 2; // skip //
        let start = self.pos;
        while self.pos < self.chars.len() && self.chars[self.pos] != '\n' {
            self.pos += 1;
        }
        let s: String = self.chars[start..self.pos].iter().collect();
        Token::Comment(s)
    }

    fn scan_hash_comment(&mut self) -> Token {
        self.pos += 1; // skip #
        let start = self.pos;
        while self.pos < self.chars.len() && self.chars[self.pos] != '\n' {
            self.pos += 1;
        }
        let s: String = self.chars[start..self.pos].iter().collect();
        Token::Comment(s)
    }

    fn scan_block_comment(&mut self) -> Token {
        self.pos += 2; // skip /*
        let start = self.pos;
        while self.pos + 1 < self.chars.len()
            && !(self.chars[self.pos] == '*' && self.chars[self.pos + 1] == '/')
        {
            self.pos += 1;
        }
        let content: String = self.chars[start..self.pos].iter().collect();
        if self.pos + 1 < self.chars.len() {
            self.pos += 2; // skip */
        }
        Token::Comment(content)
    }

    fn scan_macro(&mut self) -> Token {
        self.pos += 2; // skip ${
        let start = self.pos;
        let mut depth = 1;
        while self.pos < self.chars.len() && depth > 0 {
            match self.chars[self.pos] {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
            if depth > 0 {
                self.pos += 1;
            }
        }
        let content: String = self.chars[start..self.pos].iter().collect();
        if self.pos < self.chars.len() {
            self.advance(); // skip }
        }
        Token::Macro(content)
    }

    fn scan_number(&mut self) -> Token {
        let start = self.pos;
        let mut is_float = false;

        while self.pos < self.chars.len() && self.chars[self.pos].is_ascii_digit() {
            self.pos += 1;
        }

        if self.pos < self.chars.len() && self.chars[self.pos] == '.' {
            self.pos += 1;
            while self.pos < self.chars.len() && self.chars[self.pos].is_ascii_digit() {
                self.pos += 1;
                is_float = true;
            }
        }

        // gotcha 6: maximal-munch duration suffix — try ms before m
        let num_str: String = self.chars[start..self.pos].iter().collect();

        if self.pos < self.chars.len() {
            let unit_str = self.scan_unit_suffix();
            if let Some(unit) = unit_str {
                if let Some(du) = parse_duration_unit(&unit) {
                    let val: f64 = num_str.parse().unwrap_or(0.0);
                    return Token::Duration(val, du);
                }
                if let Some(bu) = parse_byte_unit(&unit) {
                    let val: u64 = num_str.parse().unwrap_or(0);
                    return Token::Bytes(val, bu);
                }
            }
        }

        if is_float {
            let val: f64 = num_str.parse().unwrap_or(0.0);
            Token::Real(val)
        } else {
            let val: i64 = num_str.parse().unwrap_or(0);
            Token::Integer(val)
        }
    }

    fn scan_unit_suffix(&mut self) -> Option<String> {
        let mut suffix = String::new();
        let mut scan_pos = self.pos;
        while scan_pos < self.chars.len() && self.chars[scan_pos].is_ascii_alphabetic() {
            suffix.push(self.chars[scan_pos]);
            scan_pos += 1;
        }
        if suffix.is_empty() {
            return None;
        }
        // maximal-munch: try longest match first
        let s = suffix.as_str();
        let longest = if s.len() >= 2 {
            let two = &s[..2];
            if two == "ms" || two == "KB" || two == "MB" || two == "GB" || two == "TB" {
                self.pos += 2;
                Some(two.to_string())
            } else {
                None
            }
        } else {
            None
        };
        if longest.is_some() {
            return longest;
        }
        let first = &s[..1];
        self.pos += 1;
        Some(first.to_string())
    }

    fn scan_attr_name(&mut self) -> Token {
        let start = self.pos;
        self.advance(); // skip leading '.'
        while self.pos < self.chars.len()
            && (self.chars[self.pos].is_ascii_alphanumeric()
                || self.chars[self.pos] == '_'
                || self.chars[self.pos] == '-')
        {
            self.pos += 1;
        }
        let name: String = self.chars[start..self.pos].iter().collect();
        Token::AttrName(name)
    }

    fn is_ident_char(c: char) -> bool {
        c.is_ascii_alphanumeric() || c == '_' || c == '-'
    }

    fn scan_ident_or_path(&mut self) -> Token {
        let start = self.pos;

        // Consume identifier chars (gotcha 5: '-' is inside identifiers)
        while self.pos < self.chars.len() && Self::is_ident_char(self.chars[self.pos]) {
            self.pos += 1;
        }

        let mut segments: Vec<String> = vec![self.chars[start..self.pos].iter().collect()];

        // Check if this is a dotted path: ident.ident.ident...
        while self.pos < self.chars.len() && self.chars[self.pos] == '.' {
            self.pos += 1; // skip '.'

            // gotcha 7: quoted header name in dotted path
            if self.pos < self.chars.len() && self.chars[self.pos] == '"' {
                self.pos += 1; // skip opening "
                let seg_start = self.pos;
                while self.pos < self.chars.len() && self.chars[self.pos] != '"' {
                    self.pos += 1;
                }
                let seg: String = self.chars[seg_start..self.pos].iter().collect();
                if self.pos < self.chars.len() {
                    self.pos += 1; // skip closing "
                }
                segments.push(format!("\"{}\"", seg));
                continue;
            }

            // Otherwise, an alphanumeric segment
            if self.pos < self.chars.len()
                && (self.chars[self.pos].is_ascii_alphanumeric()
                    || self.chars[self.pos] == '_'
                    || self.chars[self.pos] == '-')
            {
                let seg_start = self.pos;
                while self.pos < self.chars.len() && Self::is_ident_char(self.chars[self.pos]) {
                    self.pos += 1;
                }
                segments.push(self.chars[seg_start..self.pos].iter().collect());
            } else {
                // Lone dot (e.g. method call)
                break;
            }
        }

        if segments.len() > 1 {
            Token::DottedPath(segments)
        } else {
            Token::Ident(segments.into_iter().next().unwrap())
        }
    }

    fn scan_operator_or_punct(&mut self) -> Token {
        let c = self.chars[self.pos];
        match c {
            '!' if self.peek(1) == Some('~') => {
                self.pos += 2;
                Token::Op("!~".to_string())
            }
            '~' if self.peek(1) == Some('=') => {
                self.pos += 2;
                Token::Op("~=".to_string())
            }
            '~' => {
                self.advance();
                // ~ can be regex match or subtraction; emit as regex op
                Token::Op("~".to_string())
            }
            '=' if self.peek(1) == Some('=') => {
                self.pos += 2;
                Token::Op("==".to_string())
            }
            '!' if self.peek(1) == Some('=') => {
                self.pos += 2;
                Token::Op("!=".to_string())
            }
            '<' if self.peek(1) == Some('=') => {
                self.pos += 2;
                Token::Op("<=".to_string())
            }
            '>' if self.peek(1) == Some('=') => {
                self.pos += 2;
                Token::Op(">=".to_string())
            }
            '&' if self.peek(1) == Some('&') => {
                self.pos += 2;
                Token::Op("&&".to_string())
            }
            '|' if self.peek(1) == Some('|') => {
                self.pos += 2;
                Token::Op("||".to_string())
            }
            '+' if self.peek(1) == Some('=') => {
                self.pos += 2;
                Token::Op("+=".to_string())
            }
            '+' => {
                self.advance();
                Token::Punct('+')
            }
            '-' if self.peek(1) == Some('=') => {
                self.pos += 2;
                Token::Op("-=".to_string())
            }
            '-' => {
                self.advance();
                Token::Punct('-')
            }
            '*' if self.peek(1) == Some('=') => {
                self.pos += 2;
                Token::Op("*=".to_string())
            }
            '*' => {
                self.advance();
                Token::Punct('*')
            }
            '/' if self.peek(1) == Some('=') => {
                self.pos += 2;
                Token::Op("/=".to_string())
            }
            _ => {
                self.advance();
                Token::Punct(c)
            }
        }
    }
}

fn parse_duration_unit(s: &str) -> Option<DurationUnit> {
    match s {
        "ms" => Some(DurationUnit::Ms),
        "s" => Some(DurationUnit::S),
        "m" => Some(DurationUnit::M),
        "h" => Some(DurationUnit::H),
        "d" => Some(DurationUnit::D),
        "w" => Some(DurationUnit::W),
        "y" => Some(DurationUnit::Y),
        _ => None,
    }
}

fn parse_byte_unit(s: &str) -> Option<ByteUnit> {
    match s {
        "B" => Some(ByteUnit::B),
        "KB" => Some(ByteUnit::Kb),
        "MB" => Some(ByteUnit::Mb),
        "GB" => Some(ByteUnit::Gb),
        "TB" => Some(ByteUnit::Tb),
        _ => None,
    }
}

/// Tokenize source with hash comments (# for line comments, VTC context).
#[cfg(test)]
pub(crate) fn tokenize_hash_comments(source: &str) -> Vec<Token> {
    Lexer::new_hash_comments(source).tokenize()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Gotcha 1: ACL mask outside quotes ----
    #[test]
    fn test_gotcha_1_acl_mask() {
        let tokens = tokenize(r#""192.0.2.0"/24"#);
        assert!(matches!(&tokens[0], Token::ShortString(_)));
        assert_eq!(tokens[1], Token::Punct('/'));
        assert!(matches!(&tokens[2], Token::Integer(24)));

        let tokens = tokenize(r#""firewall.example.com" / 24"#);
        assert!(matches!(&tokens[0], Token::ShortString(_)));
        assert_eq!(tokens[1], Token::Punct('/'));
        assert!(matches!(&tokens[2], Token::Integer(24)));
    }

    // ---- Gotcha 2: Adjacent string concatenation ----
    #[test]
    fn test_gotcha_2_adjacent_strings() {
        let tokens = tokenize(r#""GET / HTTP/1.1" "Host: x" "Connection: close""#);
        assert_eq!(tokens.len(), 4); // 3 strings + EOF
        assert!(matches!(&tokens[0], Token::ShortString(s) if s == "GET / HTTP/1.1"));
        assert!(matches!(&tokens[1], Token::ShortString(s) if s == "Host: x"));
        assert!(matches!(&tokens[2], Token::ShortString(s) if s == "Connection: close"));
    }

    // ---- Gotcha 3: Attribute names in backend/probe ----
    #[test]
    fn test_gotcha_3_attr_name_not_parsed_outside_block() {
        let tokens = tokenize(".host = \"x\";");
        // Outside attr block, .host is Punct('.') then Ident("host")
        assert!(matches!(&tokens[0], Token::Punct('.')));
        assert!(matches!(&tokens[1], Token::Ident(s) if s == "host"));
    }

    // ---- Gotcha 5: Hyphens in identifiers ----
    #[test]
    fn test_gotcha_5_hyphens_in_ident() {
        let tokens = tokenize("req.http.X-Forwarded-For");
        assert!(
            matches!(&tokens[0], Token::DottedPath(segs) if segs == &["req", "http", "X-Forwarded-For"])
        );
    }

    // ---- Gotcha 6: Duration maximal-munch ----
    #[test]
    fn test_gotcha_6_duration_ms_before_m() {
        let tokens = tokenize("10ms");
        assert!(matches!(
            &tokens[0],
            Token::Duration(10.0, DurationUnit::Ms)
        ));

        let tokens = tokenize("10m");
        assert!(matches!(&tokens[0], Token::Duration(10.0, DurationUnit::M)));

        let tokens = tokenize("1.5s");
        assert!(matches!(&tokens[0], Token::Duration(1.5, DurationUnit::S)));
    }

    // ---- Gotcha 7: Quoted header names ----
    #[test]
    fn test_gotcha_7_quoted_header_name() {
        let tokens = tokenize(r#"req.http."grammatically.valid""#);
        assert!(matches!(&tokens[0], Token::DottedPath(segs) if segs.len() == 3));
        if let Token::DottedPath(segs) = &tokens[0] {
            assert_eq!(segs[2], "\"grammatically.valid\"");
        }
    }

    // ---- Gotcha 8: Long strings ----
    #[test]
    fn test_gotcha_8_long_string_braces() {
        let tokens = tokenize("{\"long string\\nwith \" quotes\"}");
        assert!(matches!(&tokens[0], Token::LongString(s) if s.contains("long string")));
    }

    #[test]
    fn test_gotcha_8_triple_quoted_string() {
        let tokens = tokenize("\"\"\"long string\nwith quotes\"\"\"");
        assert!(matches!(&tokens[0], Token::LongString(s) if s.contains("long string")));
    }

    // ---- Gotcha 9: Extended status codes ----
    #[test]
    fn test_gotcha_9_extended_status() {
        let tokens = tokenize("return (synth(12404))");
        // Should parse synth as ident, 12404 as integer
        assert!(matches!(&tokens[4], Token::Integer(12404)));
    }

    // ---- Gotcha 10: Comment styles ----
    #[test]
    fn test_gotcha_10_comments() {
        let tokens = tokenize("// c++ style\n/* block\n comment */\nident");
        assert!(tokens.iter().any(|t| matches!(t, Token::Comment(_))));
        assert!(
            tokens
                .iter()
                .any(|t| matches!(t, Token::Ident(s) if s == "ident"))
        );
    }

    #[test]
    fn test_gotcha_10_hash_comment() {
        let tokens = tokenize_hash_comments("# this is a comment\nident");
        assert!(
            tokens
                .iter()
                .any(|t| matches!(t, Token::Comment(c) if c == " this is a comment"))
        );
        assert!(
            tokens
                .iter()
                .any(|t| matches!(t, Token::Ident(s) if s == "ident"))
        );
    }

    // ---- Gotcha 12: Macro tokens ----
    #[test]
    fn test_gotcha_12_macro() {
        let tokens = tokenize(r#"backend b { .port = ${s1_port}; }"#);
        assert!(
            tokens
                .iter()
                .any(|t| matches!(t, Token::Macro(m) if m == "s1_port"))
        );
    }

    // ---- Gotcha 15: No escape processing in strings ----
    #[test]
    fn test_gotcha_15_no_escape_in_strings() {
        let tokens = tokenize(r#""\R$""#);
        // \R should be verbatim, not an escape
        assert!(matches!(&tokens[0], Token::ShortString(s) if s == "\\R$"));

        // trailing backslash does NOT escape closing quote
        let tokens = tokenize(r#""C:\path\""#);
        assert!(matches!(&tokens[0], Token::ShortString(s) if s == "C:\\path\\"));

        // regex with backslash before closing quote
        let tokens = tokenize(r#""^(foo|bar)\?""#);
        assert!(matches!(
            &tokens[0], Token::ShortString(s) if s == "^(foo|bar)\\?"
        ));
    }

    // ---- General tests ----
    #[test]
    fn test_tokenize_empty() {
        let tokens = tokenize("");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0], Token::Eof);
    }

    #[test]
    fn test_tokenize_keywords_as_idents() {
        let tokens = tokenize("sub vcl_recv backend import");
        assert!(matches!(&tokens[0], Token::Ident(s) if s == "sub"));
        assert!(matches!(&tokens[1], Token::Ident(s) if s == "vcl_recv"));
        assert!(matches!(&tokens[2], Token::Ident(s) if s == "backend"));
        assert!(matches!(&tokens[3], Token::Ident(s) if s == "import"));
    }

    #[test]
    fn test_tokenize_vcl_version() {
        let tokens = tokenize("vcl 4.1;");
        assert!(matches!(&tokens[0], Token::Ident(s) if s == "vcl"));
        assert!(matches!(&tokens[1], Token::Real(4.1)));
        assert_eq!(tokens[2], Token::Punct(';'));
    }

    #[test]
    fn test_tokenize_tilde_operators() {
        let tokens = tokenize("~ !~ ~=");
        assert_eq!(tokens[0], Token::Op("~".to_string()));
        assert_eq!(tokens[1], Token::Op("!~".to_string()));
        assert_eq!(tokens[2], Token::Op("~=".to_string()));
    }
}
#[test]
fn test_debug_acl_tokens() {
    let source = "acl foo -pedantic +log +table +fold(-report) {";
    let tokens = tokenize(source);
    println!("{:#?}", tokens);
}
