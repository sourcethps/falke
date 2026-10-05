use std::path::PathBuf;

use anyhow::{anyhow, Result};
use simplelog::{
    ColorChoice, CombinedLogger, ConfigBuilder, TermLogger, TerminalMode, WriteLogger,
};

pub fn init(filename: &str) -> Result<()> {
    let mut log_path = get_exe_folder()?;
    log_path.push(filename);

    let log_conf = ConfigBuilder::new()
        .set_time_offset_to_local()
        .unwrap()
        .build();
    CombinedLogger::init(vec![
        TermLogger::new(
            log::LevelFilter::Debug,
            log_conf.clone(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ),
        WriteLogger::new(
            log::LevelFilter::Debug,
            log_conf.clone(),
            std::fs::File::create(log_path)?,
        ),
    ])?;

    Ok(())
}

fn get_exe_folder() -> Result<PathBuf> {
    let path =
        std::env::current_exe().map_err(|err| anyhow!("failed to get binary path, {err}"))?;
    path.parent()
        .ok_or_else(|| anyhow!("failed to get binary folder"))
        .map(|v| v.to_path_buf())
}
