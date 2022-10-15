use crate::{EnvConf, ServerContext};
use anyhow::Context;
use file_owner::{group, owner, set_owner_group, Group, Owner};
use simplelog::trace;
use std::borrow::Borrow;
use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::fs::{read, set_permissions};
use std::iter::Map;
use std::mem::take;
use std::ops::Index;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use handlebars::Handlebars;
use walkdir::WalkDir;

pub struct PermissionManager {
    owner: Owner,
    group: Group,
}

pub struct File {
    source_bytes: HashMap<ServerContext, Vec<u8>>,
    existing_bytes: Option<Vec<u8>>,

    pub relative: PathBuf,
    pub template: bool,
}

pub struct FileSystem {
    pub source_root: PathBuf,
    pub destination_root: PathBuf,

    context_files: BTreeMap<String, Vec<File>>,
}

impl PermissionManager {
    pub fn new(conf: &EnvConf) -> anyhow::Result<Self> {
        let owner = conf
            .get_env("UID")
            .and_then(|s| s.parse::<u32>().ok())
            .map(|uid| Owner::from_uid(uid))
            .or(conf
                .get_env("USER")
                .map(|s| Owner::from_name(s.as_str()).ok())
                .flatten())
            .context("Get owner")?;

        let group = conf
            .get_env("GID")
            .and_then(|s| s.parse::<u32>().ok())
            .map(|gid| Group::from_gid(gid))
            .or(conf
                .get_env("GROUP")
                .map(|s| Group::from_name(s.as_str()).ok())
                .flatten())
            .context("Get group")?;

        Ok(Self { owner, group })
    }

    pub fn set_permissions(&self, path: &Path) -> anyhow::Result<()> {
        if !path.exists() {
            return Err(anyhow::anyhow!("Path {} does not exist", path.display()));
        }

        if path.is_symlink() {
            trace!("Path {} is a symlink, skipping permissions", path.display());
            return Ok(());
        }

        let permission = if path.is_dir() {
            PermissionsExt::from_mode(0o755)
        } else {
            PermissionsExt::from_mode(0o644)
        };

        set_permissions(path, permission).context("Set permissions for path")?;
        set_owner_group(path, self.owner, self.group).context("Set owner and group")
    }
}

impl File {
    pub fn new<'a>(relative: PathBuf, conf: &'_ EnvConf) -> Self {
        let destination_path = conf.destination_root.join(&relative);
        let existing_bytes = read(&destination_path).ok();
        let mut source_bytes: HashMap<ServerContext, _> = HashMap::new();
        conf.contexts
            .iter()
            .map(|ctx| (ctx, read(ctx.source_root.join(&relative)).ok()))
            .filter(|(_, bytes)| bytes.is_some())
            .for_each(|(ctx, bytes)| {
                source_bytes.insert(ctx.clone(), bytes.unwrap());
            });

        Self {
            source_bytes,
            existing_bytes,
            relative,
            template: false,
        }
    }

    pub fn get_string(&self, context: &str) -> Option<String> {
        None
    }

    pub fn ensure_dirs(&self, )
}

impl FileSystem {
    pub fn new<'a>(conf: &'_ EnvConf) -> anyhow::Result<Self> {
        let source_root = PathBuf::from(conf.get_env("SERVER_SYNC_REPO").context("Get repo location")?).join("contexts");

        let mut context_files: BTreeMap<String, Vec<File>> = BTreeMap::new();
        for context in conf.contexts.iter() {
            let walker = WalkDir::new(&context.source_root)
                .same_file_system(true)
                .into_iter()
                .filter(|e| e.is_ok())
                .map(|e| e.unwrap());

            let mut files: Vec<File> = Vec::new();
            for entry in walker {
                let path = entry.path();
                if path.is_dir() || path.is_symlink() {
                    continue;
                }

                let file = File::new(
                    path.strip_prefix(&context.source_root)
                        .unwrap()
                        .to_path_buf(),
                    conf,
                );

                files.push(file);
            }

            context_files.insert(context.name.clone(), files);
        }

        Ok(Self {
            source_root,
            destination_root: conf.destination_root.clone(),
            context_files,
        })
    }

    pub fn get_context_files(&self, context: &str) -> Option<&Vec<File>> {
        return self.context_files.get(context);
    }

    pub fn sync(&self, handlebars: &mut Handlebars) -> anyhow::Result<()> {
        for (context_name, files) in self.context_files.iter() {
            for file in files {
                if file.template {

                }

                file.ensure_dirs()?;

                let destination_path = self.destination_root.join(&file.relative);
                let source_path = self.source_root.join(context_name).join(&file.relative);
                let source_bytes = file.source_bytes.get(context_name).unwrap();
                let existing_bytes = file.existing_bytes.as_ref();

                if source_bytes == existing_bytes {
                    trace!("File {} is up to date", destination_path.display());
                    continue;
                }

                if file.template {
                    let template = String::from_utf8_lossy(source_bytes);
                    let rendered = handlebars.render_template(&template, &context_name)?;
                    let rendered_bytes = rendered.as_bytes();
                    std::fs::write(&destination_path, rendered_bytes)?;
                } else {
                    std::fs::copy(&source_path, &destination_path)?;
                }
            }
        }

        Ok(())
    }
}
