pub mod client;
pub mod server;
pub mod version;

pub const SERVER_SUBCOMMAND: &str = "server";
pub const SERVER_DESCRIPTION: &str = "Run the server";

pub const VERSION_SUBCOMMAND: &str = "version";
pub const VERSION_DESCRIPTION: &str = "Display the version and build information";

pub const INSERT_SUBCOMMAND: &str = "insert";
pub const INSERT_DESCRIPTION: &str = "Bstore insert file(s) into store";

pub const FILE_SUBCOMMAND: &str = "file";
pub const INSERT_FILE_DESCRIPTION: &str = "Insert single file into store";

pub const LIST_SUBCOMMAND: &str = "list";
pub const LIST_DESCRIPTION: &str = "List objects in bstore";

pub const BUCKET_SUBCOMMAND: &str = "bucket";
pub const BUCKET_LIST_DESCRIPTION: &str = "List buckets in bstore";
