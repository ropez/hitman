//! Source-preserving lexer and parser for one `.http` request file.
//! The body is opaque: `#`, blank lines, and template syntax in it are never parsed as headers.
use std::ops::Range;

mod lexer;
mod parser;

pub use lexer::{Lexer, SpannedToken, Token};
pub use parser::parse;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span<'a> {
    pub text: &'a str,
    pub range: Range<usize>,
}

impl<'a> Span<'a> {
    pub fn new(input: &'a str, range: Range<usize>) -> Self {
        Self {
            text: &input[range.clone()],
            range,
        }
    }

    pub fn trimmed(input: &'a str, range: Range<usize>) -> Self {
        let raw = &input[range.clone()];
        let start = range.start + raw.len() - raw.trim_start().len();
        let end = start + raw.trim().len();
        Self::new(input, start..end)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variable<'a> {
    pub name: Span<'a>,
    pub value: Span<'a>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header<'a> {
    pub name: Span<'a>,
    pub value: Span<'a>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestLine<'a> {
    pub method: Span<'a>,
    pub target: Option<Span<'a>>,
    pub version: Option<Span<'a>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request<'a> {
    pub name: Option<Span<'a>>,
    pub line: RequestLine<'a>,
    pub headers: Vec<Header<'a>>,
    /// None if no blank separator occurs; Some even for an empty body.
    pub body: Option<Span<'a>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    pub range: Range<usize>,
}

#[derive(Debug)]
pub struct Document<'a> {
    pub variables: Vec<Variable<'a>>,
    pub request: Option<Request<'a>>,
    pub errors: Vec<ParseError>,
    /// Byte offset after declarations and leading trivia.
    pub request_start: usize,
}
