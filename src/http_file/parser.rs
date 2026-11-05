use super::{
    lexer::{Lexer, Token},
    Document, Header, Request, RequestLine, Variable,
};

/// Parses the complete file, retaining valid prefix nodes when editing leaves a line incomplete.
/// Execution should reject `errors`; the LSP can still use the nodes parsed before an error.
pub fn parse(input: &str) -> Document<'_> {
    let mut document = Document {
        variables: Vec::new(),
        request: None,
        errors: Vec::new(),
        request_start: input.len(),
    };
    let mut name = None;
    let mut variable_name = None;
    let mut header_name = None;
    for token in Lexer::new(input) {
        let token = match token {
            Ok(token) => token,
            Err(error) => {
                document.errors.push(error);
                continue;
            }
        };
        match token.kind {
            Token::VariableName(value) => variable_name = Some(value),
            Token::VariableValue(value) => {
                if let Some(name) = variable_name.take() {
                    document.variables.push(Variable { name, value });
                }
            }
            Token::RequestName(value) => name = Some(value),
            Token::Method(method) => {
                document.request_start = token.range.start;
                document.request = Some(Request {
                    name: name.take(),
                    line: RequestLine {
                        method,
                        target: None,
                        version: None,
                    },
                    headers: Vec::new(),
                    body: None,
                });
            }
            Token::Target(value) => {
                if let Some(request) = document.request.as_mut() {
                    request.line.target = Some(value);
                }
            }
            Token::Version(value) => {
                if let Some(request) = document.request.as_mut() {
                    request.line.version = Some(value);
                }
            }
            Token::HeaderName(value) => header_name = Some(value),
            Token::HeaderValue(value) => {
                if let (Some(request), Some(name)) = (document.request.as_mut(), header_name.take())
                {
                    request.headers.push(Header { name, value });
                }
            }
            Token::Body(value) => {
                if let Some(request) = document.request.as_mut() {
                    request.body = Some(value);
                }
            }
            Token::Comment(_) | Token::BlankLine | Token::Eof => {}
        }
    }
    document
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_complete_file_with_source_spans_and_opaque_body() {
        let input = r##"# @name Fetch user
# note
@  id = 42 # comment
GET https://example.com/{{ id }} HTTP/1.1 # note
# header note
Accept: application/json # note

{
  "#text": "@id"
}
"##
        .replace('\n', "\r\n");
        let doc = parse(&input);
        assert!(doc.errors.is_empty(), "{:?}", doc.errors);
        assert_eq!(doc.variables[0].name.text, "id");
        assert_eq!(&input[doc.variables[0].name.range.clone()], "id");
        assert_eq!(doc.variables[0].value.text, "42");
        let request = doc.request.unwrap();
        assert_eq!(request.name.unwrap().text, "Fetch user");
        assert_eq!(request.line.method.text, "GET");
        assert_eq!(
            request.line.target.unwrap().text,
            "https://example.com/{{ id }}"
        );
        assert_eq!(request.line.version.unwrap().text, "HTTP/1.1");
        assert_eq!(request.headers[0].name.text, "Accept");
        assert_eq!(request.headers[0].value.text, "application/json");
        let expected_body = r##"{
  "#text": "@id"
}
"##
        .replace('\n', "\r\n");
        assert_eq!(request.body.unwrap().text, expected_body);
    }

    #[test]
    fn incomplete_declaration_keeps_prior_nodes_for_editor() {
        let doc = parse(
            r#"@id=ok
# comment
@bad-name=no
GET /
"#,
        );
        assert_eq!(doc.variables.len(), 1);
        assert!(doc.errors[0].message.contains("Invalid variable name"));
    }

    #[test]
    fn no_body_without_separator() {
        let doc = parse(
            r#"GET https://example.com
Accept: */*
"#,
        );
        let request = doc.request.unwrap();
        assert!(request.line.version.is_none());
        assert!(request.body.is_none());
    }
}
