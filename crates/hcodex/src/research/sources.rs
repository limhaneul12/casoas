//! Host-owned, bounded source observations. Model citations never enter this path.

use std::collections::BTreeSet;
use std::fs::{self, File, Metadata};
use std::future::Future;
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::time::Duration;

use chrono::{SecondsFormat, Utc};
use reqwest::header::{CONTENT_LENGTH, CONTENT_TYPE, LOCATION};
use sha2::{Digest, Sha256};
use tokio::time::{Instant, timeout_at};
use url::{Host, Url};

use super::types::{
    Excerpt, ResearchRequest, SourceBundle, SourceFailure, SourceKind, SourceRecord, SourceSpec,
};

const MAX_REDIRECTS: usize = 3;
const MAX_DNS_ADDRESSES: usize = 16;
const SOURCE_IO_LIMIT: Duration = Duration::from_secs(180);

pub(super) fn source_io_deadline(
    job_deadline: crate::execution_policy::ExecutionDeadline,
) -> std::time::Instant {
    job_deadline.bounded_io_deadline(SOURCE_IO_LIMIT)
}

#[derive(Debug)]
struct SourceIssue {
    code: &'static str,
    detail: &'static str,
    observed_bytes: usize,
}

impl SourceIssue {
    fn new(code: &'static str, detail: &'static str) -> Self {
        Self {
            code,
            detail,
            observed_bytes: 0,
        }
    }
    fn read(mut self, observed_bytes: usize) -> Self {
        self.observed_bytes = observed_bytes;
        self
    }
}

type Acquired<T> = std::result::Result<T, SourceIssue>;
type WebFuture<'a, T> = Pin<Box<dyn Future<Output = Acquired<T>> + Send + 'a>>;

struct PublicTarget {
    url: Url,
    addresses: Vec<SocketAddr>,
}

struct WebReply {
    status: u16,
    location: Option<String>,
    media_type: String,
    body: Vec<u8>,
}

/// The resolver and HTTP reader are the external boundary replaced by fixtures.
trait WebTransport: Sync {
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> WebFuture<'a, Vec<SocketAddr>>;
    fn fetch<'a>(
        &'a self,
        target: &'a PublicTarget,
        max_bytes: usize,
        remaining: Duration,
    ) -> WebFuture<'a, WebReply>;
}

struct HttpsTransport;

impl WebTransport for HttpsTransport {
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> WebFuture<'a, Vec<SocketAddr>> {
        Box::pin(async move {
            let addresses = tokio::net::lookup_host((host, port))
                .await
                .map_err(|_| SourceIssue::new("dns_failed", "public source DNS lookup failed"))?
                .take(MAX_DNS_ADDRESSES + 1)
                .collect::<Vec<_>>();
            Ok(addresses)
        })
    }

    fn fetch<'a>(
        &'a self,
        target: &'a PublicTarget,
        max_bytes: usize,
        remaining: Duration,
    ) -> WebFuture<'a, WebReply> {
        Box::pin(async move {
            let host = target
                .url
                .host_str()
                .ok_or_else(|| SourceIssue::new("invalid_url", "source URL has no host"))?;
            let client = reqwest::Client::builder()
                .https_only(true)
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(remaining)
                .connect_timeout(remaining.min(Duration::from_secs(10)))
                .resolve_to_addrs(host, &target.addresses)
                .user_agent(concat!("hcodex-research/", env!("CARGO_PKG_VERSION")))
                .build()
                .map_err(|_| {
                    SourceIssue::new(
                        "https_client_failed",
                        "HTTPS source client could not be initialized",
                    )
                })?;
            let mut response = client.get(target.url.clone()).send().await.map_err(|_| {
                SourceIssue::new("https_request_failed", "HTTPS source request failed")
            })?;
            let status = response.status().as_u16();
            let location = response
                .headers()
                .get(LOCATION)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            let media_type = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            if (300..400).contains(&status) || !(200..300).contains(&status) {
                return Ok(WebReply {
                    status,
                    location,
                    media_type,
                    body: Vec::new(),
                });
            }
            if !textual_media(&media_type) {
                return Err(SourceIssue::new(
                    "unsupported_media_type",
                    "only explicit textual HTTPS sources are supported",
                ));
            }
            if response
                .headers()
                .get(CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok())
                .is_some_and(|size| size > max_bytes as u64)
            {
                return Err(SourceIssue::new(
                    "source_too_large",
                    "source Content-Length exceeds its bounded read budget",
                ));
            }
            let mut body = Vec::new();
            loop {
                let chunk = response.chunk().await.map_err(|_| {
                    SourceIssue::new(
                        "https_body_failed",
                        "HTTPS source body could not be completely observed",
                    )
                    .read(body.len())
                })?;
                let Some(chunk) = chunk else {
                    break;
                };
                let observed = body.len().saturating_add(chunk.len());
                if observed > max_bytes {
                    return Err(SourceIssue::new(
                        "source_too_large",
                        "HTTPS source exceeds its bounded read budget",
                    )
                    .read(observed));
                }
                body.extend_from_slice(&chunk);
            }
            Ok(WebReply {
                status,
                location,
                media_type,
                body,
            })
        })
    }
}

struct CapturedBody {
    kind: SourceKind,
    locator: String,
    media_type: String,
    redirect_chain: Vec<String>,
    body: String,
}

pub(super) async fn collect(request: &ResearchRequest) -> crate::Result<SourceBundle> {
    collect_with(request, &HttpsTransport).await
}

async fn collect_with<T: WebTransport>(
    request: &ResearchRequest,
    transport: &T,
) -> crate::Result<SourceBundle> {
    collect_with_deadline(
        request,
        transport,
        Instant::from_std(source_io_deadline(
            crate::execution_policy::ExecutionDeadline::from_limit(Duration::from_secs(
                request.budget.timeout_seconds,
            )),
        )),
    )
    .await
}

pub(super) async fn collect_before(
    request: &ResearchRequest,
    deadline: std::time::Instant,
) -> crate::Result<SourceBundle> {
    collect_with_deadline(request, &HttpsTransport, Instant::from_std(deadline)).await
}

async fn collect_with_deadline<T: WebTransport>(
    request: &ResearchRequest,
    transport: &T,
    deadline: Instant,
) -> crate::Result<SourceBundle> {
    let budget = &request.budget;
    if request.sources.len() > budget.max_sources
        || budget.max_source_bytes == 0
        || budget.max_excerpt_bytes == 0
    {
        return Err("research source acquisition budget is invalid".into());
    }
    let workspace = request.workspace_dir.canonicalize()?;
    let mut bundle = SourceBundle::default();
    let mut remaining_body = budget.max_total_source_bytes;
    let mut remaining_excerpt = budget.max_total_excerpt_bytes;
    let mut locators = BTreeSet::new();
    let mut bodies = BTreeSet::new();
    for (index, spec) in request.sources.iter().enumerate() {
        let max_body = budget.max_source_bytes.min(remaining_body);
        let max_excerpt = budget.max_excerpt_bytes.min(remaining_excerpt);
        let preliminary = if crate::supervisor::is_cancelled() {
            Some(SourceIssue::new(
                "cancelled",
                "source acquisition cancelled before admission",
            ))
        } else if Instant::now() >= deadline {
            Some(SourceIssue::new(
                "source_deadline_exceeded",
                "research source deadline expired",
            ))
        } else if max_body == 0 || max_excerpt == 0 {
            Some(SourceIssue::new(
                "source_budget_exhausted",
                "aggregate source or excerpt budget is exhausted",
            ))
        } else {
            None
        };
        let acquired = if let Some(issue) = preliminary {
            Err(issue)
        } else {
            match spec {
                SourceSpec::Repository { path } | SourceSpec::TechnicalDocument { path } => {
                    let root = workspace.clone();
                    let path = path.clone();
                    let kind = spec.kind();
                    tokio::task::spawn_blocking(move || read_local(&root, &path, kind, max_body))
                        .await?
                }
                SourceSpec::Web { url } => {
                    tokio::select! {
                        result = timeout_at(deadline, read_web(url, max_body, deadline, transport)) => {
                            result.unwrap_or_else(|_| Err(SourceIssue::new("source_deadline_exceeded", "research source deadline expired").read(max_body)))
                        }
                        () = crate::supervisor::cancelled() => Err(SourceIssue::new("cancelled", "HTTPS source acquisition cancelled").read(max_body)),
                    }
                }
            }
        };
        match acquired {
            Err(issue) => {
                remaining_body = remaining_body.saturating_sub(issue.observed_bytes);
                bundle
                    .failures
                    .push(failure(spec.kind(), safe_locator(spec), issue));
            }
            Ok(captured) => {
                remaining_body = remaining_body.saturating_sub(captured.body.len());
                let resource = if captured.kind == SourceKind::Web {
                    format!("web:{}", captured.locator)
                } else {
                    format!("file:{}", captured.locator)
                };
                if !locators.insert(resource) {
                    bundle.failures.push(failure(
                        captured.kind,
                        safe_locator(spec),
                        SourceIssue::new(
                            "duplicate_source",
                            "source resolves to an already captured resource",
                        ),
                    ));
                    continue;
                }
                let body_sha256 = digest(captured.body.as_bytes());
                if !bodies.insert(body_sha256.clone()) {
                    bundle.failures.push(failure(
                        captured.kind,
                        safe_locator(spec),
                        SourceIssue::new(
                            "duplicate_source_content",
                            "identical source bodies do not establish independent evidence",
                        ),
                    ));
                    continue;
                }
                let source_id = format!("source-{:03}", index + 1);
                let excerpt = select_captured_excerpt(
                    &source_id,
                    &captured.body,
                    &request.question,
                    max_excerpt,
                    captured.kind,
                    &captured.media_type,
                );
                if excerpt.text.trim().is_empty() {
                    bundle.failures.push(failure(
                        captured.kind,
                        captured.locator,
                        SourceIssue::new("empty_source", "source has no captured textual evidence"),
                    ));
                    continue;
                }
                remaining_excerpt = remaining_excerpt.saturating_sub(excerpt.text.len());
                bundle.sources.push(SourceRecord {
                    source_id,
                    kind: captured.kind,
                    locator: captured.locator,
                    observed_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
                    body_sha256,
                    body_bytes: captured.body.len() as u64,
                    media_type: captured.media_type,
                    redirect_chain: captured.redirect_chain,
                    excerpt,
                });
            }
        }
    }
    bundle.snapshot_digest = digest(&serde_json::to_vec(&(&bundle.sources, &bundle.failures))?);
    Ok(bundle)
}

pub(super) async fn recheck_local_sources(
    request: &ResearchRequest,
    bundle: &SourceBundle,
) -> crate::Result<Vec<SourceFailure>> {
    let workspace = request.workspace_dir.canonicalize()?;
    let records = bundle
        .sources
        .iter()
        .filter(|source| source.kind != SourceKind::Web)
        .cloned()
        .collect::<Vec<_>>();
    let max_body = request.budget.max_source_bytes;
    tokio::task::spawn_blocking(move || {
        let mut failures = Vec::new();
        for record in records {
            let locator = Path::new(&record.locator);
            // Current records carry canonical absolute locators for exported
            // citations. Historical relative records retain their original scope.
            let relative = if locator.is_absolute() {
                locator.strip_prefix(&workspace).map_err(|_| {
                    SourceIssue::new(
                        "unsafe_source_path",
                        "local evidence locator belongs to another workspace",
                    )
                })
            } else {
                Ok(locator)
            };
            let read = relative
                .and_then(|relative| read_local(&workspace, relative, record.kind, max_body));
            match read {
                Ok(captured) if digest(captured.body.as_bytes()) == record.body_sha256 => {}
                Ok(_) => failures.push(failure(
                    record.kind,
                    record.locator,
                    SourceIssue::new(
                        "source_changed",
                        "local evidence differs from its admitted snapshot",
                    ),
                )),
                Err(issue) => failures.push(failure(record.kind, record.locator, issue)),
            }
        }
        Ok(failures)
    })
    .await?
}

fn read_local(
    workspace: &Path,
    requested: &Path,
    kind: SourceKind,
    max_bytes: usize,
) -> Acquired<CapturedBody> {
    read_local_observed(workspace, requested, kind, max_bytes, || {})
}

fn read_local_observed<F: FnOnce()>(
    workspace: &Path,
    requested: &Path,
    kind: SourceKind,
    max_bytes: usize,
    after_open: F,
) -> Acquired<CapturedBody> {
    let absolute = confined_file(workspace, requested)?;
    let before = fs::symlink_metadata(&absolute).map_err(|_| {
        SourceIssue::new("source_unavailable", "local source metadata is unavailable")
    })?;
    if before.len() > max_bytes as u64 {
        return Err(SourceIssue::new(
            "source_too_large",
            "local source exceeds its bounded read budget",
        ));
    }
    let mut file = File::open(&absolute)
        .map_err(|_| SourceIssue::new("source_unavailable", "local source could not be opened"))?;
    let opened = file.metadata().map_err(|_| {
        SourceIssue::new(
            "source_unavailable",
            "opened source metadata is unavailable",
        )
    })?;
    if !same_snapshot(&before, &opened) {
        return Err(SourceIssue::new(
            "source_changed",
            "local source changed before reading",
        ));
    }
    after_open();
    let mut bytes = Vec::new();
    (&mut file)
        .take(max_bytes.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| {
            SourceIssue::new(
                "source_read_failed",
                "local source could not be completely observed",
            )
            .read(bytes.len())
        })?;
    if bytes.len() > max_bytes {
        return Err(SourceIssue::new(
            "source_too_large",
            "local source exceeded its bounded read budget",
        )
        .read(bytes.len()));
    }
    let after_open = file.metadata().map_err(|_| {
        SourceIssue::new(
            "source_changed",
            "local source metadata changed after reading",
        )
    })?;
    let after_path = fs::symlink_metadata(&absolute).map_err(|_| {
        SourceIssue::new("source_changed", "local source path changed after reading")
    })?;
    if !same_snapshot(&opened, &after_open)
        || !same_snapshot(&opened, &after_path)
        || confined_file(workspace, requested)? != absolute
    {
        return Err(
            SourceIssue::new("source_changed", "local source changed during reading")
                .read(bytes.len()),
        );
    }
    let body = textual_body(bytes)?;
    Ok(CapturedBody {
        kind,
        locator: absolute
            .to_str()
            .ok_or_else(|| {
                SourceIssue::new(
                    "unsupported_source_path",
                    "local source locator must be valid UTF-8",
                )
            })?
            .to_owned(),
        media_type: "text/plain".to_owned(),
        redirect_chain: Vec::new(),
        body,
    })
}

fn confined_file(workspace: &Path, requested: &Path) -> Acquired<PathBuf> {
    if requested.as_os_str().is_empty()
        || requested.is_absolute()
        || requested
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(SourceIssue::new(
            "unsafe_source_path",
            "local sources require a confined relative path",
        ));
    }
    let mut current = workspace.to_path_buf();
    for component in requested.components() {
        let name = component.as_os_str().to_string_lossy().to_ascii_lowercase();
        if sensitive_component(&name) {
            return Err(SourceIssue::new(
                "sensitive_source_path",
                "private state and credential paths are not research sources",
            ));
        }
        current.push(component);
        let metadata = fs::symlink_metadata(&current).map_err(|_| {
            SourceIssue::new("source_unavailable", "local source path is unavailable")
        })?;
        if metadata.file_type().is_symlink() {
            return Err(SourceIssue::new(
                "symlink_source",
                "research source paths may not traverse symlinks",
            ));
        }
    }
    let metadata = fs::symlink_metadata(&current).map_err(|_| {
        SourceIssue::new("source_unavailable", "local source metadata is unavailable")
    })?;
    if !metadata.is_file() || current.canonicalize().ok().as_ref() != Some(&current) {
        return Err(SourceIssue::new(
            "non_regular_source",
            "research source must be a regular confined file",
        ));
    }
    Ok(current)
}

fn sensitive_component(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | ".hcodex"
            | ".codex"
            | ".omx"
            | ".codegraph"
            | ".ssh"
            | ".aws"
            | ".azure"
            | ".gemini"
            | ".envrc"
            | ".netrc"
            | ".npmrc"
            | ".pypirc"
            | "auth.json"
            | "credentials"
            | "credentials.json"
            | "credentials.toml"
            | "secrets.toml"
            | "id_rsa"
            | "id_ed25519"
    ) || name == ".env"
        || name.starts_with(".env.")
        || name.ends_with(".pem")
        || name.ends_with(".key")
}

fn same_snapshot(left: &Metadata, right: &Metadata) -> bool {
    let (Ok(left_modified), Ok(right_modified)) = (left.modified(), right.modified()) else {
        return false;
    };
    if !left.is_file()
        || !right.is_file()
        || left.len() != right.len()
        || left_modified != right_modified
    {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        left.dev() == right.dev() && left.ino() == right.ino()
    }
    #[cfg(not(unix))]
    {
        left.created().ok() == right.created().ok()
    }
}

async fn read_web<T: WebTransport>(
    input: &str,
    max_bytes: usize,
    deadline: Instant,
    transport: &T,
) -> Acquired<CapturedBody> {
    let mut current = parse_web_url(input)?;
    let mut redirects = vec![current.to_string()];
    let mut visited = BTreeSet::new();
    loop {
        if !visited.insert(current.to_string()) {
            return Err(SourceIssue::new(
                "redirect_cycle",
                "HTTPS source redirect cycle rejected",
            ));
        }
        let host = current
            .host()
            .ok_or_else(|| SourceIssue::new("invalid_url", "HTTPS source has no host"))?;
        let port = current
            .port_or_known_default()
            .ok_or_else(|| SourceIssue::new("invalid_url", "HTTPS source has no port"))?;
        let addresses = match host {
            Host::Ipv4(ip) => vec![SocketAddr::new(IpAddr::V4(ip), port)],
            Host::Ipv6(ip) => vec![SocketAddr::new(IpAddr::V6(ip), port)],
            Host::Domain(domain) => transport.resolve(domain, port).await?,
        };
        let addresses = checked_addresses(addresses, port)?;
        let target = PublicTarget {
            url: current.clone(),
            addresses,
        };
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| {
                SourceIssue::new(
                    "source_deadline_exceeded",
                    "research source deadline expired",
                )
            })?;
        let reply = transport.fetch(&target, max_bytes, remaining).await?;
        if (300..400).contains(&reply.status) {
            if redirects.len() > MAX_REDIRECTS {
                return Err(SourceIssue::new(
                    "redirect_limit",
                    "HTTPS source exceeds three redirects",
                ));
            }
            let location = reply
                .location
                .as_deref()
                .filter(|value| value.len() <= 2048)
                .ok_or_else(|| {
                    SourceIssue::new(
                        "invalid_redirect",
                        "HTTPS redirect target is missing or oversized",
                    )
                })?;
            let joined = current.join(location).map_err(|_| {
                SourceIssue::new("invalid_redirect", "HTTPS redirect target is invalid")
            })?;
            current = parse_web_url(joined.as_str())?;
            redirects.push(current.to_string());
            continue;
        }
        if !(200..300).contains(&reply.status) {
            return Err(SourceIssue::new(
                "http_status_failed",
                "HTTPS source did not return a successful status",
            ));
        }
        if !textual_media(&reply.media_type) {
            return Err(SourceIssue::new(
                "unsupported_media_type",
                "only textual HTTPS sources are supported",
            ));
        }
        if reply.body.len() > max_bytes {
            return Err(SourceIssue::new(
                "source_too_large",
                "HTTPS source exceeds its bounded read budget",
            )
            .read(reply.body.len()));
        }
        let body = textual_body(reply.body)?;
        return Ok(CapturedBody {
            kind: SourceKind::Web,
            locator: current.to_string(),
            media_type: reply.media_type,
            redirect_chain: redirects,
            body,
        });
    }
}

fn parse_web_url(input: &str) -> Acquired<Url> {
    if input.len() > 2048 {
        return Err(SourceIssue::new(
            "invalid_url",
            "HTTPS source URL exceeds its length limit",
        ));
    }
    let mut url = Url::parse(input)
        .map_err(|_| SourceIssue::new("invalid_url", "web source URL is invalid"))?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host().is_none()
        || url.query_pairs().any(|(key, _)| sensitive_query(&key))
    {
        return Err(SourceIssue::new(
            "invalid_url",
            "web source must be credential-free HTTPS",
        ));
    }
    if let Some(Host::Domain(host)) = url.host() {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        if host == "localhost"
            || [
                ".localhost",
                ".local",
                ".internal",
                ".lan",
                ".home",
                ".onion",
            ]
            .iter()
            .any(|suffix| host.ends_with(suffix))
        {
            return Err(SourceIssue::new(
                "private_address",
                "local and internal source hosts are not permitted",
            ));
        }
    }
    url.set_fragment(None);
    Ok(url)
}

pub(super) fn sensitive_query(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.starts_with("x-amz-")
        || key.starts_with("x-goog-")
        || key.contains("token")
        || key.contains("secret")
        || key.contains("signature")
        || key.contains("auth")
        || matches!(
            key.as_str(),
            "token"
                | "access_token"
                | "auth"
                | "api_key"
                | "api-key"
                | "apikey"
                | "password"
                | "secret"
                | "client_secret"
                | "authorization"
                | "signature"
                | "sig"
                | "x-amz-credential"
                | "x-amz-security-token"
                | "x-amz-signature"
                | "x-goog-credential"
                | "x-goog-signature"
        )
}

fn checked_addresses(addresses: Vec<SocketAddr>, port: u16) -> Acquired<Vec<SocketAddr>> {
    if addresses.is_empty()
        || addresses.len() > MAX_DNS_ADDRESSES
        || addresses
            .iter()
            .any(|address| address.port() != port || !public_ip(address.ip()))
    {
        return Err(SourceIssue::new(
            "private_address",
            "source DNS must resolve exclusively to bounded public addresses",
        ));
    }
    Ok(addresses
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect())
}

fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => public_v4(ip),
        IpAddr::V6(ip) => ip.to_ipv4_mapped().map_or_else(|| public_v6(ip), public_v4),
    }
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_multicast()
        || ip.is_broadcast()
        || ip.is_documentation()
        || a == 0
        || a >= 240
        || (a == 100 && (64..=127).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 88 && c == 99)
        || (a == 198 && (b == 18 || b == 19)))
}

fn public_v6(ip: Ipv6Addr) -> bool {
    let segments = ip.segments();
    // Admit ordinary global unicast only, excluding documentation and protocol assignments.
    (segments[0] & 0xe000) == 0x2000
        && !(segments[0] == 0x2001 && (segments[1] < 0x0200 || segments[1] == 0x0db8))
        && segments[0] != 0x2002
        && !(segments[0] == 0x3fff && segments[1] & 0xf000 == 0)
}

fn textual_media(media: &str) -> bool {
    media.starts_with("text/")
        || matches!(
            media,
            "application/json"
                | "application/ld+json"
                | "application/xhtml+xml"
                | "application/xml"
        )
}

fn textual_body(bytes: Vec<u8>) -> Acquired<String> {
    let observed = bytes.len();
    if bytes.contains(&0) || bytes.starts_with(b"%PDF-") {
        return Err(SourceIssue::new(
            "unsupported_body",
            "binary and PDF source bodies are unsupported",
        )
        .read(observed));
    }
    let text = String::from_utf8(bytes).map_err(|_| {
        SourceIssue::new("unsupported_body", "source body must be UTF-8 text").read(observed)
    })?;
    if text.trim().is_empty() {
        return Err(
            SourceIssue::new("empty_source", "source contains no textual evidence").read(observed),
        );
    }
    Ok(text)
}

#[cfg(test)]
fn select_excerpt(source_id: &str, body: &str, question: &str, max_bytes: usize) -> Excerpt {
    select_excerpt_in_range(source_id, body, question, max_bytes, 0..body.len())
}

fn select_captured_excerpt(
    source_id: &str,
    body: &str,
    question: &str,
    max_bytes: usize,
    kind: SourceKind,
    media_type: &str,
) -> Excerpt {
    let range = primary_html_content(body, kind, media_type).unwrap_or(0..body.len());
    select_excerpt_in_range(source_id, body, question, max_bytes, range)
}

fn select_excerpt_in_range(
    source_id: &str,
    body: &str,
    question: &str,
    max_bytes: usize,
    range: std::ops::Range<usize>,
) -> Excerpt {
    let scoped = &body[range.clone()];
    let terms = question
        .split(|character: char| !character.is_alphanumeric())
        .filter(|term| term.chars().count() >= 3)
        .take(12)
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    let pattern = terms
        .iter()
        .map(|term| regex::escape(term))
        .collect::<Vec<_>>()
        .join("|");
    let matcher = if pattern.is_empty() {
        None
    } else {
        regex::RegexBuilder::new(&pattern)
            .case_insensitive(true)
            .build()
            .ok()
    };
    let mut offset = 0;
    let mut best = (0, 0);
    for line in scoped.split_inclusive('\n') {
        let lower = line.to_lowercase();
        let score = terms
            .iter()
            .filter(|term| lower.contains(term.as_str()))
            .count();
        if score > best.0 {
            // HTML is often one long line. Retain exact raw bytes around a
            // question match rather than repeatedly capturing its navigation.
            let within_line = if line.len() > max_bytes {
                matcher
                    .as_ref()
                    .and_then(|matcher| matcher.find(line))
                    .map_or(0, |found| found.start())
            } else {
                0
            };
            best = (score, offset + within_line);
        }
        offset += line.len();
    }
    let mut start = best.1.saturating_sub(max_bytes / 4);
    while !scoped.is_char_boundary(start) {
        start -= 1;
    }
    let line_start = scoped[..start].rfind('\n').map_or(0, |newline| newline + 1);
    if start.saturating_sub(line_start) <= max_bytes / 4 {
        start = line_start;
    }
    let mut end = start.saturating_add(max_bytes).min(scoped.len());
    while !scoped.is_char_boundary(end) {
        end -= 1;
    }
    start += range.start;
    end += range.start;
    let text = body[start..end].to_owned();
    let start_line = body[..start].bytes().filter(|byte| *byte == b'\n').count() + 1;
    let end_line = start_line + text.lines().count().max(1) - 1;
    Excerpt {
        excerpt_id: format!("{source_id}-excerpt-1"),
        source_id: source_id.to_owned(),
        sha256: digest(text.as_bytes()),
        text,
        start_byte: start as u64,
        end_byte: end as u64,
        start_line,
        end_line,
    }
}

fn primary_html_content(
    body: &str,
    kind: SourceKind,
    media_type: &str,
) -> Option<std::ops::Range<usize>> {
    if kind != SourceKind::Web || !matches!(media_type, "text/html" | "application/xhtml+xml") {
        return None;
    }
    raw_html_element_content(body, b"main").or_else(|| raw_html_element_content(body, b"article"))
}

fn raw_html_element_content(body: &str, tag: &[u8]) -> Option<std::ops::Range<usize>> {
    let bytes = body.as_bytes();
    let mut opening = Vec::with_capacity(tag.len() + 1);
    opening.push(b'<');
    opening.extend_from_slice(tag);
    let open = find_ascii_case_insensitive(bytes, &opening, 0, |next| {
        next == b'>' || next.is_ascii_whitespace()
    })?;
    let open_end = bytes[open..]
        .iter()
        .take(4_096)
        .position(|byte| *byte == b'>')?
        + open
        + 1;
    let mut closing = Vec::with_capacity(tag.len() + 2);
    closing.extend_from_slice(b"</");
    closing.extend_from_slice(tag);
    let close = find_ascii_case_insensitive(bytes, &closing, open_end, |next| {
        next == b'>' || next.is_ascii_whitespace()
    })?;
    (open_end < close && !body[open_end..close].trim().is_empty()).then_some(open_end..close)
}

fn find_ascii_case_insensitive<F>(
    haystack: &[u8],
    needle: &[u8],
    from: usize,
    valid_boundary: F,
) -> Option<usize>
where
    F: Fn(u8) -> bool,
{
    let mut cursor = from;
    while let Some(relative) = haystack
        .get(cursor..)?
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle))
    {
        let start = cursor + relative;
        if haystack
            .get(start + needle.len())
            .copied()
            .is_some_and(&valid_boundary)
        {
            return Some(start);
        }
        cursor = start.saturating_add(1);
    }
    None
}

fn failure(kind: SourceKind, locator: String, issue: SourceIssue) -> SourceFailure {
    SourceFailure {
        kind,
        locator,
        error_code: issue.code.to_owned(),
        detail: issue.detail.to_owned(),
    }
}

fn safe_locator(spec: &SourceSpec) -> String {
    let SourceSpec::Web { url: input } = spec else {
        return spec.locator();
    };
    let Ok(mut url) = Url::parse(input) else {
        return "[invalid web source]".to_owned();
    };
    if url.scheme() != "https" {
        return "[invalid HTTPS source]".to_owned();
    }
    let _username = url.set_username("");
    let _password = url.set_password(None);
    if url.query_pairs().any(|(key, _)| sensitive_query(&key)) {
        url.set_query(None);
    }
    url.set_fragment(None);
    url.to_string()
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests;
