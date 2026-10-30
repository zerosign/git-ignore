use std::io::{self, BufWriter};

// use argh::FromArgs;
use git_ignore::{CliState, Config, cli::GitIgnoreCli};
use usage_rs::RunWith;
// use mimalloc::MiMalloc;
//
// #[global_allocator]
// static GLOBAL: MiMalloc = MiMalloc;
//
/// Entry point for the git-ignore CLI.
///
/// **Logic:**
/// 1. Initializes the `gix` interrupt handler to handle CTRL+C safely during long network operations (cloning/fetching).
/// 2. Checks if any arguments were provided; if not, triggers the auto-generated help message.
/// 3. Parses command-line arguments using `argh`.
/// 4. Resolves the execution configuration (data paths, environment variables).
/// 5. Executes the core CLI logic and reports any errors to the user.
fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let cli = GitIgnoreCli::parse();
    let config = Config::from_env()?;

    let stdout = BufWriter::new(io::stdout().lock()); 
    let stderr = BufWriter::new(io::stderr().lock());

    let state = CliState {
        config: config,
        stdout: Box::new(stdout),
        stderr: Box::new(stderr),
    };

    cli.command.run_with(state)?;

    Ok(())
}
