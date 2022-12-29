use std::env;
use serde::{Deserialize, Serialize};


#[derive(Debug, Clone, Serialize, Deserialize)]
struct VersionInfo {
    pub name: String,
    pub version: String,
    pub os: String,
    pub architecture: String,
}

pub fn run() {
    let info = VersionInfo {
        name: clap::crate_name!().to_string(),
        version: clap::crate_version!().to_string(),
        os: env::consts::OS.to_string(),
        architecture: env::consts::ARCH.to_string(),
    };

    println!("Name           : {}", info.name);
    println!("Version        : {}", info.version);
    println!("OS             : {}", info.os);
    println!("Architecture   : {}", info.architecture);
}