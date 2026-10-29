use usage_rs::{Args, RunWith};

use crate::Config;

#[derive(Args, Debug)]
pub struct List;

impl RunWith<Config> for List {
    type Output = crate::result::Result<()>;

    fn run_with(self, ctx: Config) -> Self::Output {
        todo!()
    }
}
