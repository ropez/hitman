use std::{collections::VecDeque, ops::Range};

use super::{ParseError, Span};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token<'a> {
    VariableName(Span<'a>),
    VariableValue(Span<'a>),
    RequestName(Span<'a>),
    Method(Span<'a>),
    Target(Span<'a>),
    Version(Span<'a>),
    HeaderName(Span<'a>),
    HeaderValue(Span<'a>),
    Comment(Span<'a>),
    BlankLine,
    Body(Span<'a>),
    Eof,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpannedToken<'a> {
    pub kind: Token<'a>,
    /// Field span, except Method starts at the physical line start and BlankLine includes its newline.
    pub range: Range<usize>,
}

#[derive(Clone, Copy)]
enum Mode {
    Prefix,
    Headers,
    Body,
    Eof,
    Done,
}

/// A stateful, source-preserving lexer. It never interprets template expressions or body text.
pub struct Lexer<'a> {
    input: &'a str,
    offset: usize,
    mode: Mode,
    pending: VecDeque<SpannedToken<'a>>,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self {
        Self {
            input,
            offset: 0,
            mode: Mode::Prefix,
            pending: VecDeque::new(),
        }
    }

    fn emit(&mut self, kind: Token<'a>, range: Range<usize>) {
        self.pending.push_back(SpannedToken { kind, range });
    }

    fn field(&mut self, make: impl FnOnce(Span<'a>) -> Token<'a>, field: Span<'a>) {
        let range = field.range.clone();
        self.emit(make(field), range);
    }

    /// Consume exactly one physical line; byte offsets remain valid for UTF-8 and CRLF.
    fn line(&mut self) -> Option<(&'a str, Range<usize>)> {
        if self.offset == self.input.len() {
            return None;
        }
        let start = self.offset;
        let raw = self.input[start..].split_inclusive('\n').next()?;
        self.offset += raw.len();
        Some((raw.trim_end_matches(['\r', '\n']), start..self.offset))
    }

    fn tokenize_line(&mut self, content: &'a str, line: Range<usize>) -> Result<(), ParseError> {
        if content.trim().is_empty() {
            self.emit(Token::BlankLine, line);
            if matches!(self.mode, Mode::Headers) {
                self.mode = Mode::Body;
            }
            return Ok(());
        }
        if content.trim_start().starts_with('#') {
            if matches!(self.mode, Mode::Prefix) {
                if let Some(rest) = content.trim_start().strip_prefix("# @name ") {
                    let start = line.start + content.len() - rest.len();
                    self.field(
                        Token::RequestName,
                        Span::trimmed(self.input, start..line.start + content.len()),
                    );
                    return Ok(());
                }
            }
            self.field(
                Token::Comment,
                Span::new(self.input, line.start..line.start + content.len()),
            );
            return Ok(());
        }
        let (text, comment) = match content.split_once('#') {
            Some((before, _)) => (
                before.trim_end(),
                Some(line.start + before.len()..line.start + content.len()),
            ),
            None => (content, None),
        };
        let start = line.start;
        let end = start + text.len();
        match self.mode {
            Mode::Prefix if text.starts_with('@') => {
                let after_at = &text[1..];
                let Some((raw_name, _)) = after_at.split_once('=') else {
                    self.mode = Mode::Done;
                    return Err(ParseError {
                        message: format!(
                            "Invalid variable declaration `{after_at}`; expected `@name = value`"
                        ),
                        range: start..end,
                    });
                };
                let key = raw_name.trim();
                if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    self.mode = Mode::Done;
                    return Err(ParseError {
                        message: format!(
                            "Invalid variable name `{key}` in declaration `{after_at}`"
                        ),
                        range: start..end,
                    });
                }
                let equals = start + 1 + raw_name.len();
                self.field(
                    Token::VariableName,
                    Span::trimmed(self.input, start + 1..equals),
                );
                self.field(
                    Token::VariableValue,
                    Span::trimmed(self.input, equals + 1..end),
                );
            }
            Mode::Prefix => {
                let text = text.trim();
                let start = start + content.find(text).unwrap_or(0);
                let end = start + text.len();
                let method_end = text.find(char::is_whitespace).unwrap_or(text.len());
                let method = Span::new(self.input, start..start + method_end);
                self.emit(Token::Method(method), line.start..start + method_end);
                let rest = Span::trimmed(self.input, start + method_end..end);
                if let Some(pos) = rest.text.rfind(char::is_whitespace) {
                    let candidate =
                        Span::trimmed(self.input, rest.range.start + pos..rest.range.end);
                    if candidate.text.starts_with("HTTP/") {
                        self.field(
                            Token::Target,
                            Span::trimmed(self.input, rest.range.start..rest.range.start + pos),
                        );
                        self.field(Token::Version, candidate);
                    } else if !rest.text.is_empty() {
                        self.field(Token::Target, rest);
                    }
                } else if !rest.text.is_empty() {
                    self.field(Token::Target, rest);
                }
                self.mode = Mode::Headers;
            }
            Mode::Headers => {
                let Some(colon) = text.find(':') else {
                    return Err(ParseError {
                        message: format!("Invalid header line: {content}"),
                        range: start..start + content.len(),
                    });
                };
                self.field(
                    Token::HeaderName,
                    Span::trimmed(self.input, start..start + colon),
                );
                self.field(
                    Token::HeaderValue,
                    Span::trimmed(self.input, start + colon + 1..end),
                );
            }
            _ => unreachable!("body and EOF are emitted without tokenizing lines"),
        }
        if let Some(range) = comment {
            self.field(Token::Comment, Span::new(self.input, range));
        }
        Ok(())
    }
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Result<SpannedToken<'a>, ParseError>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(token) = self.pending.pop_front() {
            return Some(Ok(token));
        }
        match self.mode {
            Mode::Body => {
                self.mode = Mode::Eof;
                let body = Span::new(self.input, self.offset..self.input.len());
                self.offset = self.input.len();
                Some(Ok(SpannedToken {
                    range: body.range.clone(),
                    kind: Token::Body(body),
                }))
            }
            Mode::Eof => {
                self.mode = Mode::Done;
                Some(Ok(SpannedToken {
                    kind: Token::Eof,
                    range: self.offset..self.offset,
                }))
            }
            Mode::Done => None,
            Mode::Prefix | Mode::Headers => {
                if let Some((content, line)) = self.line() {
                    if let Err(error) = self.tokenize_line(content, line) {
                        return Some(Err(error));
                    }
                    self.next()
                } else {
                    self.mode = Mode::Eof;
                    self.next()
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexer_emits_spanned_fields_and_keeps_the_body_opaque() {
        let input = r##"# @name Example
@id=42 # note
POST https://example.com/{{ id }} HTTP/1.1
Content-Type: application/json

{"#": "@id"}
"##;
        let tokens = Lexer::new(input).collect::<Result<Vec<_>, _>>().unwrap();
        let at = |text: &str| {
            let start = input.find(text).unwrap();
            Span::new(input, start..start + text.len())
        };
        let kinds: Vec<_> = tokens.iter().map(|token| token.kind.clone()).collect();
        assert_eq!(
            kinds,
            vec![
                Token::RequestName(at("Example")),
                Token::VariableName(at("id")),
                Token::VariableValue(at("42")),
                Token::Comment(at("# note")),
                Token::Method(at("POST")),
                Token::Target(at("https://example.com/{{ id }}")),
                Token::Version(at("HTTP/1.1")),
                Token::HeaderName(at("Content-Type")),
                Token::HeaderValue(at("application/json")),
                Token::BlankLine,
                Token::Body(at("{\"#\": \"@id\"}\n")),
                Token::Eof,
            ]
        );
        let separator = input.find("\n\n").unwrap() + 1;
        assert_eq!(tokens[9].range, separator..separator + 1);
        assert_eq!(tokens[11].range, input.len()..input.len());
    }

    #[test]
    fn lexer_reports_bad_header_and_keeps_lexing() {
        let input = "GET https://example.com\nnot a header\nAccept: */*\n";
        let mut lexer = Lexer::new(input);
        assert!(matches!(
            lexer.next().unwrap().unwrap().kind,
            Token::Method(_)
        ));
        assert!(matches!(
            lexer.next().unwrap().unwrap().kind,
            Token::Target(_)
        ));
        assert!(lexer
            .next()
            .unwrap()
            .unwrap_err()
            .message
            .contains("Invalid header"));
        assert!(matches!(
            lexer.next().unwrap().unwrap().kind,
            Token::HeaderName(_)
        ));
    }
}
