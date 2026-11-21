//! The plugin registry over HTTPS: a [`RegistrySource`] for the signed
//! documents and the package download, on one shared reqwest client
//! (rustls, the stack of the `deckpilot-net` transport).
//!
//! Nothing here decides what is trusted. Every document goes to
//! `RegistryClient` (signatures, the key set chain, rollback and freeze
//! checks, pins) exactly as from a local directory, and a package is used
//! only if its bytes are the size and sha256 the verified detail pins.
//! Where a file was hosted never counts. This module only keeps the
//! transport bounded:
//!
//! - **https only**, for the configured URLs, every request and every
//!   redirect (the client refuses plain http, too);
//! - **redirects** are followed one hop at a time, at most
//!   [`MAX_REDIRECTS`], and only within the origin of the request
//!   (scheme, host, port). A package from the pinned GitHub release URL
//!   may also be redirected to GitHub's asset hosts
//!   ([`GITHUB_ASSET_HOSTS`]), as GitHub serves release assets; nowhere
//!   else. A redirect anywhere else is refused, not followed;
//! - **sizes**: each document stops at the client's limit for it, a
//!   package at its pinned size; a larger `Content-Length` is refused
//!   before the body is read;
//! - **timeouts**: connect 10 s, 20 s without data, 30 s per document and
//!   120 s per package in all;
//! - **errors**: no answer, a TLS failure, a 5xx or a body cut off is
//!   [`FetchError::Unavailable`] (the client then works offline from its
//!   verified state); a refused redirect, an oversized answer or an
//!   unexpected status is [`FetchError::Refused`] (an error, never
//!   "offline").
//!
//! Downloads stay in memory (a package is at most 5 MiB) and never touch
//! the disk before they verified, so a partial or corrupt download leaves
//! nothing behind; the store then stages and installs the verified bytes.

use std::sync::Mutex;
use std::time::Duration;

use deckpilot_plugin_manifest::{MAX_PACKAGE_BYTES, PackageDigest};
use deckpilot_plugin_package::sha256;
use deckpilot_plugin_registry::{DOWNLOAD_PREFIX, FetchError, RegistrySource};
use url::Url;

/// Longest wait for a TCP and TLS connection.
pub(super) const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Longest wait for the next bytes of an answer.
pub(super) const READ_TIMEOUT: Duration = Duration::from_secs(20);
/// Longest a registry document may take in all (redirects included).
pub(super) const DOCUMENT_TIMEOUT: Duration = Duration::from_secs(30);
/// Longest a package download may take in all.
pub(super) const PACKAGE_TIMEOUT: Duration = Duration::from_secs(120);
/// Most redirects followed for one file.
pub(super) const MAX_REDIRECTS: u32 = 5;
/// Where GitHub redirects release asset downloads. Only a package fetched
/// from the pinned release URL itself may be redirected there.
pub(super) const GITHUB_ASSET_HOSTS: &[&str] = &[
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];

/// Which certificate authorities the client trusts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TlsRoots {
    /// The operating system's (rustls-platform-verifier).
    System,
    /// Only these (PEM): a test's throwaway CA (`test-hooks` and unit
    /// tests only).
    #[cfg_attr(not(any(test, feature = "test-hooks")), allow(dead_code))]
    Only(Vec<String>),
}

/// Where the registry is.
#[derive(Debug, Clone)]
pub(super) struct HttpsConfig {
    /// The site: documents are at `<site>v1/<path>` (the publishing tool's
    /// `site/` directory).
    pub site: Url,
    /// Where packages are downloaded from instead of the pinned release
    /// URL: `<releases><id>--v<version>/<file>` (the tool's `releases/`
    /// directory). `None`: the pinned URL itself.
    pub releases: Option<Url>,
    /// Hosts a package download may be redirected to besides its own
    /// origin.
    pub package_redirect_hosts: Vec<String>,
    pub roots: TlsRoots,
}

impl HttpsConfig {
    /// From the configured URLs (both must be https).
    pub fn new(site: &str, releases: Option<&str>, roots: TlsRoots) -> Result<Self, String> {
        let site = https_base(site, "the plugin registry URL")?;
        let releases = releases
            .map(|r| https_base(r, "the plugin registry releases URL"))
            .transpose()?;
        // GitHub's asset hosts only for the pinned GitHub URL.
        let package_redirect_hosts = if releases.is_none() {
            GITHUB_ASSET_HOSTS.iter().map(|h| (*h).to_owned()).collect()
        } else {
            Vec::new()
        };
        Ok(Self {
            site,
            releases,
            package_redirect_hosts,
            roots,
        })
    }
}

/// `text` as the base URL of a directory: https, a host, no credentials,
/// query or fragment; a final `/` is added.
pub(super) fn https_base(text: &str, what: &str) -> Result<Url, String> {
    let mut url = Url::parse(text).map_err(|e| format!("{what} `{text}` is not a URL: {e}"))?;
    if url.scheme() != "https" {
        return Err(format!("{what} must be an https URL (not `{text}`)"));
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(format!("{what} `{text}` has no host"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(format!("{what} must not contain credentials"));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(format!("{what} must not have a query or fragment"));
    }
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

/// A relative path of plain segments (`index/3.json`): letters, digits,
/// `.`, `-`, `_`; no empty, `.` or `..` segment. Nothing that `Url::join`
/// could read as a scheme, host, query or escape.
fn plain_path(path: &str) -> Option<&str> {
    let ok = !path.is_empty()
        && path.split('/').all(|segment| {
            !segment.is_empty()
                && segment != "."
                && segment != ".."
                && segment
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        });
    ok.then_some(path)
}

fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port_or_known_default() == b.port_or_known_default()
}

/// A URL for messages: origin and path, never a query.
fn shown(url: &Url) -> String {
    format!("{}{}", url.origin().ascii_serialization(), url.path())
}

/// The error and its sources, without the URL.
fn describe(error: reqwest::Error) -> String {
    let timeout = error.is_timeout();
    let error = error.without_url();
    let mut text = error.to_string();
    let mut source = std::error::Error::source(&error);
    while let Some(inner) = source {
        let part = inner.to_string();
        if !text.contains(&part) {
            text.push_str(": ");
            text.push_str(&part);
        }
        source = inner.source();
    }
    if timeout {
        format!("timed out ({text})")
    } else {
        text
    }
}

/// Why a package could not be downloaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DownloadError {
    /// Not reachable, timed out, cut off: try again later.
    Unavailable(String),
    /// A refused redirect, status or URL.
    Refused(String),
    /// Bytes that are not the pinned package (size or sha256).
    Corrupt(String),
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(m) | Self::Refused(m) | Self::Corrupt(m) => f.write_str(m),
        }
    }
}

/// Which redirects a request may follow.
#[derive(Debug, Clone, Copy)]
enum Redirects<'a> {
    SameOrigin,
    /// The same origin, or one of these hosts (https).
    OrHosts(&'a [String]),
}

/// The registry's HTTPS client. One per registry: one connection pool and
/// one small runtime, used from the plugin worker thread only.
pub(super) struct HttpsSource {
    runtime: tokio::runtime::Runtime,
    client: reqwest::Client,
    config: HttpsConfig,
    documents: Url,
    /// Why the last document fetch found the registry unreachable.
    unavailable: Mutex<Option<String>>,
}

impl std::fmt::Debug for HttpsSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpsSource")
            .field("site", &self.config.site.as_str())
            .finish_non_exhaustive()
    }
}

impl HttpsSource {
    pub fn new(config: HttpsConfig) -> Result<Self, String> {
        // rustls' cryptography (as deckpilot-net); a no-op once installed.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("the registry client cannot start: {e}"))?;
        let mut builder = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
            .user_agent(concat!("DeckPilot/", env!("CARGO_PKG_VERSION")));
        if let TlsRoots::Only(pems) = &config.roots {
            let certs = pems
                .iter()
                .map(|pem| reqwest::Certificate::from_pem(pem.as_bytes()))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("a registry CA certificate is invalid: {}", describe(e)))?;
            builder = builder.tls_certs_only(certs);
        }
        let client = builder
            .build()
            .map_err(|e| format!("the registry client cannot start: {}", describe(e)))?;
        let documents = config
            .site
            .join("v1/")
            .map_err(|e| format!("the plugin registry URL: {e}"))?;
        Ok(Self {
            runtime,
            client,
            config,
            documents,
            unavailable: Mutex::new(None),
        })
    }

    /// Shown to the user and recorded as the install source.
    pub fn name(&self) -> String {
        shown(&self.config.site)
    }

    /// Why the last document fetch found the registry unreachable (taken).
    pub fn take_unavailable(&self) -> Option<String> {
        self.unavailable.lock().ok().and_then(|mut u| u.take())
    }

    /// The pinned package at `pinned` (a URL under the registry's fixed
    /// release prefix), from the configured releases URL or the pinned URL
    /// itself: exactly `digest.size` bytes with `digest.sha256`.
    pub fn package(&self, pinned: &str, digest: &PackageDigest) -> Result<Vec<u8>, DownloadError> {
        let relative = pinned
            .strip_prefix(DOWNLOAD_PREFIX)
            .and_then(plain_path)
            .ok_or_else(|| {
                DownloadError::Refused(format!("{pinned} is not a release asset of this registry"))
            })?;
        if digest.size == 0 || digest.size > MAX_PACKAGE_BYTES {
            return Err(DownloadError::Refused(format!(
                "the pinned package size {} is out of range",
                digest.size
            )));
        }
        let base = match &self.config.releases {
            Some(releases) => releases.clone(),
            None => {
                Url::parse(DOWNLOAD_PREFIX).map_err(|e| DownloadError::Refused(e.to_string()))?
            }
        };
        let url = base
            .join(relative)
            .map_err(|e| DownloadError::Refused(format!("{relative}: {e}")))?;
        let bytes = self
            .get(
                &url,
                digest.size,
                Redirects::OrHosts(&self.config.package_redirect_hosts),
                PACKAGE_TIMEOUT,
            )
            .map_err(|e| match e {
                FetchError::NotFound => {
                    DownloadError::Unavailable(format!("{} was not found", shown(&url)))
                }
                FetchError::Unavailable(m) => DownloadError::Unavailable(format!(
                    "{} could not be downloaded: {m}",
                    shown(&url)
                )),
                FetchError::Refused(m) => {
                    DownloadError::Refused(format!("{} was refused: {m}", shown(&url)))
                }
            })?;
        let length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if length != digest.size {
            return Err(DownloadError::Corrupt(format!(
                "the download of {} is incomplete: {length} of {} bytes",
                shown(&url),
                digest.size
            )));
        }
        let actual = sha256(&bytes);
        if actual != digest.sha256 {
            return Err(DownloadError::Corrupt(format!(
                "the download of {} is not the pinned package: sha256 {actual}, pinned {}",
                shown(&url),
                digest.sha256
            )));
        }
        tracing::info!(
            url = %shown(&url),
            bytes = length,
            sha256 = %actual,
            "plugin package downloaded; its size and sha256 are the pinned ones"
        );
        Ok(bytes)
    }

    /// The publisher signature file of the key `key_id` published next to
    /// the pinned package `pinned` (`<package>.<KEYID>.minisig`, at most
    /// [`publisher::MAX_SIGNATURE_BYTES`]), from the same place the package
    /// comes from. Its content is checked by the publisher verifier; where
    /// it was hosted never counts.
    ///
    /// [`publisher::MAX_SIGNATURE_BYTES`]: deckpilot_plugin_registry::publisher::MAX_SIGNATURE_BYTES
    pub fn signature(&self, pinned: &str, key_id: &str) -> Result<Vec<u8>, DownloadError> {
        if !deckpilot_plugin_registry::publisher::is_key_id(key_id) {
            return Err(DownloadError::Refused(format!("{key_id} is not a key id")));
        }
        let relative = pinned
            .strip_prefix(DOWNLOAD_PREFIX)
            .and_then(plain_path)
            .ok_or_else(|| {
                DownloadError::Refused(format!("{pinned} is not a release asset of this registry"))
            })?;
        let base = match &self.config.releases {
            Some(releases) => releases.clone(),
            None => {
                Url::parse(DOWNLOAD_PREFIX).map_err(|e| DownloadError::Refused(e.to_string()))?
            }
        };
        let name = deckpilot_plugin_registry::publisher::signature_file_name(relative, key_id);
        let url = base
            .join(&name)
            .map_err(|e| DownloadError::Refused(format!("{name}: {e}")))?;
        let limit = u64::try_from(deckpilot_plugin_registry::publisher::MAX_SIGNATURE_BYTES)
            .unwrap_or(u64::MAX);
        self.get(
            &url,
            limit,
            Redirects::OrHosts(&self.config.package_redirect_hosts),
            DOCUMENT_TIMEOUT,
        )
        .map_err(|e| match e {
            FetchError::NotFound => DownloadError::Unavailable(format!(
                "{} (a publisher signature) was not found",
                shown(&url)
            )),
            FetchError::Unavailable(m) => {
                DownloadError::Unavailable(format!("{} could not be downloaded: {m}", shown(&url)))
            }
            FetchError::Refused(m) => {
                DownloadError::Refused(format!("{} was refused: {m}", shown(&url)))
            }
        })
    }

    /// `url`, following allowed redirects, at most `limit` bytes.
    fn get(
        &self,
        url: &Url,
        limit: u64,
        redirects: Redirects<'_>,
        timeout: Duration,
    ) -> Result<Vec<u8>, FetchError> {
        self.runtime.block_on(async {
            tokio::time::timeout(timeout, self.get_async(url, limit, redirects))
                .await
                .unwrap_or_else(|_| {
                    Err(FetchError::Unavailable(format!(
                        "no complete answer within {} s",
                        timeout.as_secs()
                    )))
                })
        })
    }

    async fn get_async(
        &self,
        start: &Url,
        limit: u64,
        redirects: Redirects<'_>,
    ) -> Result<Vec<u8>, FetchError> {
        let mut url = start.clone();
        for _ in 0..=MAX_REDIRECTS {
            let response = self
                .client
                .get(url.clone())
                .send()
                .await
                .map_err(|e| FetchError::Unavailable(describe(e)))?;
            let status = response.status();
            if status.is_redirection() {
                let next = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|location| url.join(location).ok())
                    .ok_or_else(|| {
                        FetchError::Refused(format!(
                            "HTTP {} from {} without a valid location",
                            status.as_u16(),
                            shown(&url)
                        ))
                    })?;
                check_redirect(start, &url, &next, redirects)?;
                url = next;
                continue;
            }
            return match status.as_u16() {
                200 => read_body(response, limit).await,
                404 | 410 => Err(FetchError::NotFound),
                408 | 429 | 500..=599 => {
                    Err(FetchError::Unavailable(format!("HTTP {}", status.as_u16())))
                }
                other => Err(FetchError::Refused(format!("HTTP {other}"))),
            };
        }
        Err(FetchError::Refused(format!(
            "more than {MAX_REDIRECTS} redirects"
        )))
    }
}

fn check_redirect(
    start: &Url,
    from: &Url,
    next: &Url,
    redirects: Redirects<'_>,
) -> Result<(), FetchError> {
    if next.scheme() != "https" {
        return Err(FetchError::Refused(format!(
            "a redirect from {} to {}: only https is followed",
            shown(from),
            shown(next)
        )));
    }
    if same_origin(start, next) {
        return Ok(());
    }
    if let Redirects::OrHosts(hosts) = redirects
        && next
            .host_str()
            .is_some_and(|host| hosts.iter().any(|h| h.eq_ignore_ascii_case(host)))
    {
        return Ok(());
    }
    Err(FetchError::Refused(format!(
        "a redirect from {} to another origin ({}) is not followed",
        shown(from),
        next.origin().ascii_serialization()
    )))
}

/// The body, refused once it is larger than `limit`.
async fn read_body(mut response: reqwest::Response, limit: u64) -> Result<Vec<u8>, FetchError> {
    let too_large = || FetchError::Refused(format!("the answer is larger than {limit} bytes"));
    if response
        .content_length()
        .is_some_and(|length| length > limit)
    {
        return Err(too_large());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| FetchError::Unavailable(format!("the answer was cut off: {}", describe(e))))?
    {
        if u64::try_from(body.len() + chunk.len()).unwrap_or(u64::MAX) > limit {
            return Err(too_large());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

impl RegistrySource for HttpsSource {
    fn fetch(&self, path: &str, max_bytes: usize) -> Result<Vec<u8>, FetchError> {
        let relative = plain_path(path)
            .ok_or_else(|| FetchError::Refused(format!("`{path}` is not a registry path")))?;
        let url = self
            .documents
            .join(relative)
            .map_err(|e| FetchError::Refused(format!("{path}: {e}")))?;
        let limit = u64::try_from(max_bytes).unwrap_or(u64::MAX);
        let result = self.get(&url, limit, Redirects::SameOrigin, DOCUMENT_TIMEOUT);
        if let Err(FetchError::Unavailable(why)) = &result
            && let Ok(mut unavailable) = self.unavailable.lock()
        {
            *unavailable = Some(format!("{}: {why}", shown(&url)));
        }
        result
    }
}

#[cfg(test)]
mod tests;
