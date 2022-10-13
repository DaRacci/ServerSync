use envfile::EnvFile;
use std::borrow::Borrow;

use anyhow::{format_err, Context};
use clap::builder::TypedValueParser;
use clap::ArgMatches;
use simplelog::{debug, trace, warn};
use std::collections::BTreeMap;
use std::fmt::{Debug, Display, Formatter};
use std::io::BufRead;
use std::path::PathBuf;

pub struct ServerContext {
    pub name: String,
    pub source_root: PathBuf,
}

impl ServerContext {
    pub fn new(name: String, repo_path: &str) -> anyhow::Result<Self> {
        let source_root = PathBuf::from(repo_path).join("contexts/").join(&name);

        Ok(Self { name, source_root })
    }
}

impl Debug for ServerContext {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

pub struct EnvConf {
    file: Option<EnvFile>,

    matches: ArgMatches,

    pub contexts: Vec<ServerContext>,

    pub destination_root: PathBuf,
}

impl EnvConf {
    pub fn new(matches: ArgMatches) -> anyhow::Result<Self> {
        let file = EnvFile::new(matches.get_one::<String>("SERVER_SYNC_ENV").unwrap()).ok();
        let raw_destination = _get_env("SERVER_SYNC_DESTINATION", &matches, &file)
            .context("Get destination for sync")?;

        let repo_path =
            _get_env("SERVER_SYNC_REPO_STORAGE", &matches, &file).context("Get repository path")?;

        let contexts = matches
            .get_many::<String>("SERVER_SYNC_CONTEXTS")
            .map(|v| v.map(|s| s.to_string()).collect::<Vec<_>>())
            .or(file.as_ref().map(|f| {
                f.get("SERVER_SYNC_CONTEXTS")
                    .map(|s| s.to_string())
                    .or(std::env::var("SERVER_SYNC_CONTEXTS").ok())
                    .map(|s| s.split(',').map(|s| s.to_string()).collect::<Vec<_>>())
                    .unwrap_or_default()
            }))
            .map(|v| {
                v.into_iter()
                    .map(|s| ServerContext::new(s, &repo_path).unwrap())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        debug!("Contexts: {:?}", contexts);
        debug!("Destination: {}", raw_destination);

        let destination_root = PathBuf::from(raw_destination);

        if contexts.is_empty() {
            return Err(format_err!("No contexts to sync!"));
        }

        Ok(Self {
            file,
            matches,
            contexts,
            destination_root,
        })
    }

    pub fn get_env(&self, env: &str) -> Option<String> {
        return _get_env(env, &self.matches, &self.file);
    }

    pub fn get_variables(&self) -> BTreeMap<String, String> {
        let mut mut_map = if self.file.is_none() {
            BTreeMap::new()
        } else {
            self.file.as_ref().unwrap().store.clone()
        };

        std::env::vars().for_each(|(k, v)| {
            mut_map.insert(k, v);
        });

        return mut_map;
    }

    pub fn get_contexts(&self) -> &[ServerContext] {
        self.contexts.borrow()
    }
}

fn _get_env(env: &str, matches: &ArgMatches, file: &Option<EnvFile>) -> Option<String> {
    if let Ok(env) = matches.try_get_one::<String>(env) {
        if let Some(env) = env {
            trace!("Found {} in command args", env);
            return Some(env.to_string());
        }
    }

    if let Some(envfile) = file {
        if let Some(env) = envfile.get(env) {
            trace!("Found {} in env file", env);
            return Some(env.to_string());
        }
    }

    if let Ok(env) = std::env::var(env) {
        trace!("Found {} in process env", env);
        return Some(env);
    }

    if let Some(env) = std::env::var_os(env) {
        trace!("Found {} in system env", env.to_string_lossy());
        return Some(env.to_string_lossy().to_string());
    }

    trace!("Couldn't find {} in any env", env);
    None
}
