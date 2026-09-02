use std::env;

fn usage() -> &'static str {
    "Usage:\n  sky_ci_classifier [--full | --base <sha> --head <sha> | --paths-file <file>]"
}

fn required_value(
    args: &[String],
    index: &mut usize,
    option: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| format!("missing value for {option}").into())
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut full = false;
    let mut base = None;
    let mut head = None;
    let mut paths_file = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--full" => full = true,
            "--base" => base = Some(required_value(&args, &mut i, "--base")?),
            "--head" => head = Some(required_value(&args, &mut i, "--head")?),
            "--paths-file" => paths_file = Some(required_value(&args, &mut i, "--paths-file")?),
            "--help" | "-h" => {
                println!("{}", usage());
                return Ok(());
            }
            option => return Err(format!("unknown option: {option}\n{}", usage()).into()),
        }
        i += 1;
    }

    sky_ci_classifier::run(
        full,
        base.as_deref(),
        head.as_deref(),
        paths_file.as_deref(),
    )
}
