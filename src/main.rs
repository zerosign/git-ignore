use argh::FromArgs;
use git_ignore::{Config, GitIgnoreArgs, run_cli};
use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

/// Entry point for the git-ignore CLI.
/// 
/// **Logic:**
/// 1. Initializes the `gix` interrupt handler to handle CTRL+C safely during long network operations (cloning/fetching).
/// 2. Checks if any arguments were provided; if not, triggers the auto-generated help message.
/// 3. Parses command-line arguments using `argh`.
/// 4. Resolves the execution configuration (data paths, environment variables).
/// 5. Executes the core CLI logic and reports any errors to the user.
fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // If no arguments provided (only the binary name), show help and exit cleanly.
    if std::env::args().len() == 1 {
        match GitIgnoreArgs::from_args(&["git-ignore"], &["--help"]) {
            Ok(_) => unreachable!(),
            Err(exit) => {
                println!("{}", exit.output);
                std::process::exit(if exit.status.is_ok() { 0 } else { 1 });
            }
        }
    }

    // Handle CTRL+C gracefully for gix operations.
    // Setting to 2 interrupts will cause an immediate hard exit on the second press.
    // The first interrupt allows gix to try and stop ongoing network/file operations cleanly.
    unsafe {
        let _ = gix::interrupt::init_handler(2, || {});
    }
    
    let args: GitIgnoreArgs = argh::from_env();
    let config = Config::from_env()?;
    
    // Core orchestration logic is separated into lib.rs for testability and reuse.
    run_cli(args, &config, std::io::stdout())?;
    
    Ok(())
}
