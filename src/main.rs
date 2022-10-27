mod config;

use crate::config::{EnvConf, ServerContext};
use anyhow::{format_err, Context};
use clap::{command, Arg, ArgAction, ArgMatches};
use file_owner::{group, owner};
use handlebars::Handlebars;
use similar::{ChangeTag, DiffableStr, TextDiff};
use simplelog::__private::log::SetLoggerError;
use simplelog::{
    debug, error, info, trace, Color, ColorChoice, Config, ConfigBuilder, LevelFilter, TermLogger,
    TerminalMode,
};
use std::env::{current_dir, vars_os};
use std::error::Error;
use std::fs::{create_dir, create_dir_all, read, rename, set_permissions, File, Permissions};
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{exit, Command};
use std::{env, fs};
use walkdir::{DirEntry, WalkDir};

fn main() {
    let cli = get_cli();
    start_logger(&cli).context("Init logger").unwrap();
    let conf = match EnvConf::new(cli) {
        Ok(value) => value,
        Err(err) => {
            error!("Failed to init config -> {}", err);
            exit(19)
        }
    };

    match run(conf) {
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

fn run(conf: EnvConf) -> anyhow::Result<()> {
    let repo_str = conf
        .get_env("SERVER_SYNC_REPO_STORAGE")
        .context("Get repo storage location")?;
    let repo_dir = Path::new(&repo_str);
    sync_repository(&conf, &repo_dir).context("Sync repo")?;

    let mut handlebars = new_handlerbars().context("Initialize handlebars")?;

    debug!("Variables: {:?}", &conf.get_variables());

    for context in conf.get_contexts() {
        if !context.source_root.exists() || !context.source_root.is_dir() {
            return return Err(format_err!(
                "Server source root doesn't exist or is not a directory: {}",
                context.source_root.display()
            ));
        }

        info!("Processing context {}", context.name);
        debug!("Source root: {}", context.source_root.display());

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
    let walker = WalkDir::new(&context.source_root)
        .same_file_system(true)
        .into_iter()
        .filter(|e| e.is_ok())
        .filter(|e| e.as_ref().unwrap().file_type().is_file())
        .map(|e| e.unwrap());

    let mut non_utf8 = vec![];

    for entry in walker {
        let relative_path = entry
            .path()
            .strip_prefix(&context.source_root)
            .context("Get relative path")?;
        let destination_path = conf.destination_root.join(relative_path);

        trace!("Processing file {}", relative_path.display());

        let contents = match get_contents(entry.path()) {
            None => {
                non_utf8.push((entry.path().to_owned(), destination_path));
                continue;
            }
            Some(value) => value,
        };

        let rendered = render_entry(handlebars, &context, &conf, &contents, &entry)
            .context("Render source")?;
        let parent = destination_path.parent().expect("File was at / level???");

        trace!(
            "Templating {} to {}",
            &entry.path().display(),
            &destination_path.display()
        );

        ensure_ancestors(parent, &conf)?;

        if check_existing(&destination_path, &rendered)? {
            debug!("File {} is up to date", destination_path.display());
        } else {
            backup_and_write(&destination_path, rendered.as_bytes())?;
        }

        fix_permissions(&destination_path, &conf)?;
    }

    // TODO -> This is a bit of a hack, but it works for now.
    for (source, dest) in non_utf8 {
        trace!("Processing file {}", source.display());

        ensure_ancestors(&dest.parent().context("Get destination parent folder.")?, &conf)?;

        let buf = read(source).context("Read source file")?;
        if read(&dest)
            .context("Read existing file")
            .map(|e| e == buf)
            .unwrap_or(false)
        {
            debug!("File {} is up to date", dest.display());
        } else {
            backup_and_write(&dest, &buf)?;
        }

        fix_permissions(&dest, &conf)?;
    }

    Ok(())
}

fn backup_and_write(destination: &Path, contents: &[u8]) -> anyhow::Result<()> {
    trace!("Backing up {}", destination.display());
    let backup_path = Path::new(&destination).with_extension("bak");
    rename(&destination, &backup_path).context("Rename old file")?;

    trace!("Writing {}", destination.display());
    let mut file = File::create(&destination).context("Create file at destination")?;
    file.write_all(contents).context("Write out all bytes")?;

    Ok(())
}

fn ensure_ancestors(parent: &Path, conf: &EnvConf) -> anyhow::Result<()> {
    let ancestors_dirs = parent
        .ancestors()
        .collect::<Vec<&Path>>();

    for ancestor in ancestors_dirs.iter().rev() {
        if ancestor.starts_with(&conf.destination_root) {
            continue;
        }

        if !ancestor.exists() {
            create_dir(ancestor).context("Create ancestor directory")?;
        }

        fix_permissions(&ancestor, &conf)?;
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
    if path.is_symlink() {
        return Ok(());
    }

    let permission = if path.is_dir() {
        Permissions::from_mode(0o755)
    } else {
        Permissions::from_mode(0o644)
    };

    set_permissions(path, permission).context("Set permissions")?;

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
