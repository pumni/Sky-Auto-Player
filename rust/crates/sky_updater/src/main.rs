use std::env;
use std::process::ExitCode;

use sky_updater::error::Result;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!(
                "Sky Auto Player updater failed code={}: {error}",
                sky_updater::result::error_code(&error)
            );
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<()> {
    sky_updater::runner::run_production(env::args())
}
