use std::path::{Path, PathBuf};

use anyhow::{bail, Context};

pub enum Resolved {
    Simple {
        path: Box<Path>,
    },
    GraphQL {
        wrapper_path: Box<Path>,
        graphql_path: Box<Path>,
    },
}

impl Resolved {
    pub fn http_file(&self) -> &Path {
        match self {
            Self::Simple { path } => path,
            Self::GraphQL { wrapper_path: template_path, .. } => template_path,
        }
    }
}

pub fn resolve_path(path: &Path) -> anyhow::Result<Resolved> {
    let ext = path
        .extension()
        .context("Couldn't get ext")?
        .to_ascii_lowercase();

    Ok(if ext == "gql" || ext == "graphql" {
        let template_path = resolve_graphql_http_file(path)?;
        Resolved::GraphQL {
            wrapper_path: template_path.into(),
            graphql_path: path.into(),
        }
    } else {
        Resolved::Simple { path: path.into() }
    })
}

// FIXME: This is very similar to `find_root_dir`
pub fn resolve_graphql_http_file(path: &Path) -> anyhow::Result<PathBuf> {
    let mut dir = path.parent().context("No parent")?.to_path_buf();
    loop {
        let file = dir.join("_graphql.http");
        if file.exists() {
            return Ok(file);
        }
        if let Some(parent) = dir.parent() {
            dir = parent.to_path_buf();
        } else {
            bail!("Couldn't find _graphql.http");
        }
    }
}

