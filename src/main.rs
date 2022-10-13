mod config;

use crate::config::{EnvConf, ServerContext};
use anyhow::Context;
use clap::{command, Arg, ArgMatches};
use handlebars::Handlebars;
use similar::{ChangeTag, TextDiff};
use simplelog::__private::log::SetLoggerError;
use simplelog::{
    debug, error, info, trace, Color, ColorChoice, Config, LevelFilter, TermLogger, TerminalMode,
};
use std::error::Error;
use std::fs::{create_dir_all, rename, File, Permissions};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{exit, Command};
use std::{env, fs};
use walkdir::{DirEntry, WalkDir};

fn main() {
    let cli = get_cli();
    start_logger(&cli).unwrap();

    match run() {
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
        .args([Arg::new("VERBOSE")
            .short('v')
            .action(clap::ArgAction::Count)])
        .get_matches()
}

fn start_logger(matches: &ArgMatches) -> Result<(), SetLoggerError> {
    let level = matches.get_count("VERBOSE");
    let level = match level {
        2 => LevelFilter::Trace,
        1 => LevelFilter::Debug,
        0 => LevelFilter::Info,
        _ => {
            error!("Too many -v flags");
            LevelFilter::Trace
        }
    };

    TermLogger::init(
        level,
        Config::default(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    )
}

fn run() -> Result<(), Box<dyn Error>> {
    let conf = EnvConf::new()?;
    let mut handlebars = new_handlerbars().context("Initialize handlebars")?;

    sync_repository(&conf)?;

    debug!("Destination root: {}", &conf.destination_root.display());
    debug!("Variables: {:?}", &conf.get_variables());

    for context in conf.get_contexts() {
        info!("Processing context {}", context.name);
        debug!("Source root: {}", context.source_root.display());

        walk_directory(&mut handlebars, &context, &conf)?;
    }

    Ok(())
}

fn sync_repository(conf: &EnvConf) -> Result<(), Box<dyn Error>> {
    let current_dir = env::current_dir()?;

    let output = Command::new("git")
        .current_dir(&current_dir)
        .arg("pull")
        .arg(conf.get_env("GIT_REMOTE").unwrap_or(&"origin".to_string()))
        .arg(conf.get_env("GIT_BRANCH").unwrap_or(&"master".to_string()))
        .output()?;

    let output_str = String::from_utf8_lossy(&output.stdout);

    if !output.status.success() {
        return Err(Box::<dyn Error>::from(format!(
            "Failed to synchronize with repository! Error: {}",
            &output_str
        )));
    } else if &output_str == "Already up to date." {
        info!("Repository is already up to date!");
    } else {
        info!("Successfully synchronized with repository!");
        debug!("Git pull output -> {}", &output_str);
    }

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

    for entry in walker {
        let relative_path = entry.path().strip_prefix(&context.source_root)?;
        let destination_path = conf.destination_root.join(relative_path);

        trace!("Processing file {}", relative_path.display());

        let contents = match get_contents(entry.path()) {
            None => continue,
            Some(value) => value,
        };

        let rendered = render_entry(handlebars, &context, &conf, &contents, &entry)?;
        let parent = destination_path.parent().expect("File was at / level???");

        trace!(
            "Templating {} to {}",
            &entry.path().display(),
            &destination_path.display()
        );

        if !parent.exists() {
            debug!("Creating new directory {}", destination_path.display());
            create_dir_all(&parent)?;
        }

        if check_existing(&destination_path, &rendered)? {
            debug!("File {} is up to date", destination_path.display());
        } else {
            trace!("Writing {}", destination_path.display());
            let mut file = File::create(&destination_path)?;
            file.write_all(rendered.as_bytes())?;
        }

        fix_permissions(&destination_path, &conf)?;
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

        info!("{} {}", sign, change);
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
    fs::set_permissions(path, Permissions::from_mode(0o644))?;

    let owner = conf
        .get_env("UID")
        .map(|uid| file_owner::Owner::from(uid.parse::<u32>().unwrap()))
        .or_else(|| {
            conf.get_env("USER")
                .map(|user| file_owner::Owner::from_name(user).unwrap())
        })
        .context("Getting UID or USER environment variable")?;

    let group = conf
        .get_env("GID")
        .map(|gid| file_owner::Group::from(gid.parse::<u32>().unwrap()))
        .or_else(|| {
            conf.get_env("GROUP")
                .map(|group| file_owner::Group::from_name(group).unwrap())
        })
        .context("Getting GID or GROUP environment variable")?;

    file_owner::set_owner_group(path, owner, group)?;

    Ok(())
}
