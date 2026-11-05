use clap::CommandFactory;

use crate::ServerCli;

#[test]
fn cli_has_no_short_option_conflicts() -> Result<(), Box<dyn std::error::Error>> {
    ServerCli::command().debug_assert();
    Ok(())
}
