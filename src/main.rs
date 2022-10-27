mod config;
mod file_system;
mod merger;
mod merge_toml;

use crate::config::{EnvConf, ServerContext};
use crate::file_system::FileSystem;
use anyhow::{format_err, Context};
use clap::{command, Arg, ArgAction, ArgMatches};
use file_owner::{group, owner};
use handlebars::Handlebars;
use merge_yaml_hash::MergeYamlHash;
use regex::internal::Input;
use similar::{ChangeTag, DiffableStr, TextDiff};
use simplelog::__private::log::{logger, SetLoggerError};
use simplelog::{
    debug, error, info, trace, Color, ColorChoice, Config, ConfigBuilder, LevelFilter, TermLogger,
    TerminalMode,
};
use std::borrow::Borrow;
use std::cmp::Ordering;
use std::collections::hash_map::DefaultHasher;
use std::collections::BTreeMap;
use std::env::{current_dir, vars_os};
use std::error::Error;
use std::fs::{create_dir, create_dir_all, read, rename, set_permissions, File, Permissions};
use std::hash::Hash;
use std::hash::Hasher;
use std::io::{ErrorKind, Read, Write};
use std::ops::Deref;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{exit, Command};
use std::ptr::hash;
use std::{env, fs};
use walkdir::{DirEntry, WalkDir};

fn this() {
    let baseline = r#"
a:
  b:
    c: lmao
    "#;

    let insertion = r#"
    a:
      b:
        c: rofl
        d: r
        e:
          l: one
    "#;

    let mut hash = MergeYamlHash::new();

    // Merge YAML data from strings
    hash.merge(baseline);
    hash.merge(insertion);

    let new_yaml = hash.to_string();
    let diff = TextDiff::from_lines(baseline, new_yaml.as_str());
    for change in diff.iter_all_changes() {
        let sign = match change.tag() {
            ChangeTag::Delete => "<red>-",
            ChangeTag::Insert => "<green>+",
            ChangeTag::Equal => continue,
        };

        let raw = change.to_string();
        for char in raw.chars() {
            print!("{}", char.escape_unicode());
        }
        info!("{} {}", sign, change.to_string().trim());
    }

    println!("{}", new_yaml);

    ()
}

fn main() {
    let cli = get_cli();
    start_logger(&cli).context("Init logger").unwrap();
    this();
    return;

    let conf = match EnvConf::new(cli) {
        Ok(value) => value,
        Err(err) => {
            error!("Failed to init config -> {}", err);
            exit(19)
        }
    };

    let file_system = FileSystem::new(conf.borrow::<'static>()).ok().unwrap();

    match run(conf, file_system) {
        Ok(_) => {
            info!("Done!");
            exit(0)
        }
        Err(err) => {
            error!("{}", err);
            exit(1);
        }
    }
}

fn get_cli() -> ArgMatches {
    command!()
        .propagate_version(true)
        .args([
            Arg::new("VERBOSE").short('v').action(ArgAction::Count),
            Arg::new("SERVER_SYNC_ENV")
                .short('e')
                .long("env-file")
                .env("SERVER_SYNC_ENV")
                .help("The env file to load.")
                .default_value(".server_env"),
            Arg::new("SERVER_SYNC_REPO")
                .short('r')
                .long("repo")
                .env("SERVER_SYNC_REPO")
                .help("The repository to sync from."),
            Arg::new("SERVER_SYNC_BRANCH")
                .short('b')
                .long("branch")
                .env("SERVER_SYNC_BRANCH")
                .help("The branch to sync from."),
            Arg::new("SERVER_SYNC_DESTINATION")
                .short('d')
                .long("dest")
                .help("The destination to sync to.")
                .env("SERVER_SYNC_DESTINATION"),
            Arg::new("SERVER_SYNC_CONTEXTS")
                .short('c')
                .long("contexts")
                .help("The server contexts to use.")
                .action(ArgAction::Append),
            Arg::new("SERVER_SYNC_REPO_STORAGE")
                .long("repo-storage")
                .env("SERVER_SYNC_REPO_STORAGE")
                .help("The storage path for the repository.")
                .default_value("/tmp/server-sync/"),
        ])
        .get_matches()
}

fn start_logger(matches: &ArgMatches) -> anyhow::Result<()> {
    let level = matches.get_count("VERBOSE");
    let level = match level {
        2 => LevelFilter::Trace,
        1 => LevelFilter::Debug,
        0 => LevelFilter::Info,
        _ => LevelFilter::Trace,
    };

    TermLogger::init(
        level,
        ConfigBuilder::new()
            .set_time_level(LevelFilter::Off)
            .build(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    )?;

    info!("Logger started at level {}", level);

    Ok(())
}

fn run(conf: EnvConf, file_system: FileSystem) -> anyhow::Result<()> {
    let repo_str = conf
        .get_env("SERVER_SYNC_REPO_STORAGE")
        .context("Get repo storage location")?;
    let repo_dir = Path::new(&repo_str);
    sync_repository(&conf, &repo_dir).context("Sync repo")?;

    let mut handlebars = new_handlerbars().context("Initialize handlebars")?;

    debug!("Variables: {:?}", &conf.get_variables());

    file_system.sync(&mut handlebars)?;

    for context in conf.get_contexts() {
        if !context.context_root.exists() || !context.context_root.is_dir() {
            return return Err(format_err!(
                "Server source root doesn't exist or is not a directory: {}",
                context.context_root.display()
            ));
        }

        info!("Processing context {}", context.name);
        debug!("Source root: {}", context.context_root.display());

        walk_directory(&mut handlebars, &context, &conf)?;
    }

    Ok(())
}

fn git_output(cmd: &mut Command, context: String) -> anyhow::Result<()> {
    let output = cmd.output().context(context)?;
    trace!(
        "Git output -> <blue>{}",
        String::from_utf8_lossy(&output.stdout).trim()
    );

    Ok(())
}

fn sync_repository(conf: &EnvConf, repo_dir: &Path) -> anyhow::Result<()> {
    let repo_url = conf.get_env("SERVER_SYNC_REPO").unwrap();
    let repo_branch = conf
        .get_env("SERVER_SYNC_BRANCH")
        .unwrap_or("master".to_string());

    if !repo_dir.exists() {
        info!("Cloning repository {}", &repo_url);

        let mut cmd = Command::new("git");
        cmd.arg("clone").arg(&repo_url).arg(&repo_dir);
        git_output(&mut cmd, "Clone repository".to_string())?;
    } else {
        info!("Updating repository {}", &repo_url);

        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(&repo_dir).arg("pull");
        git_output(&mut cmd, "Update repository".to_string())?;
    }

    info!("Checking out branch {}", &repo_branch);

    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(&repo_dir)
        .arg("checkout")
        .arg(&repo_branch);

    git_output(&mut cmd, "Checkout branch".to_string())?;

    Ok(())
}

fn walk_directory(
    handlebars: &mut Handlebars,
    context: &ServerContext,
    conf: &EnvConf,
) -> anyhow::Result<()> {
    let walker = WalkDir::new(&context.context_root)
        .same_file_system(true)
        .into_iter()
        .filter(|e| e.is_ok())
        .filter(|e| e.as_ref().unwrap().file_type().is_file())
        .map(|e| e.unwrap());

    let mut non_utf8 = vec![];

    for entry in walker {
        let relative_path = entry
            .path()
            .strip_prefix(&context.context_root)
            .context("Get relative path")?;

        let destination_path = conf.destination_root.join(&relative_path);
        let parent = &destination_path.parent().expect("File was at / level???");

        let ancestors_dirs = parent
            .ancestors()
            .filter(|a| a.starts_with(&conf.destination_root));

        for ancestor in ancestors_dirs {
            if !ancestor.exists() {
                trace!("Creating directory {}", ancestor.display());
                create_dir(ancestor).context("Create ancestor directory")?;
            }

            fix_permissions(&ancestor, &conf)?;
        }

        let contents = match get_contents(entry.path()) {
            None => {
                non_utf8.push((entry.path().to_owned(), destination_path));
                continue;
            }
            Some(value) => value,
        };

        trace!("Processing file {}", relative_path.display());

        let rendered = render_entry(handlebars, &context, &conf, &contents, &entry)
            .context("Render source")?;

        trace!(
            "Templating {} to {}",
            &entry.path().display(),
            &destination_path.display()
        );

        if check_existing(&destination_path, &rendered)? {
            debug!("File {} is up to date", destination_path.display());
        } else {
            trace!("Writing {}", destination_path.display());
            let mut file = File::create(&destination_path)?;
            file.write_all(rendered.as_bytes())?;
        }

        fix_permissions(&destination_path, &conf)?;
    }

    // TODO -> This is a bit of a hack, but it works for now.
    for (source, dest) in non_utf8 {
        trace!(
            "Processing non-utf8 file {} to destination {}",
            source.display(),
            dest.display()
        );

        let buf = read(source).context("Read source file")?;
        if let Ok(existing) = read(&dest).context("Read existing file") {
            if buf == existing {
                continue;
            }

            let backup_path = Path::new(&dest).with_extension("bak");
            rename(&dest, &backup_path).context("Rename old file")?;
        }

        let mut file = File::create(&dest).context("Create new file")?;
        file.write_all(&buf)?;

        fix_permissions(&dest, &conf).context("Ensure file has correct permissions")?;
    }

    Ok(())
}

fn get_contents<P: AsRef<Path>>(path: P) -> Option<String> {
    let mut source = vec![];
    File::open(path).unwrap().read_to_end(&mut source).unwrap();
    return match simdutf8::basic::from_utf8(&source) {
        Ok(contents) => Some(contents.to_string()),
        Err(_) => None,
    };
}

fn render_entry(
    handlebars: &mut Handlebars,
    context: &ServerContext,
    conf: &EnvConf,
    contents: &String,
    entry: &DirEntry,
) -> anyhow::Result<String> {
    let mut variables_cloned = conf.get_variables().clone();
    variables_cloned.insert(String::from("server_name"), context.name.to_owned());

    handlebars.register_template_string(&entry.file_name().to_string_lossy(), &contents)?;

    return handlebars
        .render(&entry.file_name().to_string_lossy(), &variables_cloned)
        .ok()
        .context("Rendering template");
}

fn check_existing(destination: &Path, rendered: &String) -> anyhow::Result<bool> {
    if !destination.exists() {
        return Ok(false);
    }

    let existing_contents = match get_contents(&destination) {
        None => return Ok(false),
        Some(value) => value,
    };

    let diff = TextDiff::from_lines(&existing_contents, &rendered);
    for change in diff.iter_all_changes() {
        let sign = match change.tag() {
            ChangeTag::Delete => "<red>-",
            ChangeTag::Insert => "<green>+",
            ChangeTag::Equal => continue,
        };

        info!("{} {}", sign, change.to_string().trim());
    }

    if diff.ratio() == 1.0 {
        return Ok(true);
    }

    trace!("Backing up {}", destination.display());

    let backup_path = Path::new(&destination).with_extension("bak");
    rename(&destination, &backup_path)?;

    return Ok(false);
}

fn new_handlerbars<'a, 'b>() -> anyhow::Result<Handlebars<'b>> {
    debug!("Creating Handlebars instance...");

    let mut handlebars = Handlebars::new();

    handlebars.set_strict_mode(true); // Report missing variables as errors
    handlebars.register_escape_fn(handlebars::no_escape); // Disable HTML escaping

    Ok(handlebars)
}

fn fix_permissions(path: &Path, conf: &EnvConf) -> anyhow::Result<()> {
    // Set permission to 755 for directories, 644 for files
    let mut perms = Permissions::from_mode(0o644);
    if path.is_dir() {
        perms = Permissions::from_mode(0o755);
    }

    let perm = if path.is_dir() {
        Permissions::from_mode(0o755)
    } else {
        Permissions::from_mode(0o644)
    };
    set_permissions(path, perm).context("Set permissions")?;

    let owner = conf
        .get_env("UID")
        .map(|uid| file_owner::Owner::from(uid.parse::<u32>().unwrap()))
        .or_else(|| {
            conf.get_env("USER")
                .map(|user| file_owner::Owner::from_name(&user).unwrap())
        })
        .context("Getting UID or USER environment variable")?;

    let group = conf
        .get_env("GID")
        .map(|gid| file_owner::Group::from(gid.parse::<u32>().unwrap()))
        .or_else(|| {
            conf.get_env("GROUP")
                .map(|group| file_owner::Group::from_name(&group).unwrap())
        })
        .unwrap_or(file_owner::Group::from_gid(owner.id()));

    file_owner::set_owner_group(path, owner, group).context("Setting file owner and group")?;

    Ok(())
}
