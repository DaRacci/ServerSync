use crate::merger::Mergable;
use crate::{EnvConf, ServerContext};
use anyhow::{anyhow, Context};
use file_owner::{group, owner, set_owner_group, Group, Owner};
use handlebars::Handlebars;
use regex::{Regex, RegexBuilder};
use simplelog::trace;
use std::borrow::Borrow;
use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::fmt::format;
use std::fs::{copy, create_dir, read, remove_file, rename, set_permissions, write};
use std::io::{Read, Write};
use std::iter::Map;
use std::mem::take;
use std::ops::Index;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub struct PermissionManager {
    owner: Owner,
    group: Group,
}

pub struct File {
    source_bytes: Vec<u8>,
    existing_bytes: Option<Vec<u8>>,
    utf8_parsed: Option<String>,

    pub source: PathBuf,
    pub destination: PathBuf,
}

pub struct FileSystem {
    pub source_root: PathBuf,
    pub destination_root: PathBuf,

    // handlebars: Handlebars<'_>,
    permission_manager: PermissionManager,
    context_files: HashMap<ServerContext, Vec<File>>,
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
    pub fn new<'a>(
        context: &ServerContext,
        source_path: PathBuf,
        conf: &'_ EnvConf,
    ) -> anyhow::Result<Self> {
        let destination_path = conf.destination_root.join(
            source_path
                .strip_prefix(&context.context_root)?
                .to_path_buf(),
        );
        let existing_bytes = read(&destination_path).ok();
        let source_bytes = read(&source_path).context("Read source file")?;
        let utf8_parsed = simdutf8::basic::from_utf8(&source_bytes)
            .map(|s| s.to_string())
            .ok();

        Ok(Self {
            source_bytes,
            existing_bytes,
            utf8_parsed,
            source: source_path,
            destination: destination_path,
        })
    }
}

impl FileSystem {
    pub fn new(conf: &EnvConf) -> Result<FileSystem, anyhow::Error> {
        let source_root = PathBuf::from(
            conf.get_env("SERVER_SYNC_REPO")
                .context("Get repo location").unwrap(),
        )
        .join("contexts");

        let mut context_files: HashMap<_, _> = HashMap::new();
        for context in conf.contexts.iter() {
            let walker = WalkDir::new(&context.context_root)
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

                let file = File::new(context, path.to_path_buf(), conf).unwrap();

                files.push(file);
            }

            context_files.insert(context.clone(), files);
        }

        Ok(Self {
            source_root,
            destination_root: conf.destination_root.clone(),
            // handlebars,
            permission_manager: PermissionManager::new(&conf).unwrap(),
            context_files,
        })
    }

    pub fn sync(&self, handlebars: &mut Handlebars) -> anyhow::Result<()> {
        // for (context, files) in self.context_files.iter() {
        //     for file in files {
        //         let existing_bytes = file.existing_bytes.as_ref();
        //
        //         self.ensure_dirs(&file.destination)?;
        //         self.backup(&file.destination)?;
        //
        //         return if let Some(utf8) = &file.utf8_parsed {
        //             trace!("File {:?} is utf8.", file.destination.file_name());
        //
        //             self.render_utf8(context, file)
        //         } else {
        //             trace!("File {:?} isn't utf8.", file.destination.file_name());
        //
        //             self.copy_bytes(&file.source, &file.destination)
        //         }
        //     }
        // }

        Ok(())
    }

    // TODO -> Support Rsync, BTRFS snapshots and other methods
    pub fn backup(&self, file: &Path) -> anyhow::Result<()> {
        if !file.exists() {
            return Err(anyhow::anyhow!(
                "Cannot backup non-existent {}",
                file.display()
            ));
        }

        let backup_path = file.with_extension("bak");

        if backup_path.exists() {
            remove_file(&backup_path).context("Delete old backup file")?;
        }

        rename(file, &backup_path).context("Rename file")
    }

    pub fn ensure_dirs(&self, path: &Path) -> anyhow::Result<()> {
        let ancestors = path.ancestors();

        for ancestor in ancestors.into_iter() {
            if !ancestor.starts_with(&self.destination_root) {
                continue;
            }

            if ancestor.exists() {
                continue; // Lets not touch existing directories it only causes issues.
            }

            create_dir(ancestor).context(format!("Create ancestor / directory of {}", ""))?;

            self.permission_manager.set_permissions(ancestor)?;
        }

        Ok(())
    }

    fn copy_bytes(&self, source: &Path, destination: &Path) -> anyhow::Result<()> {
        let mut source_file = std::fs::File::open(source).context("Open source file")?;
        let mut destination_file =
            std::fs::File::create(destination).context("Create destination file")?;

        let mut buffer = [0; 1024];
        loop {
            let bytes_read = source_file.read(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }

            destination_file.write_all(&buffer[..bytes_read])?;
        }

        Ok(())
    }

    // fn render_utf8<T>(&self, context: &ServerContext, file: &File) -> anyhow::Result<T> {
    //     let extension = file.source.extension().context("Get file extension")?;
    //     let template = self.handlebars.render(file.source.to_str().context("Convert source path to str")?, &file.utf8_parsed.unwrap())?;
    //
    //     Ok(())
    // }

    // fn first_compatible<T>(&self, rendered: &String) -> anyhow::Result<T> where T : Mergable {
    //     let definition = REGEX.find(rendered);
    //     definition.map(|definition| {
    //         let definition = definition.as_str();
    //         let definition = definition.trim_start_matches("{{");
    //         let definition = definition.trim_end_matches("}}");
    //         let definition = definition.trim_start_matches("!");
    //         let definition = definition.trim_start_matches(" ");
    //         let definition = definition.trim_end_matches(" ");
    //
    //         let definition = serde_yaml::from_str::<T>(definition).context("Parse definition")?;
    //
    //         Ok(definition)
    //     }).unwrap_or_else(|| {
    //         Err(anyhow::anyhow!("No compatible definition found."))
    //     })

    // Mergable::merge(existing, inserting).context("Attempt to merge")
}

struct FileDefinition {
    file_type: String,
}

// static REGEX: Regex = RegexBuilder::new("!!!<type>(.*?)</type>!!!")
//     .case_insensitive(true)
//     .build()
//     .unwrap();
