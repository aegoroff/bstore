use clap::{Command, arg, command, crate_name};
use cli::client::{insert_single_file, list_buckets};
use client::FileParams;

mod cli;

#[tokio::main]
async fn main() {
    let cli = command!(crate_name!())
        .about(clap::crate_description!())
        .subcommand(Command::new(cli::BUGREPORT_SUBCOMMAND).about(cli::BUGREPORT_DESCRIPTION))
        .subcommand(Command::new(cli::SERVER_SUBCOMMAND).about(cli::SERVER_DESCRIPTION))
        .subcommand(
            Command::new(cli::INSERT_SUBCOMMAND)
                .about(cli::INSERT_DESCRIPTION)
                .arg(arg!(-u --uri <URI>).required(true).help("Bstore URI"))
                .subcommand(
                    Command::new(cli::FILE_SUBCOMMAND)
                        .about(cli::INSERT_FILE_DESCRIPTION)
                        .arg(
                            arg!(-f --file <FILE>)
                                .required(true)
                                .help("Path to file to insert"),
                        )
                        .arg(
                            arg!(-b --bucket <BUCKET>)
                                .required(true)
                                .help("Bucket to insert the file"),
                        )
                        .arg(
                            arg!(-n --name <NEW_FILE_NAME>)
                                .required(false)
                                .help("New file name if name should be different then name of downloadable file"),
                        ),
                ),
        )
        .subcommand(
            Command::new(cli::LIST_SUBCOMMAND)
                .about(cli::LIST_DESCRIPTION)
                .arg(arg!(-u --uri <URI>).required(true).help("Bstore URI"))
                .subcommand(
                    Command::new(cli::BUCKET_SUBCOMMAND).about(cli::BUCKET_LIST_DESCRIPTION),
                ),
        )
        .arg_required_else_help(true)
        .disable_version_flag(true)
        .get_matches();

    if cli.subcommand_matches(cli::BUGREPORT_SUBCOMMAND).is_some() {
        cli::bugreport::run();
    } else if let Some(server_matches) = cli.subcommand_matches(cli::SERVER_SUBCOMMAND) {
        cli::server::run(server_matches).await;
    } else if let Some(insert_matches) = cli.subcommand_matches(cli::INSERT_SUBCOMMAND) {
        let uri = insert_matches.get_one::<String>("uri").unwrap();
        if let Some(file_matches) = insert_matches.subcommand_matches(cli::FILE_SUBCOMMAND) {
            let file = file_matches.get_one::<String>("file").unwrap();
            let bucket = file_matches.get_one::<String>("bucket").unwrap();
            let new_file_name = file_matches.get_one::<String>("name");
            let params = FileParams {
                uri: uri.clone(),
                file: file.clone(),
                bucket: bucket.clone(),
                new_file_name: new_file_name.cloned(),
            };
            insert_single_file(params).await;
        }
    } else if let Some(insert_matches) = cli.subcommand_matches(cli::LIST_SUBCOMMAND) {
        let uri = insert_matches.get_one::<String>("uri").unwrap();
        if insert_matches
            .subcommand_matches(cli::BUCKET_SUBCOMMAND)
            .is_some()
        {
            list_buckets(uri).await;
        }
    }
}
