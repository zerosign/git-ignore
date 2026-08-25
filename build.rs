use vergen_gix::{Build, Emitter, Gix};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let build = Build::builder().build_timestamp(true).build();
    let gix = Gix::builder().sha(true).build();

    Emitter::default()
        .add_instructions(&build)?
        .add_instructions(&gix)?
        .emit()?;

    Ok(())
}
