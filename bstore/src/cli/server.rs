use std::error::Error;

use clap::ArgMatches;

pub async fn run(_cli_matches: &ArgMatches) -> Result<(), Box<dyn Error>> {
    server::run().await
}
