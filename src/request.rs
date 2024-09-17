use anyhow::{anyhow, bail, Context, Result};
use graphql_parser::query::{
    Definition, OperationDefinition, VariableDefinition,
};

use log::{info, log_enabled, warn, Level};
use reqwest::{header::HeaderMap, Client, Method, Response, Url};
use serde_json::{json, Value};
use spinoff::{spinners, Color, Spinner, Streams};
use std::{
    fmt::Display,
    path::{Path, PathBuf},
    str::{self},
    sync::Arc,
    time::Duration,
};
use toml::Table;

use crate::{
    env::{update_data, HitmanCookieJar},
    extract::extract_variables,
    prompt::{get_interaction, prepare_request_interactive},
    util::truncate,
};

#[derive(Clone)]
pub enum HitmanBody {
    REST {
        body: String,
    },
    GraphQL {
        operation: GraphQLOperation,
        body: String,
        variables: Option<String>,
    },
}

impl Display for HitmanBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HitmanBody::REST { body } => write!(f, "{}", body),
            HitmanBody::GraphQL {
                operation: _,
                body,
                variables,
            } => match variables {
                Some(v) => write!(f, "{}\n{}", body, v),
                None => write!(f, "{}", body),
            },
        }
    }
}

impl HitmanBody {
    pub fn to_body(self) -> String {
        match self {
            HitmanBody::REST { body } => body,
            HitmanBody::GraphQL {
                operation,
                body,
                variables,
            } => match variables {
                Some(v) => json!({operation.to_string(): body, "variables": v})
                    .to_string(),
                None => json!({operation.to_string(): body}).to_string(),
            },
        }
    }
}

#[derive(Clone)]
pub struct HitmanRequest {
    pub headers: HeaderMap,
    pub url: Url,
    pub method: Method,
    pub body: Option<HitmanBody>,
}

impl Display for HitmanRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{} {}", self.method.as_str(), self.url.as_str())?;
        for (key, val) in self.headers.iter() {
            writeln!(
                f,
                "{}: {}",
                key.as_str(),
                val.to_str().unwrap_or("unknown value")
            )?;
        }

        if let Some(ref body) = self.body {
            writeln!(f)?;
            for line in body.to_string().lines() {
                writeln!(f, "{}", line)?;
            }
        }
        write!(f, "")
    }
}

static USER_AGENT: &str =
    concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"),);

pub fn build_client() -> Result<Client> {
    let client = Client::builder()
        .user_agent(USER_AGENT)
        .cookie_provider(Arc::new(HitmanCookieJar))
        .build()?;
    Ok(client)
}

pub async fn make_request(file_path: &Path, env: &Table) -> Result<()> {
    let client = build_client()?;

    let interaction = get_interaction();

    let req =
        prepare_request_interactive(file_path, env, interaction.as_ref())?;

    clear_screen();
    print_request(&req);

    let mut spinner = Spinner::new_with_stream(
        spinners::BouncingBar,
        "",
        Color::Yellow,
        Streams::Stderr,
    );
    let (response, elapsed) = do_request(&client, &req).await?;
    spinner.stop();

    print_response(&response)?;

    if let Ok(json) = response.json::<Value>().await {
        println!("{}", serde_json::to_string_pretty(&json)?);
        let vars = extract_variables(&json, env)?;
        update_data(&vars)?;
    }

    warn!("# Request completed in {:.2?}", elapsed);

    Ok(())
}

fn clear_screen() {
    if cfg!(windows) {
        std::process::Command::new("cmd")
            .args(["/c", "cls"])
            .spawn()
            .expect("cls command failed to start")
            .wait()
            .expect("failed to wait");
    } else {
        // Untested!
        println!("\x1B[2J");
    }
}

pub async fn do_request(
    client: &Client,
    req: &HitmanRequest,
) -> Result<(Response, Duration)> {
    let mut builder = client.request(req.method.clone(), req.url.clone());
    builder = builder.headers(req.headers.clone());
    if let Some(ref body) = req.body {
        builder = builder.body(body.clone().to_body());
    }

    let t = std::time::Instant::now();
    let response = builder.send().await?;

    let elapsed = t.elapsed();

    Ok((response, elapsed))
}

fn print_request(req: &HitmanRequest) {
    if log_enabled!(Level::Info) {
        for line in req.to_string().lines() {
            info!("> {}", truncate(line));
        }
    }
}

fn print_response(res: &Response) -> Result<()> {
    if log_enabled!(Level::Info) {
        let status = res.status();
        info!(
            "< HTTP/1.1 {} {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("")
        );

        let mut head = String::new();
        for (name, value) in res.headers() {
            head.push_str(&format!("{}: {}\n", name, value.to_str()?));
        }

        for line in head.lines() {
            info!("< {}", truncate(line));
        }

        info!("");
    }

    Ok(())
}

// TODO: This is very similar to `find_root_dir`
pub fn resolve_http_file(path: &Path) -> Result<PathBuf> {
    let ext = path.extension().context("Couldn't find extension")?;
    if ext != "gql" && ext != "graphql" {
        return Ok(path.to_path_buf());
    };

    // &resolve_http_file(file_path)?.context("Couldn't find _graphql.http")?
    let mut dir = path.parent().context("No parent")?.to_path_buf();
    loop {
        let file = dir.join("_graphql.http");
        if file.exists() {
            return Ok(file);
        }
        if let Some(parent) = dir.parent() {
            dir = parent.to_path_buf();
        } else {
            return Err(anyhow!("Couldn't find _graphql.http"));
        }
    }
}

#[derive(Clone)]
pub enum GraphQLOperation {
    Query,
    Mutation,
}

impl Display for GraphQLOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphQLOperation::Query => write!(f, "query"),
            GraphQLOperation::Mutation => write!(f, "mutation"),
        }
    }
}

pub struct GraphQLRequest {
    pub operation: GraphQLOperation,
    pub args: Vec<String>,
}

pub fn find_args<P>(path: P) -> Result<GraphQLRequest>
where
    P: AsRef<Path>,
{
    let file = std::fs::read_to_string(path)?;
    let doc = graphql_parser::parse_query::<String>(&file)?;

    let variables = |vars: &[VariableDefinition<String>]| {
        vars.iter().map(|d| d.name.clone()).collect::<Vec<_>>()
    };

    let args = match doc.definitions[0] {
        Definition::Operation(ref op) => match op {
            OperationDefinition::Query(q) => GraphQLRequest {
                operation: GraphQLOperation::Query,
                args: variables(&q.variable_definitions),
            },
            OperationDefinition::Mutation(m) => GraphQLRequest {
                operation: GraphQLOperation::Mutation,
                args: variables(&m.variable_definitions),
            },
            OperationDefinition::SelectionSet(_) => bail!("Not supported"),
            OperationDefinition::Subscription(_) => bail!("Not supported"),
        },
        Definition::Fragment(_) => bail!("Not supported"),
    };

    Ok(args)
}
