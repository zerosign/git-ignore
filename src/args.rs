use argh::FromArgs;

#[allow(clippy::struct_excessive_bools)]
#[derive(FromArgs, PartialEq, Debug, Clone)]
/// Git-ignore CLI
pub struct GitIgnoreArgs {
    #[argh(switch, short = 'u')]
    /// update the local templates repository
    pub update: bool,
    #[argh(switch, short = 'f')]
    /// force a clean update (delete local repo caches and re-index all templates from scratch)
    /// TODO(@zerosign): we need to differentiate this to only just reindexing
    pub force: bool,
    #[argh(switch, short = 'p')]
    /// patch the current .gitignore file instead of overwriting
    pub patch: bool,
    #[argh(switch, short = 'l')]
    /// list all available templates
    pub list: bool,
    #[argh(switch, short = 'i')]
    /// show information about current setup and integrity
    pub info: bool,
    #[argh(switch, short = 'v')]
    /// show version information
    pub version: bool,
    #[argh(switch)]
    /// compact the local database to save space
    pub compact: bool,
    #[argh(positional)]
    /// comma-separated list of templates (e.g., neovim,Node,C++)
    pub templates: Vec<String>,
}

