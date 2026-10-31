use std::io::Write;

use usage_rs::{Args, RunWith};

use crate::command::state::CliState;

#[derive(Args, Debug)]
pub struct Completion {
    /// Shell to generate completions for (bash, fish, zsh, elvish, powershell)
    pub(crate) shell: String,
}

impl RunWith<CliState> for Completion {
    type Output = crate::result::Result<()>;

    fn run_with(self, mut ctx: CliState) -> Self::Output {
        let shell = match self.shell.to_lowercase().as_str() {
            "fish" => usage_rs::complete::Shell::Fish,
            "bash" => usage_rs::complete::Shell::Bash,
            "zsh" => usage_rs::complete::Shell::Zsh,
            "elvish" => usage_rs::complete::Shell::Elvish,
            "powershell" | "pwsh" => usage_rs::complete::Shell::PowerShell,
            other => {
                writeln!(
                    &mut ctx.stderr,
                    "Unsupported shell: {other}. Available: bash, fish, zsh, elvish, powershell"
                )?;
                return Ok(());
            }
        };

        let script = usage_rs::script::script("git-ignore", shell);
        write!(&mut ctx.stdout, "{script}")?;

        Ok(())
    }
}
