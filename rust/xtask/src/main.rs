#![forbid(unsafe_code)]

mod checks;
mod classifier;
mod dist;
mod manifest;
mod process;
mod repo;
mod version;

use std::env;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn usage() -> &'static str {
    "Usage:\n  cargo xtask check <static|rust|desktop|all>\n  cargo xtask ci classify [--full | --base <sha> --head <sha> | --paths-file <file>]\n  cargo xtask version check [--tag <tag>]\n  cargo xtask bindings check\n  cargo xtask dist --profile dist --output <dir>\n  cargo xtask verify-dist --release-dir <dir>"
}

fn required_value(args: &[String], index: &mut usize, option: &str) -> Result<String> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| format!("missing value for {option}").into())
}

fn main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("{}", usage());
        return Err("a command is required".into());
    }
    match args[0].as_str() {
        "check" => {
            let group = args.get(1).map(String::as_str).unwrap_or("all");
            checks::run(group)
        }
        "ci" if args.get(1).map(String::as_str) == Some("classify") => {
            let mut full = false;
            let mut base = None;
            let mut head = None;
            let mut paths_file = None;
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--full" => full = true,
                    "--base" => base = Some(required_value(&args, &mut i, "--base")?),
                    "--head" => head = Some(required_value(&args, &mut i, "--head")?),
                    "--paths-file" => {
                        paths_file = Some(required_value(&args, &mut i, "--paths-file")?)
                    }
                    option => return Err(format!("unknown classifier option: {option}").into()),
                }
                i += 1;
            }
            classifier::run(
                full,
                base.as_deref(),
                head.as_deref(),
                paths_file.as_deref(),
            )
        }
        "version" if args.get(1).map(String::as_str) == Some("check") => {
            let mut tag = None;
            let mut i = 2;
            while i < args.len() {
                if args[i] == "--tag" {
                    tag = Some(required_value(&args, &mut i, "--tag")?);
                } else {
                    return Err(format!("unknown version option: {}", args[i]).into());
                }
                i += 1;
            }
            version::check(tag.as_deref())
        }
        "bindings" if args.get(1).map(String::as_str) == Some("check") => checks::bindings(),
        "dist" => {
            let mut profile = None;
            let mut output = None;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--profile" => profile = Some(required_value(&args, &mut i, "--profile")?),
                    "--output" => output = Some(required_value(&args, &mut i, "--output")?),
                    option => return Err(format!("unknown dist option: {option}").into()),
                }
                i += 1;
            }
            if profile.as_deref() != Some("dist") {
                return Err("dist requires --profile dist".into());
            }
            dist::build(output.ok_or("dist requires --output <dir>")?.as_ref())
        }
        "verify-dist" => {
            let mut release_dir = None;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--release-dir" => {
                        release_dir = Some(required_value(&args, &mut i, "--release-dir")?)
                    }
                    option => return Err(format!("unknown verify-dist option: {option}").into()),
                }
                i += 1;
            }
            manifest::verify_release(
                release_dir
                    .ok_or("verify-dist requires --release-dir <dir>")?
                    .as_ref(),
            )
        }
        _ => {
            eprintln!("{}", usage());
            Err(format!("unknown command: {}", args[0]).into())
        }
    }
}
