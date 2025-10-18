#![warn(clippy::unwrap_in_result)]
#![warn(clippy::unwrap_used)]

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Represents a storage bucket containing multiple files.
///
/// A bucket is a logical container that groups related files together.
/// Each bucket has a unique identifier and tracks the number of files it contains.
#[derive(Serialize, Deserialize, ToSchema)]
pub struct Bucket {
    /// Unique identifier for the bucket
    pub id: String,
    /// Total number of files stored in this bucket
    pub files_count: i64,
}

/// Represents a file stored in the system.
///
/// Contains metadata about a file including its location, parent bucket,
/// content hash for integrity verification, and size information.
#[derive(Serialize, Deserialize, ToSchema)]
pub struct File {
    /// Unique numeric identifier for the file
    pub id: i64,
    /// File path or location within the storage system
    pub path: String,
    /// Identifier of the bucket containing this file
    pub bucket: String,
    /// BLAKE3 cryptographic hash of the file content for integrity verification
    pub blake3_hash: String,
    /// Size of the file in bytes
    pub size: usize,
}

/// Result of a delete operation showing the number of items removed.
///
/// Provides statistics about what was deleted during a cleanup or removal operation,
/// distinguishing between file metadata records and actual blob storage.
#[derive(Serialize, Deserialize, Default, ToSchema)]
pub struct DeleteResult {
    /// Number of file metadata records deleted
    pub files: usize,
    /// Number of blob storage objects deleted
    pub blobs: usize,
}
