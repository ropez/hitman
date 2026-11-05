use std::{collections::HashMap, fs::read_to_string, path::Path, sync::Arc, time::Duration};

use anyhow::Result;
use reqwest::Client;
use serde_json::Value;

use crate::{
    http_file,
    resolve::{Resolved, ResolvedAs},
    substitute::{self, SubstituteProvider},
};

pub mod graphql;
pub mod grpc;
pub mod http;

#[derive(Clone)]
pub enum PreparedRequest {
    Http(http::HttpRequest),
    GraphQL(graphql::GraphQLRequest),
    Grpc(grpc::GrpcRequest),
}

pub struct ExecutionResult {
    pub elapsed: Duration,
    pub json: Option<Value>,
}

pub struct TransportResponse {
    pub elapsed: Duration,
    pub response: TransportResponseBody,
}

impl TransportResponse {
    pub fn status_code(&self) -> u16 {
        match &self.response {
            TransportResponseBody::Http(resp) => resp.status().as_u16(),
            TransportResponseBody::GraphQL(resp) => resp.status().as_u16(),
            TransportResponseBody::Grpc(_) => 0, // gRPC status code is 0 for when we have a success body
        }
    }
}

pub enum TransportResponseBody {
    Http(reqwest::Response),
    GraphQL(reqwest::Response),
    Grpc(Value),
}

pub fn prepare_request(
    resolved: &Resolved,
    provider: Arc<dyn SubstituteProvider + Send + Sync + 'static>,
) -> Result<PreparedRequest> {
    let input = read_to_string(resolved.http_file())?;
    let working_dir = resolved.http_file().parent().unwrap_or(Path::new("."));
    let (template, local_values) = parse_local_variables(&input)?;
    let rendered =
        substitute::substitute_with_locals(&template, provider.clone(), working_dir, local_values)?;

    match &resolved.resolved_as {
        ResolvedAs::Simple { .. } if grpc::is_grpc_request(&rendered) => Ok(PreparedRequest::Grpc(
            grpc::prepare_request(resolved, &rendered)?,
        )),
        ResolvedAs::Simple { .. } => Ok(PreparedRequest::Http(http::prepare_request(&rendered)?)),
        ResolvedAs::GraphQL { .. } => Ok(PreparedRequest::GraphQL(graphql::prepare_request(
            resolved, &rendered, provider,
        )?)),
    }
}

fn parse_local_variables(input: &str) -> Result<(String, HashMap<String, minijinja::Value>)> {
    let document = http_file::parse(input);
    if let Some(error) = document
        .errors
        .iter()
        .find(|error| document.request.is_none() || error.range.start < document.request_start)
    {
        anyhow::bail!("{}", error.message);
    }
    let body_start = if document.variables.is_empty() {
        0
    } else {
        document.request_start
    };
    let values = document
        .variables
        .into_iter()
        .map(|variable| {
            (
                variable.name.text.to_string(),
                minijinja::Value::from(variable.value.text.to_string()),
            )
        })
        .collect();
    Ok((input[body_start..].to_string(), values))
}

pub async fn send(client: &Client, request: &PreparedRequest) -> Result<TransportResponse> {
    match request {
        PreparedRequest::Http(req) => {
            let (response, elapsed) = http::send(client, req).await?;
            Ok(TransportResponse {
                elapsed,
                response: TransportResponseBody::Http(response),
            })
        }
        PreparedRequest::GraphQL(req) => {
            let (response, elapsed) = http::send(client, &req.http).await?;
            Ok(TransportResponse {
                elapsed,
                response: TransportResponseBody::GraphQL(response),
            })
        }
        PreparedRequest::Grpc(req) => {
            let (json, elapsed) = grpc::send(req).await?;
            Ok(TransportResponse {
                elapsed,
                response: TransportResponseBody::Grpc(json),
            })
        }
    }
}

pub async fn finish_response(response: TransportResponse) -> Result<ExecutionResult> {
    let json = match response.response {
        TransportResponseBody::Http(response) => http::finish_response(response).await?,
        TransportResponseBody::GraphQL(response) => graphql::finish_response(response).await?,
        TransportResponseBody::Grpc(json) => grpc::finish_response(&json)?,
    };

    Ok(ExecutionResult {
        elapsed: response.elapsed,
        json,
    })
}

pub fn print_request(request: &PreparedRequest) {
    match request {
        PreparedRequest::Http(req) => http::print_request(req),
        PreparedRequest::GraphQL(req) => graphql::print_request(req),
        PreparedRequest::Grpc(req) => grpc::print_request(req),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_local_variables;

    #[test]
    fn parses_leading_local_variables_and_removes_them_from_request() {
        let (body, values) = parse_local_variables(
            "@base_url = https://example.com\n@user_id = 42\n\nGET {{base_url}}/users/{{user_id}} HTTP/1.1\n",
        ).unwrap();

        assert_eq!(body, "GET {{base_url}}/users/{{user_id}} HTTP/1.1\n");
        assert_eq!(values["base_url"].as_str(), Some("https://example.com"));
        assert_eq!(values["user_id"].as_str(), Some("42"));
    }

    #[test]
    fn ignores_trailing_comments_on_local_variable_declarations() {
        let (body, values) = parse_local_variables(
            "@base_url = https://example.com # service URL\n\nGET {{base_url}} HTTP/1.1\n",
        )
        .unwrap();

        assert_eq!(values["base_url"].as_str(), Some("https://example.com"));
        assert_eq!(body, "GET {{base_url}} HTTP/1.1\n");
    }

    #[test]
    fn parses_local_variables_with_interspersed_comment_lines() {
        let input = "@id=old\n# @id=commented\n@id=new\n\nGET /{{id}} HTTP/1.1\n";
        let (body, values) = parse_local_variables(input).unwrap();

        assert_eq!(values["id"].as_str(), Some("new"));
        assert_eq!(body, "GET /{{id}} HTTP/1.1\n");
    }

    #[test]
    fn parses_declarations_after_leading_comments_and_blank_lines() {
        let input = "# request settings\r\n\r\n@id = 42\r\nGET /{{id}} HTTP/1.1\r\n";
        let (body, values) = parse_local_variables(input).unwrap();

        assert_eq!(body, "GET /{{id}} HTTP/1.1\r\n");
        assert_eq!(values["id"].as_str(), Some("42"));
    }

    #[test]
    fn leaves_request_unchanged_without_leading_declarations() {
        let input = "GET https://example.com HTTP/1.1\n@not_a_declaration = value\n";
        let (body, values) = parse_local_variables(input).unwrap();

        assert_eq!(body, input);
        assert!(values.is_empty());
    }

    #[test]
    fn rejects_malformed_leading_declaration() {
        assert!(parse_local_variables("@missing_equals\nGET / HTTP/1.1\n").is_err());
    }
}
