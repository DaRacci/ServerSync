use anyhow::Context;
use clap::{command, Arg, ArgMatches};
use envfile::EnvFile;
use handlebars::Handlebars;
use similar::{ChangeTag, TextDiff};
use simplelog::__private::log::SetLoggerError;
use simplelog::{
    debug, error, info, trace, ColorChoice, Config, ConfigBuilder, LevelFilter, TermLogger,
    TerminalMode,
};
use std::any::Any;
use std::collections::BTreeMap;
use std::env;
use std::env::var_os;
use std::error::Error;
use std::fs::{create_dir_all, rename, File};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{exit, Command};
use walkdir::WalkDir;

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
        _ => LevelFilter::Info,
    };

    println!("level: {:?}", level);

    TermLogger::init(
        level,
        Config::default(),
        TerminalMode::Mixed,
        ColorChoice::Auto,
    )
}

fn run() -> Result<(), Box<dyn Error>> {
    let envfile = EnvFile::new(&Path::new(".env"));

    let raw_string = get_env("SERVER_NAME", &envfile)?;
    let source_names = raw_string.split(";");
    let path_str = get_env("SERVER_PATH", &envfile)?;

    let sources_str = source_names
        .map(|n| format!("servers/{n}/"))
        .collect::<Vec<String>>();
    let (server_dest, source_paths) = get_paths(&path_str, &sources_str)?;

    info!("Sources: {:?}", &sources_str);
    info!(
        "Source paths: {}",
        &source_paths
            .iter()
            .map(|x| x.to_str().unwrap())
            .collect::<Vec<&str>>()
            .join(", ")
    );
    info!("Destination Path: {}", &server_dest.display());

    let mut handlebars = new_handlerbars().context("Initialize handlebars")?;

    match sync_repository() {
        Ok(_) => Ok(()),
        Err(err) => Err(Box::<dyn Error>::from(err)),
    }?;

    let mut variables = BTreeMap::<String, String>::new();
    if let Ok(envs) = envfile {
        for (key, value) in envs.store {
            variables.insert(key, value);
        }
    }

    debug!("Variables: {:?}", &variables);

    for source in source_paths {
        info!("Syncing {}", &source.display());
        walk_directory(&source, &server_dest, &mut handlebars, &variables)?;
    }

    Ok(())
}

fn get_env<'a>(env: &str, envfile: &std::io::Result<EnvFile>) -> Result<String, Box<dyn Error>> {
    return match var_os(env) {
        Some(value) => Ok(value.to_string_lossy().to_string()),
        None => match envfile {
            Ok(envfile) => match envfile.get(env) {
                Some(value) => Ok(value.to_string()),
                None => Err(Box::<dyn Error>::from(format!("{} is not set", env))),
            },
            Err(_) => Err(Box::<dyn Error>::from(format!("{} is not set", env))),
        },
    };
}

fn get_paths<'a>(
    path_str: &'a str,
    sources: &'a Vec<String>,
) -> Result<(&'a Path, Vec<&Path>), Box<dyn Error>> {
    let server_path = Path::new(path_str);
    exists_and_dir(server_path)?;

    let mut source_paths = Vec::new();
    for source in sources {
        let source_path = Path::new(source);
        exists_and_dir(source_path)?;
        source_paths.push(source_path);
    }

    Ok((server_path, source_paths))
}

fn exists_and_dir(path: &Path) -> Result<(), Box<dyn Error>> {
    if !path.exists() || !path.is_dir() {
        return Err(Box::<dyn Error>::from(format!(
            "Destination path {} does not exist or is not a directory!",
            path.display()
        )));
    }

    Ok(())
}

fn sync_repository() -> Result<(), Box<dyn Error>> {
    let current_dir = env::current_dir()?;

    let output = Command::new("git")
        .arg("pull")
        .current_dir(&current_dir)
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

fn walk_directory<'a>(
    root: &Path,
    to: &Path,
    handlebars: &mut Handlebars,
    variables: &BTreeMap<String, String>,
) -> Result<(), Box<dyn Error>> {
    let server_name = root
        .strip_prefix("servers/")
        .unwrap()
        .to_string_lossy()
        .to_string();

    let walker = WalkDir::new(root)
        .same_file_system(true)
        .into_iter()
        .filter(|e| e.is_ok());

    for result in walker {
        let entry = match result {
            Ok(unwrapped) => unwrapped,
            Err(_) => continue,
        };

        if entry.path().is_dir() {
            continue;
        }

        let relative_str = entry
            .path()
            .strip_prefix(root)
            .unwrap()
            .to_str()
            .unwrap()
            .clone();
        let target_path = to.join(&relative_str);
        let absolute_source = env::current_dir().unwrap().join(&root).join(&relative_str);

        trace!("Current file: {}", &absolute_source.display());

        let mut source_contents = String::new();
        if let Err(_) = File::open(&absolute_source)?.read_to_string(&mut source_contents) {
            continue;
        };

        let mut variables_cloned = variables.clone();
        variables_cloned.insert(String::from("server_name"), server_name.to_owned());
        handlebars.register_template_string(&relative_str, &source_contents)?;

        let rendered = handlebars.render(&relative_str, &variables_cloned)?;
        let parent = target_path.parent().expect("File was at / level???");

        trace!(
            "Templating {} to {}",
            &absolute_source.display(),
            &target_path.display()
        );

        if !parent.exists() {
            debug!("Creating new directory {}", target_path.display());
            create_dir_all(&parent)?;
        }

        if target_path.exists() {
            let mut open_file = File::open(&target_path)?;

            let mut target_contents = String::new();
            if let Err(_) = open_file.read_to_string(&mut target_contents) {
                continue;
            }

            let diff = TextDiff::from_lines(&target_contents, &rendered);
            for change in diff.iter_all_changes() {
                let sign = match change.tag() {
                    ChangeTag::Delete => "-",
                    ChangeTag::Insert => "+",
                    ChangeTag::Equal => continue,
                };

                print!("{} {}", sign, change);
            }

            if diff.ratio() == 1.0 {
                debug!(
                    "Skipping {} as it is already up to date",
                    &relative_str
                );
                continue;
            }

            trace!("Backing up {}", target_path.display());
            let backup_path = Path::new(&target_path).with_extension("bak");
            rename(&target_path, &backup_path)?;
        }

        trace!("Writing {}", target_path.display());
        let mut file = File::create(&target_path)?;
        file.write_all(rendered.as_bytes())?;
    }

    Ok(())
}

fn new_handlerbars<'a, 'b>() -> anyhow::Result<Handlebars<'b>> {
    debug!("Creating Handlebars instance...");

    let mut handlebars = Handlebars::new();

    handlebars.set_strict_mode(true); // Report missing variables as errors
    handlebars.register_escape_fn(handlebars::no_escape); // Disable HTML escaping

    Ok(handlebars)
}
