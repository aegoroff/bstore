use clap::ArgMatches;

pub async fn run(_cli_matches: &ArgMatches)  { 
    server::run().await;
}