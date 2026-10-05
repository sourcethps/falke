use anyhow::{Context, Result};

use crate::{dx_hooks, game, logger};

pub const FALKE_VERSION: &str = "0.5.0";

pub fn main() -> Result<()> {
    setup_logger()?;
    install_hooks()?;

    Ok(())
}

pub fn setup_logger() -> Result<()> {
    logger::init("falke.log")?;
    log::info!("{} initialized", version_string());
    Ok(())
}

pub fn install_hooks() -> Result<()> {
    unsafe {
        dx_hooks::install_hooks().context("failed to install d3d9 hooks")?;
    }
    game::init()?;
    log::info!("hooks installed");
    Ok(())
}

pub fn version_string() -> String {
    let sha = env!("VERGEN_GIT_SHA");
    let short_sha = sha.get(..7).unwrap_or(sha);

    format!("falke {} ({})", FALKE_VERSION, short_sha)
}
