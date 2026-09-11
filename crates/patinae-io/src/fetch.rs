//! Fetch molecular structures from RCSB PDB
//!
//! This module provides functionality to download molecular structures from the
//! RCSB Protein Data Bank by their PDB ID.
//!
//! # Features
//!
//! - `fetch` - Enables synchronous fetching using `ureq`
//! - `fetch-async` - Enables asynchronous fetching using `reqwest`
//!
//! # Example (synchronous)
//!
//! ```no_run
//! # #[cfg(feature = "fetch")]
//! # fn main() -> patinae_io::IoResult<()> {
//! use patinae_io::fetch::{fetch, FetchFormat};
//!
//! // Fetch structure in BinaryCIF format (default)
//! let mol = fetch("1ubq", FetchFormat::default())?;
//!
//! // Fetch in PDB format
//! let mol = fetch("4hhb", FetchFormat::Pdb)?;
//! # Ok(())
//! # }
//! # #[cfg(not(feature = "fetch"))]
//! # fn main() {}
//! ```
//!
//! # Example (asynchronous)
//!
//! ```no_run
//! # #[cfg(feature = "fetch-async")]
//! # async fn example() -> patinae_io::IoResult<()> {
//! use patinae_io::fetch::{fetch_async, FetchFormat};
//!
//! // Async fetch
//! let mol = fetch_async("1ubq", FetchFormat::Cif).await?;
//! # Ok(())
//! # }
//! ```

#[cfg(feature = "fetch-async")]
use std::error::Error;
#[cfg(feature = "fetch-async")]
use std::future::Future;
use std::io::Cursor;
#[cfg(feature = "fetch")]
use std::io::Read;
#[cfg(feature = "fetch-async")]
use std::pin::Pin;
#[cfg(feature = "fetch-async")]
use std::sync::OnceLock;
#[cfg(feature = "fetch-async")]
use std::time::Duration;

use crate::compress::gzip_reader;
use crate::error::{IoError, IoResult};
use patinae_mol::ObjectMolecule;

/// RCSB PDB base URL for text file downloads (PDB, mmCIF)
const RCSB_BASE_URL: &str = "https://files.rcsb.org/download";

/// RCSB models API base URL for BinaryCIF downloads
const RCSB_MODELS_URL: &str = "https://models.rcsb.org";

#[cfg(feature = "fetch-async")]
const RCSB_GRAPHQL_URL: &str = "https://data.rcsb.org/graphql";

#[cfg(feature = "fetch-async")]
const RCSB_METADATA_QUERY: &str = r#"
query structure($id: String!) {
  entry(entry_id: $id) {
    rcsb_id
    struct {
      title
    }
    rcsb_accession_info {
      deposit_date
      initial_release_date
    }
    exptl {
      method
    }
    rcsb_primary_citation {
      pdbx_database_id_DOI
    }
  }
}
"#;

/// User-Agent header for HTTP requests
const USER_AGENT: &str = concat!("patinae/", env!("CARGO_PKG_VERSION"));

#[cfg(feature = "fetch-async")]
const ASYNC_HTTP_TIMEOUT_SECS: u64 = 10;

#[cfg(feature = "fetch-async")]
fn ensure_rustls_crypto_provider() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// Download URL bytes with Patinae's TLS provider and request timeout.
///
/// # Errors
/// Returns an I/O error for client setup, network, HTTP status or body failures.
#[cfg(feature = "fetch-async")]
pub async fn fetch_url_bytes(url: &str, timeout: std::time::Duration) -> IoResult<Vec<u8>> {
    ensure_rustls_crypto_provider();
    let result = async {
        reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(timeout)
            .build()?
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await
    }
    .await;
    result
        .map(|bytes| bytes.to_vec())
        .map_err(|error| IoError::fetch(format_reqwest_error(error)))
}

#[cfg(feature = "fetch-async")]
fn format_reqwest_error(err: reqwest::Error) -> String {
    let mut message = err.to_string();
    let mut source = err.source();
    while let Some(err) = source {
        message.push_str(": ");
        message.push_str(&err.to_string());
        source = err.source();
    }
    message
}

/// Format to fetch from RCSB PDB
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FetchFormat {
    /// PDB format (.pdb)
    ///
    /// Traditional PDB format. Note that PDB format has limitations for
    /// large structures (>99,999 atoms or >9,999 residues).
    Pdb,

    /// mmCIF format (.cif)
    ///
    /// Macromolecular Crystallographic Information File format.
    /// Handles large structures and contains more complete information
    /// than PDB format.
    Cif,

    /// BinaryCIF format (.bcif) - default
    ///
    /// Binary encoding of mmCIF using MessagePack. Smallest file size
    /// and fastest parsing. The primary format served by RCSB PDB.
    #[default]
    Bcif,
}

#[cfg(feature = "fetch-async")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdbMetadataPreview {
    pub rcsb_id: String,
    pub title: String,
    pub deposit_date: Option<String>,
    pub release_date: Option<String>,
    pub methods: Vec<String>,
    pub doi: Option<String>,
}

impl FetchFormat {
    /// Get the file extension for this format
    pub fn extension(&self) -> &'static str {
        match self {
            FetchFormat::Pdb => "pdb",
            FetchFormat::Cif => "cif",
            FetchFormat::Bcif => "bcif",
        }
    }

    /// Get a human-readable name for this format
    pub fn name(&self) -> &'static str {
        match self {
            FetchFormat::Pdb => "PDB",
            FetchFormat::Cif => "mmCIF",
            FetchFormat::Bcif => "BinaryCIF",
        }
    }
}

/// Validate a PDB ID
///
/// PDB IDs are 4-character alphanumeric codes (e.g., "1ubq", "4hhb").
/// This function validates the format but does not check if the ID exists.
///
/// # Arguments
///
/// * `pdb_id` - The PDB ID to validate
///
/// # Returns
///
/// Returns `Ok(())` if valid, or an error if the PDB ID is invalid.
pub fn validate_pdb_id(pdb_id: &str) -> IoResult<()> {
    if pdb_id.len() != 4 {
        return Err(IoError::invalid_pdb_id(format!(
            "'{}' - must be exactly 4 characters",
            pdb_id
        )));
    }

    if !pdb_id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(IoError::invalid_pdb_id(format!(
            "'{}' - must contain only alphanumeric characters",
            pdb_id
        )));
    }

    Ok(())
}

/// Build the RCSB download URL for a PDB ID
///
/// # Arguments
///
/// * `pdb_id` - The 4-character PDB ID (case-insensitive)
/// * `format` - The format to download
///
/// # Returns
///
/// The full URL to download the structure file (gzip-compressed).
pub fn build_rcsb_url(pdb_id: &str, format: FetchFormat) -> String {
    // RCSB uses lowercase PDB IDs in URLs
    let pdb_id_lower = pdb_id.to_lowercase();
    let url = match format {
        FetchFormat::Bcif => RCSB_MODELS_URL,
        _ => RCSB_BASE_URL,
    };
    format!("{}/{}.{}.gz", url, pdb_id_lower, format.extension())
}

/// Parse the fetched content based on format
fn parse_content(
    content: &[u8],
    format: FetchFormat,
    bond_tolerance: f32,
) -> IoResult<ObjectMolecule> {
    let decoder = gzip_reader(Cursor::new(content));
    match format {
        FetchFormat::Pdb => crate::pdb::read_pdb_from_with_bond_tolerance(decoder, bond_tolerance),
        FetchFormat::Cif => crate::cif::read_cif_from_with_bond_tolerance(decoder, bond_tolerance),
        FetchFormat::Bcif => {
            crate::bcif::read_bcif_from_with_bond_tolerance(decoder, bond_tolerance)
        }
    }
}

#[cfg(feature = "fetch")]
struct SyncFetchResponse {
    status: u16,
    body: Box<dyn Read>,
}

#[cfg(feature = "fetch")]
trait SyncFetchTransport {
    fn get(&self, url: &str, user_agent: &str) -> Result<SyncFetchResponse, String>;
}

#[cfg(feature = "fetch")]
struct UreqFetchTransport;

#[cfg(feature = "fetch")]
impl SyncFetchTransport for UreqFetchTransport {
    fn get(&self, url: &str, user_agent: &str) -> Result<SyncFetchResponse, String> {
        let response = ureq::get(url)
            .header("User-Agent", user_agent)
            .call()
            .map_err(|e| format!("Network error: {}", e))?;
        Ok(SyncFetchResponse {
            status: response.status().as_u16(),
            body: Box::new(response.into_body().into_reader()),
        })
    }
}

#[cfg(feature = "fetch")]
fn read_sync_response_body(response: SyncFetchResponse) -> IoResult<Vec<u8>> {
    let mut content = Vec::new();
    let mut body = response.body;
    body.read_to_end(&mut content)
        .map_err(|e| IoError::fetch(format!("Failed to read response: {}", e)))?;
    Ok(content)
}

#[cfg(feature = "fetch")]
fn fetch_with_transport(
    pdb_id: &str,
    format: FetchFormat,
    bond_tolerance: f32,
    transport: &impl SyncFetchTransport,
) -> IoResult<ObjectMolecule> {
    validate_pdb_id(pdb_id)?;

    let url = build_rcsb_url(pdb_id, format);
    let response = transport.get(&url, USER_AGENT).map_err(IoError::fetch)?;

    if response.status == 404 {
        return Err(IoError::fetch(format!("PDB ID '{}' not found", pdb_id)));
    }
    if response.status >= 400 {
        return Err(IoError::fetch(format!(
            "HTTP error {}: {}",
            response.status, url
        )));
    }

    let content = read_sync_response_body(response)?;
    parse_content(&content, format, bond_tolerance)
}

#[cfg(feature = "fetch-async")]
trait AsyncFetchTransport {
    type Response;
    type GetFuture<'a>: Future<Output = Result<Self::Response, String>> + Send + 'a
    where
        Self: 'a;
    type BodyFuture: Future<Output = Result<Vec<u8>, String>> + Send;

    fn get<'a>(&'a self, url: &'a str, user_agent: &'a str) -> Self::GetFuture<'a>;
    fn status(response: &Self::Response) -> u16;
    fn read_body(response: Self::Response) -> Self::BodyFuture;
}

#[cfg(feature = "fetch-async")]
struct ReqwestFetchTransport;

#[cfg(feature = "fetch-async")]
impl AsyncFetchTransport for ReqwestFetchTransport {
    type Response = reqwest::Response;
    type GetFuture<'a> = Pin<Box<dyn Future<Output = Result<Self::Response, String>> + Send + 'a>>;
    type BodyFuture = Pin<Box<dyn Future<Output = Result<Vec<u8>, String>> + Send>>;

    fn get<'a>(&'a self, url: &'a str, user_agent: &'a str) -> Self::GetFuture<'a> {
        Box::pin(async move {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(ASYNC_HTTP_TIMEOUT_SECS))
                .build()
                .map_err(|e| format!("Failed to create HTTP client: {}", e))?;
            client
                .get(url)
                .header("User-Agent", user_agent)
                .send()
                .await
                .map_err(|e| format!("Network error: {}", format_reqwest_error(e)))
        })
    }

    fn status(response: &Self::Response) -> u16 {
        response.status().as_u16()
    }

    fn read_body(response: Self::Response) -> Self::BodyFuture {
        Box::pin(async move {
            response
                .bytes()
                .await
                .map(|bytes| bytes.to_vec())
                .map_err(|e| format!("Failed to read response: {}", format_reqwest_error(e)))
        })
    }
}

#[cfg(feature = "fetch-async")]
async fn fetch_async_with_transport<T: AsyncFetchTransport>(
    pdb_id: &str,
    format: FetchFormat,
    bond_tolerance: f32,
    transport: &T,
) -> IoResult<ObjectMolecule> {
    validate_pdb_id(pdb_id)?;
    ensure_rustls_crypto_provider();

    let url = build_rcsb_url(pdb_id, format);
    let response = transport
        .get(&url, USER_AGENT)
        .await
        .map_err(IoError::fetch)?;

    let status = T::status(&response);
    if status == 404 {
        return Err(IoError::fetch(format!("PDB ID '{}' not found", pdb_id)));
    }
    if !(200..300).contains(&status) {
        return Err(IoError::fetch(format!("HTTP error {}: {}", status, url)));
    }

    let content = T::read_body(response).await.map_err(IoError::fetch)?;
    parse_content(&content, format, bond_tolerance)
}

/// Fetch a molecular structure from RCSB PDB (synchronous)
///
/// Downloads and parses a molecular structure from the RCSB Protein Data Bank.
///
/// # Arguments
///
/// * `pdb_id` - The 4-character PDB ID (e.g., "1ubq", "4hhb"). Case-insensitive.
/// * `format` - The format to fetch ([`FetchFormat::Pdb`] or [`FetchFormat::Cif`])
///
/// # Returns
///
/// The parsed molecular structure, or an error if the fetch or parse failed.
///
/// # Example
///
/// ```no_run
/// # #[cfg(feature = "fetch")]
/// # fn main() -> patinae_io::IoResult<()> {
/// use patinae_io::fetch::{fetch, FetchFormat};
///
/// // Fetch ubiquitin in mmCIF format
/// let mol = fetch("1ubq", FetchFormat::Cif)?;
/// println!("Fetched {} atoms", mol.atom_count());
/// # Ok(())
/// # }
/// # #[cfg(not(feature = "fetch"))]
/// # fn main() {}
/// ```
///
/// # Errors
///
/// Returns an error if:
/// - The PDB ID is invalid (not 4 alphanumeric characters)
/// - The network request fails
/// - The PDB ID does not exist (404 error)
/// - The downloaded content cannot be parsed
#[cfg(feature = "fetch")]
pub fn fetch(pdb_id: &str, format: FetchFormat) -> IoResult<ObjectMolecule> {
    fetch_with_bond_tolerance(pdb_id, format, patinae_mol::DEFAULT_BOND_TOLERANCE)
}

#[cfg(feature = "fetch")]
pub fn fetch_with_bond_tolerance(
    pdb_id: &str,
    format: FetchFormat,
    bond_tolerance: f32,
) -> IoResult<ObjectMolecule> {
    fetch_with_transport(pdb_id, format, bond_tolerance, &UreqFetchTransport)
}

/// Fetch a molecular structure from RCSB PDB (asynchronous)
///
/// Downloads and parses a molecular structure from the RCSB Protein Data Bank.
///
/// # Arguments
///
/// * `pdb_id` - The 4-character PDB ID (e.g., "1ubq", "4hhb"). Case-insensitive.
/// * `format` - The format to fetch ([`FetchFormat::Pdb`] or [`FetchFormat::Cif`])
///
/// # Returns
///
/// The parsed molecular structure, or an error if the fetch or parse failed.
///
/// # Example
///
/// ```no_run
/// # #[cfg(feature = "fetch-async")]
/// # async fn example() -> patinae_io::IoResult<()> {
/// use patinae_io::fetch::{fetch_async, FetchFormat};
///
/// // Fetch hemoglobin in PDB format
/// let mol = fetch_async("4hhb", FetchFormat::Pdb).await?;
/// println!("Fetched {} atoms", mol.atom_count());
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns an error if:
/// - The PDB ID is invalid (not 4 alphanumeric characters)
/// - The network request fails
/// - The PDB ID does not exist (404 error)
/// - The downloaded content cannot be parsed
#[cfg(feature = "fetch-async")]
pub async fn fetch_async(pdb_id: &str, format: FetchFormat) -> IoResult<ObjectMolecule> {
    fetch_async_with_bond_tolerance(pdb_id, format, patinae_mol::DEFAULT_BOND_TOLERANCE).await
}

#[cfg(feature = "fetch-async")]
pub async fn fetch_async_with_bond_tolerance(
    pdb_id: &str,
    format: FetchFormat,
    bond_tolerance: f32,
) -> IoResult<ObjectMolecule> {
    fetch_async_with_transport(pdb_id, format, bond_tolerance, &ReqwestFetchTransport).await
}

#[cfg(feature = "fetch-async")]
pub async fn fetch_pdb_metadata(pdb_id: &str) -> IoResult<PdbMetadataPreview> {
    validate_pdb_id(pdb_id)?;
    ensure_rustls_crypto_provider();

    let pdb_id = pdb_id.to_ascii_uppercase();
    let body = serde_json::json!({
        "query": RCSB_METADATA_QUERY,
        "variables": { "id": pdb_id },
    });
    let body = serde_json::to_vec(&body)
        .map_err(|err| IoError::fetch(format!("Failed to encode metadata request: {err}")))?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(ASYNC_HTTP_TIMEOUT_SECS))
        .build()
        .map_err(|err| IoError::fetch(format!("Failed to create HTTP client: {err}")))?;

    let response = client
        .post(RCSB_GRAPHQL_URL)
        .header("User-Agent", USER_AGENT)
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .await
        .map_err(|err| IoError::fetch(format!("Network error: {}", format_reqwest_error(err))))?;

    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(IoError::fetch(format!(
            "metadata HTTP error {}: {}",
            status, RCSB_GRAPHQL_URL
        )));
    }

    let bytes = response.bytes().await.map_err(|err| {
        IoError::fetch(format!(
            "Failed to read response: {}",
            format_reqwest_error(err)
        ))
    })?;
    let json: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|err| IoError::parse_msg(format!("Invalid metadata response: {err}")))?;

    parse_pdb_metadata_json(&json)
}

#[cfg(feature = "fetch-async")]
fn parse_pdb_metadata_json(json: &serde_json::Value) -> IoResult<PdbMetadataPreview> {
    if let Some(message) = json
        .pointer("/errors/0/message")
        .and_then(serde_json::Value::as_str)
    {
        return Err(IoError::fetch(message.to_string()));
    }

    let entry = json
        .pointer("/data/entry")
        .ok_or_else(|| IoError::fetch("metadata response did not contain an entry"))?;
    if entry.is_null() {
        return Err(IoError::fetch("PDB ID not found"));
    }

    let rcsb_id = entry
        .pointer("/rcsb_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string();
    let title = entry
        .pointer("/struct/title")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("(no title)")
        .to_string();
    let deposit_date = entry
        .pointer("/rcsb_accession_info/deposit_date")
        .and_then(serde_json::Value::as_str)
        .map(trim_iso_date);
    let release_date = entry
        .pointer("/rcsb_accession_info/initial_release_date")
        .and_then(serde_json::Value::as_str)
        .map(trim_iso_date);
    let doi = entry
        .pointer("/rcsb_primary_citation/pdbx_database_id_DOI")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let methods = entry
        .pointer("/exptl")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.pointer("/method").and_then(serde_json::Value::as_str))
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(PdbMetadataPreview {
        rcsb_id,
        title,
        deposit_date,
        release_date,
        methods,
        doi,
    })
}

#[cfg(feature = "fetch-async")]
fn trim_iso_date(value: &str) -> String {
    value.get(..10).unwrap_or(value).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(feature = "fetch", feature = "fetch-async"))]
    fn assert_fetch_message(err: IoError, expected: &str) {
        assert!(err.is_fetch(), "expected fetch error, got {:?}", err);
        assert!(
            err.message()
                .is_some_and(|message| message.contains(expected)),
            "expected message containing {expected:?}, got {:?}",
            err.message()
        );
    }

    #[cfg(feature = "fetch")]
    #[derive(Clone, Copy)]
    enum FakeSyncBody {
        Bytes(&'static [u8]),
        ReadError,
    }

    #[cfg(feature = "fetch")]
    #[derive(Clone, Copy)]
    struct FakeSyncTransport {
        result: Result<(u16, FakeSyncBody), &'static str>,
    }

    #[cfg(feature = "fetch")]
    impl SyncFetchTransport for FakeSyncTransport {
        fn get(&self, _url: &str, _user_agent: &str) -> Result<SyncFetchResponse, String> {
            let (status, body) = self.result.map_err(str::to_string)?;
            let body: Box<dyn Read> = match body {
                FakeSyncBody::Bytes(bytes) => Box::new(Cursor::new(bytes.to_vec())),
                FakeSyncBody::ReadError => Box::new(FailingReader),
            };
            Ok(SyncFetchResponse { status, body })
        }
    }

    #[cfg(feature = "fetch")]
    struct FailingReader;

    #[cfg(feature = "fetch")]
    impl Read for FailingReader {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "fake body read failure",
            ))
        }
    }

    #[cfg(feature = "fetch")]
    struct PanicSyncTransport;

    #[cfg(feature = "fetch")]
    impl SyncFetchTransport for PanicSyncTransport {
        fn get(&self, _url: &str, _user_agent: &str) -> Result<SyncFetchResponse, String> {
            panic!("transport should not be called")
        }
    }

    #[cfg(feature = "fetch-async")]
    #[derive(Clone, Copy)]
    enum FakeAsyncBody {
        Bytes(&'static [u8]),
        ReadError,
    }

    #[cfg(feature = "fetch-async")]
    #[derive(Clone, Copy)]
    struct FakeAsyncResponse {
        status: u16,
        body: FakeAsyncBody,
    }

    #[cfg(feature = "fetch-async")]
    #[derive(Clone, Copy)]
    struct FakeAsyncTransport {
        result: Result<FakeAsyncResponse, &'static str>,
    }

    #[cfg(feature = "fetch-async")]
    impl AsyncFetchTransport for FakeAsyncTransport {
        type Response = FakeAsyncResponse;
        type GetFuture<'a> = std::future::Ready<Result<Self::Response, String>>;
        type BodyFuture = std::future::Ready<Result<Vec<u8>, String>>;

        fn get<'a>(&'a self, _url: &'a str, _user_agent: &'a str) -> Self::GetFuture<'a> {
            std::future::ready(self.result.map_err(str::to_string))
        }

        fn status(response: &Self::Response) -> u16 {
            response.status
        }

        fn read_body(response: Self::Response) -> Self::BodyFuture {
            std::future::ready(match response.body {
                FakeAsyncBody::Bytes(bytes) => Ok(bytes.to_vec()),
                FakeAsyncBody::ReadError => {
                    Err("Failed to read response: fake body read failure".to_string())
                }
            })
        }
    }

    #[cfg(feature = "fetch-async")]
    struct PanicAsyncTransport;

    #[cfg(feature = "fetch-async")]
    impl AsyncFetchTransport for PanicAsyncTransport {
        type Response = FakeAsyncResponse;
        type GetFuture<'a> = std::future::Ready<Result<Self::Response, String>>;
        type BodyFuture = std::future::Ready<Result<Vec<u8>, String>>;

        fn get<'a>(&'a self, _url: &'a str, _user_agent: &'a str) -> Self::GetFuture<'a> {
            panic!("transport should not be called")
        }

        fn status(response: &Self::Response) -> u16 {
            response.status
        }

        fn read_body(response: Self::Response) -> Self::BodyFuture {
            std::future::ready(match response.body {
                FakeAsyncBody::Bytes(bytes) => Ok(bytes.to_vec()),
                FakeAsyncBody::ReadError => Err("fake body read failure".to_string()),
            })
        }
    }

    #[test]
    fn test_validate_pdb_id_valid() {
        assert!(validate_pdb_id("1ubq").is_ok());
        assert!(validate_pdb_id("4HHB").is_ok());
        assert!(validate_pdb_id("1A2B").is_ok());
        assert!(validate_pdb_id("9xyz").is_ok());
    }

    #[cfg(feature = "fetch-async")]
    #[test]
    fn parse_pdb_metadata_json_extracts_preview_fields() {
        let json = serde_json::json!({
            "data": {
                "entry": {
                    "rcsb_id": "1UBQ",
                    "struct": { "title": "Ubiquitin" },
                    "rcsb_accession_info": {
                        "deposit_date": "1987-01-02T00:00:00Z",
                        "initial_release_date": "1987-03-03T00:00:00Z"
                    },
                    "exptl": [
                        { "method": "X-RAY DIFFRACTION" },
                        { "method": "NEUTRON DIFFRACTION" }
                    ],
                    "rcsb_primary_citation": {
                        "pdbx_database_id_DOI": "10.0000/example"
                    }
                }
            }
        });

        let preview = parse_pdb_metadata_json(&json).unwrap();

        assert_eq!(preview.rcsb_id, "1UBQ");
        assert_eq!(preview.title, "Ubiquitin");
        assert_eq!(preview.deposit_date.as_deref(), Some("1987-01-02"));
        assert_eq!(preview.release_date.as_deref(), Some("1987-03-03"));
        assert_eq!(
            preview.methods,
            vec![
                "X-RAY DIFFRACTION".to_string(),
                "NEUTRON DIFFRACTION".to_string()
            ]
        );
        assert_eq!(preview.doi.as_deref(), Some("10.0000/example"));
    }

    #[cfg(feature = "fetch-async")]
    #[test]
    fn parse_pdb_metadata_json_reports_missing_entry() {
        let json = serde_json::json!({ "data": { "entry": null } });

        let error = parse_pdb_metadata_json(&json).unwrap_err();

        assert_fetch_message(error, "PDB ID not found");
    }

    #[test]
    fn test_validate_pdb_id_invalid_length() {
        assert!(validate_pdb_id("").is_err());
        assert!(validate_pdb_id("1ub").is_err());
        assert!(validate_pdb_id("1ubqx").is_err());
        assert!(validate_pdb_id("12345").is_err());
    }

    #[test]
    fn test_validate_pdb_id_invalid_chars() {
        assert!(validate_pdb_id("1ub!").is_err());
        assert!(validate_pdb_id("1u-q").is_err());
        assert!(validate_pdb_id("1u q").is_err());
        assert!(validate_pdb_id("1u.q").is_err());
    }

    #[test]
    fn test_build_rcsb_url() {
        assert_eq!(
            build_rcsb_url("1UBQ", FetchFormat::Pdb),
            "https://files.rcsb.org/download/1ubq.pdb.gz"
        );
        assert_eq!(
            build_rcsb_url("4hhb", FetchFormat::Cif),
            "https://files.rcsb.org/download/4hhb.cif.gz"
        );
        assert_eq!(
            build_rcsb_url("1UBQ", FetchFormat::Bcif),
            "https://models.rcsb.org/1ubq.bcif.gz"
        );
    }

    #[test]
    fn test_fetch_format_extension() {
        assert_eq!(FetchFormat::Pdb.extension(), "pdb");
        assert_eq!(FetchFormat::Cif.extension(), "cif");
        assert_eq!(FetchFormat::Bcif.extension(), "bcif");
    }

    #[test]
    fn test_fetch_format_name() {
        assert_eq!(FetchFormat::Pdb.name(), "PDB");
        assert_eq!(FetchFormat::Cif.name(), "mmCIF");
        assert_eq!(FetchFormat::Bcif.name(), "BinaryCIF");
    }

    #[test]
    fn test_fetch_format_default() {
        assert_eq!(FetchFormat::default(), FetchFormat::Bcif);
    }

    // Integration tests - require network access
    // Run with: cargo test -p patinae-io --features fetch -- --ignored

    #[cfg(feature = "fetch")]
    #[test]
    #[ignore = "requires network access"]
    fn test_fetch_pdb_format() {
        let mol = fetch("1ubq", FetchFormat::Pdb).expect("Failed to fetch 1ubq in PDB format");
        // Ubiquitin has 76 residues, ~600 atoms
        assert!(
            mol.atom_count() > 500,
            "Expected >500 atoms, got {}",
            mol.atom_count()
        );
    }

    #[cfg(feature = "fetch")]
    #[test]
    #[ignore = "requires network access"]
    fn test_fetch_cif_format() {
        let mol = fetch("1ubq", FetchFormat::Cif).expect("Failed to fetch 1ubq in mmCIF format");
        // Ubiquitin has 76 residues, ~600 atoms
        assert!(
            mol.atom_count() > 500,
            "Expected >500 atoms, got {}",
            mol.atom_count()
        );
    }

    #[cfg(feature = "fetch")]
    #[test]
    #[ignore = "requires network access"]
    fn test_fetch_bcif_format() {
        let mol = fetch("1ubq", FetchFormat::Bcif).expect("Failed to fetch 1ubq in bCIF format");
        // Ubiquitin has 76 residues, ~600 atoms
        assert!(
            mol.atom_count() > 500,
            "Expected >500 atoms, got {}",
            mol.atom_count()
        );
    }

    #[cfg(feature = "fetch")]
    #[test]
    #[ignore = "requires network access"]
    fn test_fetch_nonexistent_pdb() {
        let result = fetch("9999", FetchFormat::Pdb);
        assert!(result.is_err(), "Expected error for nonexistent PDB ID");
        let err = result.unwrap_err();
        assert!(err.is_fetch(), "Expected Fetch error, got {:?}", err);
    }

    #[cfg(feature = "fetch")]
    #[test]
    fn test_fetch_invalid_pdb_id() {
        let result = fetch("invalid", FetchFormat::Pdb);
        assert!(result.is_err(), "Expected error for invalid PDB ID");
        let err = result.unwrap_err();
        assert!(
            err.is_invalid_pdb_id(),
            "Expected InvalidPdbId error, got {:?}",
            err
        );
    }

    #[cfg(feature = "fetch")]
    #[test]
    fn sync_fetch_rejects_invalid_id_before_network() {
        let result = fetch_with_transport(
            "invalid",
            FetchFormat::Pdb,
            patinae_mol::DEFAULT_BOND_TOLERANCE,
            &PanicSyncTransport,
        );

        assert!(result.unwrap_err().is_invalid_pdb_id());
    }

    #[cfg(feature = "fetch")]
    #[test]
    fn sync_fetch_reports_network_error() {
        let err = fetch_with_transport(
            "1ubq",
            FetchFormat::Pdb,
            patinae_mol::DEFAULT_BOND_TOLERANCE,
            &FakeSyncTransport {
                result: Err("Network error: offline"),
            },
        )
        .expect_err("network error should fail");

        assert_fetch_message(err, "Network error: offline");
    }

    #[cfg(feature = "fetch")]
    #[test]
    fn sync_fetch_reports_not_found() {
        let err = fetch_with_transport(
            "1ubq",
            FetchFormat::Pdb,
            patinae_mol::DEFAULT_BOND_TOLERANCE,
            &FakeSyncTransport {
                result: Ok((404, FakeSyncBody::Bytes(b""))),
            },
        )
        .expect_err("404 should fail");

        assert_fetch_message(err, "PDB ID '1ubq' not found");
    }

    #[cfg(feature = "fetch")]
    #[test]
    fn sync_fetch_reports_http_error() {
        let err = fetch_with_transport(
            "1ubq",
            FetchFormat::Pdb,
            patinae_mol::DEFAULT_BOND_TOLERANCE,
            &FakeSyncTransport {
                result: Ok((500, FakeSyncBody::Bytes(b""))),
            },
        )
        .expect_err("HTTP error should fail");

        assert_fetch_message(err, "HTTP error 500");
    }

    #[cfg(feature = "fetch")]
    #[test]
    fn sync_fetch_reports_body_read_error() {
        let err = fetch_with_transport(
            "1ubq",
            FetchFormat::Pdb,
            patinae_mol::DEFAULT_BOND_TOLERANCE,
            &FakeSyncTransport {
                result: Ok((200, FakeSyncBody::ReadError)),
            },
        )
        .expect_err("body read error should fail");

        assert_fetch_message(err, "Failed to read response: fake body read failure");
    }

    // Async integration tests
    #[cfg(feature = "fetch-async")]
    #[tokio::test]
    #[ignore = "requires network access"]
    async fn test_fetch_async_pdb_format() {
        let mol = fetch_async("1ubq", FetchFormat::Pdb)
            .await
            .expect("Failed to fetch 1ubq in PDB format");
        assert!(
            mol.atom_count() > 500,
            "Expected >500 atoms, got {}",
            mol.atom_count()
        );
    }

    #[cfg(feature = "fetch-async")]
    #[tokio::test]
    #[ignore = "requires network access"]
    async fn test_fetch_async_cif_format() {
        let mol = fetch_async("1ubq", FetchFormat::Cif)
            .await
            .expect("Failed to fetch 1ubq in mmCIF format");
        assert!(
            mol.atom_count() > 500,
            "Expected >500 atoms, got {}",
            mol.atom_count()
        );
    }

    #[cfg(feature = "fetch-async")]
    #[tokio::test]
    #[ignore = "requires network access"]
    async fn test_fetch_async_bcif_format() {
        let mol = fetch_async("1ubq", FetchFormat::Bcif)
            .await
            .expect("Failed to fetch 1ubq in bCIF format");
        assert!(
            mol.atom_count() > 500,
            "Expected >500 atoms, got {}",
            mol.atom_count()
        );
    }

    #[cfg(feature = "fetch-async")]
    #[tokio::test]
    #[ignore = "requires network access"]
    async fn test_fetch_async_nonexistent_pdb() {
        let result = fetch_async("9999", FetchFormat::Pdb).await;
        assert!(result.is_err(), "Expected error for nonexistent PDB ID");
        let err = result.unwrap_err();
        assert!(err.is_fetch(), "Expected Fetch error, got {:?}", err);
    }

    #[cfg(feature = "fetch-async")]
    #[tokio::test]
    async fn test_fetch_async_invalid_pdb_id() {
        let result = fetch_async("invalid", FetchFormat::Pdb).await;
        assert!(result.is_err(), "Expected error for invalid PDB ID");
        let err = result.unwrap_err();
        assert!(
            err.is_invalid_pdb_id(),
            "Expected InvalidPdbId error, got {:?}",
            err
        );
    }

    #[cfg(feature = "fetch-async")]
    #[tokio::test]
    async fn async_fetch_rejects_invalid_id_before_network() {
        let result = fetch_async_with_transport(
            "invalid",
            FetchFormat::Pdb,
            patinae_mol::DEFAULT_BOND_TOLERANCE,
            &PanicAsyncTransport,
        )
        .await;

        assert!(result.unwrap_err().is_invalid_pdb_id());
    }

    #[cfg(feature = "fetch-async")]
    #[tokio::test]
    async fn async_fetch_reports_network_error() {
        let err = fetch_async_with_transport(
            "1ubq",
            FetchFormat::Pdb,
            patinae_mol::DEFAULT_BOND_TOLERANCE,
            &FakeAsyncTransport {
                result: Err("Network error: offline"),
            },
        )
        .await
        .expect_err("network error should fail");

        assert_fetch_message(err, "Network error: offline");
    }

    #[cfg(feature = "fetch-async")]
    #[tokio::test]
    async fn async_fetch_reports_not_found() {
        let err = fetch_async_with_transport(
            "1ubq",
            FetchFormat::Pdb,
            patinae_mol::DEFAULT_BOND_TOLERANCE,
            &FakeAsyncTransport {
                result: Ok(FakeAsyncResponse {
                    status: 404,
                    body: FakeAsyncBody::Bytes(b""),
                }),
            },
        )
        .await
        .expect_err("404 should fail");

        assert_fetch_message(err, "PDB ID '1ubq' not found");
    }

    #[cfg(feature = "fetch-async")]
    #[tokio::test]
    async fn async_fetch_reports_http_error() {
        let err = fetch_async_with_transport(
            "1ubq",
            FetchFormat::Pdb,
            patinae_mol::DEFAULT_BOND_TOLERANCE,
            &FakeAsyncTransport {
                result: Ok(FakeAsyncResponse {
                    status: 500,
                    body: FakeAsyncBody::Bytes(b""),
                }),
            },
        )
        .await
        .expect_err("HTTP error should fail");

        assert_fetch_message(err, "HTTP error 500");
    }

    #[cfg(feature = "fetch-async")]
    #[tokio::test]
    async fn async_fetch_reports_body_read_error() {
        let err = fetch_async_with_transport(
            "1ubq",
            FetchFormat::Pdb,
            patinae_mol::DEFAULT_BOND_TOLERANCE,
            &FakeAsyncTransport {
                result: Ok(FakeAsyncResponse {
                    status: 200,
                    body: FakeAsyncBody::ReadError,
                }),
            },
        )
        .await
        .expect_err("body read error should fail");

        assert_fetch_message(err, "Failed to read response: fake body read failure");
    }
}
