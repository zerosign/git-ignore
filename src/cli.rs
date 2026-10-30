use usage_rs::{Cli, Subcommands};

use crate::command::{
    compact::Compact, info::Info, list::List, patch::Patch, update::Update, version::Version,
};
//
// pub struct CliState {
//     config: Config,
//     out: Box<dyn io::Write>,
//     err: Box<dyn io::Write>,
// }

#[derive(Cli, Debug)]
#[usage(bin = "git-ignore")]
/// Git-ignore CLI
pub struct GitIgnoreCli {
    #[usage(subcommand)]
    pub command: Command,
}

#[derive(Subcommands, Debug)]
#[usage(run_with)] // Generates RunWith<AppContext> for Command
pub enum Command {
    /// Update the local templates repository
    // Update {
    //     #[usage(short, long)]
    //     /// Force a clean update (delete local caches and re-index from scratch)
    //     force: bool,
    // },
    Update(Update),

    /// Patch the current .gitignore file instead of overwriting
    Patch(Patch),

    /// List all available templates
    List(List),

    /// Show information about current setup and integrity
    Info(Info),

    /// Compact the local database to save space
    Compact(Compact),

    /// Show version information
    Version(Version),
}
