use envfile::EnvFile;
use std::borrow::Borrow;

use anyhow::{format_err, Context};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub struct ServerContext {
    pub name: String,
    pub source_root: PathBuf,
}

impl ServerContext {
    pub fn new(name: String) -> anyhow::Result<Self> {
        let source_root = std::env::current_dir()
            .unwrap()
            .join("contexts/")
            .join(&name);

        if !source_root.exists() || !source_root.is_dir() {
            return Err(format_err!(
                "Server source root doesn't exist or is not a directory: {}",
                source_root.display()
            ));
        }

        Ok(Self { name, source_root })
    }
}

pub struct EnvConf {
    file: Option<EnvFile>,

    pub contexts: Vec<ServerContext>,

    pub destination_root: PathBuf,
}

impl EnvConf {
    pub fn new() -> anyhow::Result<Self> {
        let file = EnvFile::new(".env").ok();
        let contexts = std::env::var("CONTEXTS")
            .ok()
            .map(|s| s.split(";").map(|s| s.to_string()).collect::<Vec<String>>())
            .or(file
                .as_ref()
                .map(|f| f.get("CONTEXTS"))
                .flatten()
                .map(|s| s.split(";").map(|s| s.to_string()).collect::<Vec<String>>()))
            .map(|v| {
                v.iter()
                    .map(|s| ServerContext::new(s.to_owned()))
                    .collect::<anyhow::Result<Vec<ServerContext>>>()
                    .ok()
            })
            .flatten()
            .unwrap_or(vec![]);

        let destination_root = std::env::var("DESTINATION")
            .ok()
            .map(|s| PathBuf::from(s))
            .or(file
                .as_ref()
                .map(|f| f.get("DESTINATION").map(|s| PathBuf::from(s)))
                .flatten())
            .context("Destination root not set")
            .unwrap();

        Ok(Self {
            file,
            contexts,
            destination_root,
        })
    }

    pub fn get_env(&self, env: &str) -> Option<&String> {
        match &self.file {
            Some(envfile) => envfile.store.get(env),
            None => None,
        }
    }

    pub fn get_variables(&self) -> BTreeMap<String, String> {
        if self.file.is_none() {
            return BTreeMap::new();
        }

        return self.file.as_ref().unwrap().store.clone();
    }

    pub fn get_contexts(&self) -> &[ServerContext] {
        self.contexts.borrow()
    }
}
