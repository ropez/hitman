use std::{
    env::current_dir,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};

const CONFIG_FILE: &str = "hitman.toml";

#[derive(Debug, Clone)]
pub enum ResolvedAs {
    Simple {
        path: Box<Path>,
    },
    GraphQL {
        wrapper_path: Box<Path>,
        graphql_path: Box<Path>,
    },
}

pub struct Resolved {
    pub root_dir: Box<Path>,
    pub resolved_as: ResolvedAs,
}

impl Resolved {
    pub fn original_path(&self) -> &Path {
        match &self.resolved_as {
            ResolvedAs::Simple { path } => path,
            ResolvedAs::GraphQL { graphql_path, .. } => graphql_path,
        }
    }

    pub fn toml_path(&self) -> PathBuf {
        let orig = self.original_path();
        match orig.extension() {
            Some(ext) => orig.with_extension(with_suffix(ext, ".toml")),
            None => orig.with_extension("toml"),
        }
    }

    pub fn http_file(&self) -> &Path {
        match &self.resolved_as {
            ResolvedAs::Simple { path } => path,
            ResolvedAs::GraphQL { wrapper_path, .. } => wrapper_path,
        }
    }
}

pub fn resolve_path(path: &Path) -> Result<Resolved> {
    let root_dir = find_root_dir(path)?.unwrap_or(current_dir()?.into());

    let resolved_as = if is_graphql(path) {
        let template_path = resolve_graphql_http_file(path)?;
        ResolvedAs::GraphQL {
            wrapper_path: template_path.into(),
            graphql_path: path.into(),
        }
    } else {
        ResolvedAs::Simple { path: path.into() }
    };

    Ok(Resolved {
        root_dir,
        resolved_as,
    })
}

pub fn is_graphql(path: &Path) -> bool {
    match path.extension().map(|e| e.to_ascii_lowercase()) {
        Some(ext) => ext == "gql" || ext == "graphql",
        None => false,
    }
}

// The root dir is where we find hitman.toml,
// scanning parent directories until we find it
pub fn find_root_dir(path: &Path) -> Result<Option<Box<Path>>> {
    let mut dir: Box<Path> = path.into();
    let res = loop {
        if dir.join(CONFIG_FILE).exists() {
            break Some(dir);
        }
        if let Some(parent) = dir.parent() {
            dir = parent.into();
        } else {
            break None;
        }
    };

    Ok(res)
}

// FIXME: This is very similar to `find_root_dir`
pub fn resolve_graphql_http_file(path: &Path) -> Result<PathBuf> {
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

fn with_suffix(s: &OsStr, suffix: &str) -> OsString {
    let mut s = s.to_owned();
    s.push(suffix);
    s
}
