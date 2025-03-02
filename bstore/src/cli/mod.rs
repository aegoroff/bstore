pub mod client;
pub mod server;
pub mod bugreport;

pub const SERVER_SUBCOMMAND: &str = "server";
pub const SERVER_DESCRIPTION: &str = "Run the server";

pub const BUGREPORT_SUBCOMMAND: &str = "bugreport";
pub const BUGREPORT_DESCRIPTION: &str = "Collect information about the system and the environment that users can send along with a bug report";

pub const INSERT_SUBCOMMAND: &str = "insert";
pub const INSERT_DESCRIPTION: &str = "Bstore insert file(s) into store";

pub const FILE_SUBCOMMAND: &str = "file";
pub const INSERT_FILE_DESCRIPTION: &str = "Insert single file into store";

pub const LIST_SUBCOMMAND: &str = "list";
pub const LIST_DESCRIPTION: &str = "List objects in bstore";

pub const BUCKET_SUBCOMMAND: &str = "bucket";
pub const BUCKET_LIST_DESCRIPTION: &str = "List buckets in bstore";
