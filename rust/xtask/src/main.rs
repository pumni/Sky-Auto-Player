#![forbid(unsafe_code)]

mod audits;
mod branding;
mod builtin_catalog;
mod checks;
mod classifier;
mod hash;
mod process;
mod release_metadata;
mod repo;
mod sbom;
mod supply_chain;
mod tauri_bundle;
mod updater_trust;
mod version;

use std::env;
use std::path::Path;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn usage() -> &'static str {
    "Usage:\n  cargo xtask check <static|rust|desktop|all> [--skip-supply-chain]\n  cargo xtask audit supply-chain [--attestation <path>]\n  cargo xtask ci classify [--full | --base <sha> --head <sha> | --paths-file <file>]\n  cargo xtask version check [--tag <tag>]\n  cargo xtask bindings <generate|check>\n  cargo xtask branding validate\n  cargo xtask branding build-ico --layers-dir <dir> --output <ico>\n  cargo xtask builtin-catalog <verify|verify-installed|refresh|add|rename|retire|restore> [options]\n  cargo xtask verify-tauri-bundle --bundle-dir <dir> --authenticode-evidence <path> --sbom <path> [--summary <path>]\n  cargo xtask sbom <generate|verify> --artifact-dir <dir> --output|--sbom <path>\n  cargo xtask updater-trust <inventory|export-public-key|verify-private-key|rotation-self-test>\n  cargo xtask release-metadata generate --channel <stable|beta> --version <semver> --notes-file <path> --pub-date <rfc3339> --platform windows-x86_64 --asset-url <url> --signature-file <path> --output <path>\n  cargo xtask release-metadata validate --channel <stable|beta> --metadata <path>\n  cargo xtask release-metadata validate-monotonic --channel <stable|beta> --current <path> --candidate <path>"
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
            let skip_supply_chain = args.iter().any(|a| a == "--skip-supply-chain");
            checks::run(group, skip_supply_chain)
        }
        "audit" if args.get(1).map(String::as_str) == Some("supply-chain") => {
            let mut attestation = None;
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--attestation" => {
                        attestation = Some(required_value(&args, &mut i, "--attestation")?)
                    }
                    option => return Err(format!("unknown supply-chain option: {option}").into()),
                }
                i += 1;
            }
            supply_chain::run(attestation.as_deref().map(Path::new))
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
        "bindings" => match args.get(1).map(String::as_str) {
            Some("check") => checks::bindings(),
            Some("generate") => checks::bindings_generate(),
            _ => Err("bindings requires generate or check".into()),
        },
        "branding" if args.get(1).map(String::as_str) == Some("validate") => {
            if args.len() != 2 {
                return Err("branding validate does not accept options".into());
            }
            branding::validate(&repo::root())
        }
        "branding" if args.get(1).map(String::as_str) == Some("build-ico") => {
            let mut layers = None;
            let mut output = None;
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--layers-dir" => layers = Some(required_value(&args, &mut i, "--layers-dir")?),
                    "--output" => output = Some(required_value(&args, &mut i, "--output")?),
                    option => return Err(format!("unknown branding option: {option}").into()),
                }
                i += 1;
            }
            branding::write_ico(
                Path::new(&layers.ok_or("branding requires --layers-dir")?),
                Path::new(&output.ok_or("branding requires --output")?),
            )
        }
        "builtin-catalog" => {
            let operation = args
                .get(1)
                .ok_or("builtin-catalog requires an operation")?
                .clone();
            let mut options = Vec::new();
            let mut i = 2;
            while i < args.len() {
                let option = args[i]
                    .strip_prefix("--")
                    .ok_or_else(|| {
                        format!("builtin-catalog option must start with --: {}", args[i])
                    })?
                    .to_owned();
                let value = required_value(&args, &mut i, &format!("--{option}"))?;
                options.push((option, value));
                i += 1;
            }
            builtin_catalog::run(&repo::root(), &operation, &options)
        }
        "verify-tauri-bundle" => {
            let mut bundle_dir = None;
            let mut summary = None;
            let mut authenticode_evidence = None;
            let mut sbom_path = None;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--bundle-dir" => {
                        bundle_dir = Some(required_value(&args, &mut i, "--bundle-dir")?)
                    }
                    "--summary" => summary = Some(required_value(&args, &mut i, "--summary")?),
                    "--authenticode-evidence" => {
                        authenticode_evidence =
                            Some(required_value(&args, &mut i, "--authenticode-evidence")?)
                    }
                    "--sbom" => sbom_path = Some(required_value(&args, &mut i, "--sbom")?),
                    option => {
                        return Err(format!("unknown verify-tauri-bundle option: {option}").into());
                    }
                }
                i += 1;
            }
            tauri_bundle::verify(
                &repo::root(),
                Path::new(
                    bundle_dir
                        .ok_or("verify-tauri-bundle requires --bundle-dir <dir>")?
                        .as_str(),
                ),
                summary.as_deref().map(Path::new),
                authenticode_evidence.as_deref().map(Path::new),
                sbom_path.as_deref().map(Path::new),
            )
        }
        "sbom" => {
            let operation = args.get(1).map(String::as_str);
            let mut artifact_dir = None;
            let mut output = None;
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--artifact-dir" => {
                        artifact_dir = Some(required_value(&args, &mut i, "--artifact-dir")?)
                    }
                    "--output" | "--sbom" => {
                        output = Some(required_value(&args, &mut i, "--output/--sbom")?)
                    }
                    option => return Err(format!("unknown sbom option: {option}").into()),
                }
                i += 1;
            }
            let artifact_dir = Path::new(
                artifact_dir
                    .ok_or("sbom requires --artifact-dir <dir>")?
                    .as_str(),
            )
            .to_owned();
            let output =
                Path::new(&output.ok_or("sbom requires --output <path> or --sbom <path>")?)
                    .to_owned();
            match operation {
                Some("generate") => sbom::generate(&repo::root(), &artifact_dir, &output),
                Some("verify") => sbom::verify(&repo::root(), &artifact_dir, &output),
                _ => Err("sbom requires generate or verify".into()),
            }
        }
        "updater-trust" if args.get(1).map(String::as_str) == Some("inventory") => {
            updater_trust::print_inventory(&repo::root())
        }
        "updater-trust" if args.get(1).map(String::as_str) == Some("export-public-key") => {
            let mut output = None;
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--output" => output = Some(required_value(&args, &mut i, "--output")?),
                    option => {
                        return Err(format!(
                            "unknown updater-trust export-public-key option: {option}"
                        )
                        .into());
                    }
                }
                i += 1;
            }
            let output = output.ok_or("export-public-key requires --output <path>")?;
            updater_trust::export_canonical_public_key(&repo::root(), Path::new(&output))
        }
        "updater-trust" if args.get(1).map(String::as_str) == Some("verify-private-key") => {
            let mut key_file = None;
            let mut password_env = None;
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--key-file" => key_file = Some(required_value(&args, &mut i, "--key-file")?),
                    "--password-env" => {
                        password_env = Some(required_value(&args, &mut i, "--password-env")?)
                    }
                    option => {
                        return Err(format!(
                            "unknown updater-trust verify-private-key option: {option}"
                        )
                        .into());
                    }
                }
                i += 1;
            }
            let key_file = key_file
                .or_else(|| env::var("TAURI_SIGNING_PRIVATE_KEY_PATH").ok())
                .ok_or(
                    "verify-private-key requires --key-file <path> or TAURI_SIGNING_PRIVATE_KEY_PATH env var",
                )?;
            let password = if let Some(env_var) = password_env {
                env::var(env_var).ok()
            } else {
                env::var("TAURI_SIGNING_PRIVATE_KEY_PASSWORD").ok()
            };
            updater_trust::verify_local_private_key(
                &repo::root(),
                Path::new(&key_file),
                password.as_deref(),
            )?;
            let key_id = updater_trust::canonical_key_id()?;
            println!(
                "[xtask] Local updater private key matches canonical production v4 root (Key ID: {key_id})"
            );
            Ok(())
        }
        "updater-trust" if args.get(1).map(String::as_str) == Some("verify-signature") => {
            let mut installer = None;
            let mut signature = None;
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--installer" => {
                        installer = Some(required_value(&args, &mut i, "--installer")?)
                    }
                    "--signature" => {
                        signature = Some(required_value(&args, &mut i, "--signature")?)
                    }
                    option => {
                        return Err(format!(
                            "unknown updater-trust verify-signature option: {option}"
                        )
                        .into());
                    }
                }
                i += 1;
            }
            let installer = installer.ok_or("verify-signature requires --installer <path>")?;
            let signature = signature.ok_or("verify-signature requires --signature <path>")?;
            updater_trust::verify_updater_signature(Path::new(&installer), Path::new(&signature))?;
            Ok(())
        }
        "updater-trust" if args.get(1).map(String::as_str) == Some("rotation-self-test") => {
            let mut old_public = None;
            let mut new_public = None;
            let mut old_signature = None;
            let mut new_signature = None;
            let mut payload = None;
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--old-public" => {
                        old_public = Some(required_value(&args, &mut i, "--old-public")?)
                    }
                    "--new-public" => {
                        new_public = Some(required_value(&args, &mut i, "--new-public")?)
                    }
                    "--old-signature" => {
                        old_signature = Some(required_value(&args, &mut i, "--old-signature")?)
                    }
                    "--new-signature" => {
                        new_signature = Some(required_value(&args, &mut i, "--new-signature")?)
                    }
                    "--payload" => payload = Some(required_value(&args, &mut i, "--payload")?),
                    option => return Err(format!("unknown updater-trust option: {option}").into()),
                }
                i += 1;
            }
            let old_public = old_public.ok_or("rotation-self-test requires --old-public <path>")?;
            let new_public = new_public.ok_or("rotation-self-test requires --new-public <path>")?;
            let old_signature =
                old_signature.ok_or("rotation-self-test requires --old-signature <path>")?;
            let new_signature =
                new_signature.ok_or("rotation-self-test requires --new-signature <path>")?;
            let payload = payload.ok_or("rotation-self-test requires --payload <path>")?;
            updater_trust::rotation_self_test(Path::new(&old_public), Path::new(&new_public))?;
            updater_trust::verify_rotation_signatures(
                Path::new(&old_public),
                Path::new(&new_public),
                Path::new(&old_signature),
                Path::new(&new_signature),
                Path::new(&payload),
            )
        }
        "release-metadata" => match args.get(1).map(String::as_str) {
            Some("generate") => {
                let mut channel = None;
                let mut version = None;
                let mut notes_file = None;
                let mut pub_date = None;
                let mut platform = None;
                let mut asset_url = None;
                let mut signature_file = None;
                let mut output = None;
                let mut i = 2;
                while i < args.len() {
                    match args[i].as_str() {
                        "--channel" => channel = Some(required_value(&args, &mut i, "--channel")?),
                        "--version" => version = Some(required_value(&args, &mut i, "--version")?),
                        "--notes-file" => {
                            notes_file = Some(required_value(&args, &mut i, "--notes-file")?)
                        }
                        "--pub-date" => {
                            pub_date = Some(required_value(&args, &mut i, "--pub-date")?)
                        }
                        "--platform" => {
                            platform = Some(required_value(&args, &mut i, "--platform")?)
                        }
                        "--asset-url" => {
                            asset_url = Some(required_value(&args, &mut i, "--asset-url")?)
                        }
                        "--signature-file" => {
                            signature_file =
                                Some(required_value(&args, &mut i, "--signature-file")?)
                        }
                        "--output" => output = Some(required_value(&args, &mut i, "--output")?),
                        option => {
                            return Err(format!(
                                "unknown release-metadata generate option: {option}"
                            )
                            .into());
                        }
                    }
                    i += 1;
                }
                release_metadata::generate(release_metadata::GenerateInput {
                    channel: release_metadata::Channel::parse(
                        channel
                            .as_deref()
                            .ok_or("release-metadata generate requires --channel <stable|beta>")?,
                    )?,
                    version: version
                        .as_deref()
                        .ok_or("release-metadata generate requires --version <semver>")?,
                    notes_path: Path::new(
                        notes_file
                            .as_deref()
                            .ok_or("release-metadata generate requires --notes-file <path>")?,
                    ),
                    pub_date: pub_date
                        .as_deref()
                        .ok_or("release-metadata generate requires --pub-date <rfc3339>")?,
                    platform: platform
                        .as_deref()
                        .ok_or("release-metadata generate requires --platform windows-x86_64")?,
                    asset_url: asset_url
                        .as_deref()
                        .ok_or("release-metadata generate requires --asset-url <url>")?,
                    signature_path: Path::new(
                        signature_file
                            .as_deref()
                            .ok_or("release-metadata generate requires --signature-file <path>")?,
                    ),
                    output: Path::new(
                        output
                            .as_deref()
                            .ok_or("release-metadata generate requires --output <path>")?,
                    ),
                })
            }
            Some("validate") => {
                let mut channel = None;
                let mut metadata = None;
                let mut i = 2;
                while i < args.len() {
                    match args[i].as_str() {
                        "--channel" => channel = Some(required_value(&args, &mut i, "--channel")?),
                        "--metadata" => {
                            metadata = Some(required_value(&args, &mut i, "--metadata")?)
                        }
                        option => {
                            return Err(format!(
                                "unknown release-metadata validate option: {option}"
                            )
                            .into());
                        }
                    }
                    i += 1;
                }
                release_metadata::validate(
                    release_metadata::Channel::parse(
                        channel
                            .as_deref()
                            .ok_or("release-metadata validate requires --channel <stable|beta>")?,
                    )?,
                    Path::new(
                        metadata
                            .as_deref()
                            .ok_or("release-metadata validate requires --metadata <path>")?,
                    ),
                )
            }
            Some("validate-monotonic") => {
                let mut channel = None;
                let mut current = None;
                let mut candidate = None;
                let mut i = 2;
                while i < args.len() {
                    match args[i].as_str() {
                        "--channel" => channel = Some(required_value(&args, &mut i, "--channel")?),
                        "--current" => current = Some(required_value(&args, &mut i, "--current")?),
                        "--candidate" => {
                            candidate = Some(required_value(&args, &mut i, "--candidate")?)
                        }
                        option => {
                            return Err(format!(
                                "unknown release-metadata validate-monotonic option: {option}"
                            )
                            .into());
                        }
                    }
                    i += 1;
                }
                release_metadata::validate_monotonic(
                    release_metadata::Channel::parse(channel.as_deref().ok_or(
                        "release-metadata validate-monotonic requires --channel <stable|beta>",
                    )?)?,
                    Path::new(
                        current.as_deref().ok_or(
                            "release-metadata validate-monotonic requires --current <path>",
                        )?,
                    ),
                    Path::new(candidate.as_deref().ok_or(
                        "release-metadata validate-monotonic requires --candidate <path>",
                    )?),
                )
            }
            _ => Err("release-metadata requires generate, validate, or validate-monotonic".into()),
        },
        _ => {
            eprintln!("{}", usage());
            Err(format!("unknown command: {}", args[0]).into())
        }
    }
}
