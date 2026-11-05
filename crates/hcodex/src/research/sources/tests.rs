use super::*;
use std::collections::VecDeque;
use std::sync::Mutex;

#[derive(Default)]
struct FixtureTransport {
    dns: Mutex<VecDeque<Vec<SocketAddr>>>,
    replies: Mutex<VecDeque<WebReply>>,
    observed: Mutex<Vec<(String, Vec<SocketAddr>)>>,
}

impl FixtureTransport {
    fn with(replies: Vec<WebReply>, dns: Vec<Vec<SocketAddr>>) -> Self {
        Self {
            dns: Mutex::new(dns.into()),
            replies: Mutex::new(replies.into()),
            observed: Mutex::new(Vec::new()),
        }
    }
}

impl WebTransport for FixtureTransport {
    fn resolve<'a>(&'a self, _host: &'a str, _port: u16) -> WebFuture<'a, Vec<SocketAddr>> {
        Box::pin(async move {
            self.dns
                .lock()
                .map_err(|_| SourceIssue::new("fixture_lock", "DNS fixture lock failed"))?
                .pop_front()
                .ok_or_else(|| SourceIssue::new("dns_failed", "fixture has no DNS response"))
        })
    }
    fn fetch<'a>(
        &'a self,
        target: &'a PublicTarget,
        _max_bytes: usize,
        _remaining: Duration,
    ) -> WebFuture<'a, WebReply> {
        Box::pin(async move {
            self.observed
                .lock()
                .map_err(|_| SourceIssue::new("fixture_lock", "HTTP fixture lock failed"))?
                .push((target.url.to_string(), target.addresses.clone()));
            self.replies
                .lock()
                .map_err(|_| SourceIssue::new("fixture_lock", "HTTP fixture lock failed"))?
                .pop_front()
                .ok_or_else(|| {
                    SourceIssue::new("https_request_failed", "fixture has no HTTP response")
                })
        })
    }
}

fn reply(body: &str) -> WebReply {
    WebReply {
        status: 200,
        location: None,
        media_type: "text/plain".to_owned(),
        body: body.as_bytes().to_vec(),
    }
}

fn html_reply(body: &str) -> WebReply {
    WebReply {
        status: 200,
        location: None,
        media_type: "text/html".to_owned(),
        body: body.as_bytes().to_vec(),
    }
}

fn address(ip: &str) -> crate::Result<SocketAddr> {
    Ok(SocketAddr::new(ip.parse()?, 443))
}

#[tokio::test]
async fn captures_local_repository_and_document_without_retaining_full_body() -> crate::Result<()> {
    let workspace = tempfile::tempdir()?;
    fs::write(
        workspace.path().join("lib.rs"),
        "pub fn checked() -> bool { true }\n",
    )?;
    fs::write(
        workspace.path().join("guide.md"),
        "Title\nEvidence is bounded and explicit.\n",
    )?;
    let mut request = ResearchRequest::new("bounded evidence", workspace.path().to_path_buf());
    request.budget.max_excerpt_bytes = 24;
    request.sources = vec![
        SourceSpec::Repository {
            path: "lib.rs".into(),
        },
        SourceSpec::TechnicalDocument {
            path: "guide.md".into(),
        },
    ];
    let bundle = collect(&request).await?;
    assert_eq!(bundle.sources.len(), 2);
    assert!(bundle.failures.is_empty());
    assert!(bundle.snapshot_digest.starts_with("sha256:"));
    for source in &bundle.sources {
        assert!(Path::new(&source.locator).is_absolute());
        let full = fs::read(workspace.path().join(&source.locator))?;
        assert_eq!(source.body_sha256, digest(&full));
        let excerpt = &source.excerpt;
        let start = usize::try_from(excerpt.start_byte)?;
        let end = usize::try_from(excerpt.end_byte)?;
        assert_eq!(&full[start..end], excerpt.text.as_bytes());
        assert_eq!(excerpt.sha256, digest(excerpt.text.as_bytes()));
        assert!(excerpt.text.len() <= 24);
        assert!(chrono::DateTime::parse_from_rfc3339(&source.observed_at).is_ok());
    }
    assert!(recheck_local_sources(&request, &bundle).await?.is_empty());
    fs::write(workspace.path().join("guide.md"), "Changed evidence.\n")?;
    let stale = recheck_local_sources(&request, &bundle).await?;
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].error_code, "source_changed");
    Ok(())
}

#[tokio::test]
async fn canonical_local_locator_preserves_path_and_supports_legacy_freshness() -> crate::Result<()>
{
    let workspace = tempfile::tempdir()?;
    #[cfg(unix)]
    let name = "source\\name.md";
    #[cfg(not(unix))]
    let name = "source-name.md";
    fs::write(workspace.path().join(name), "Exact local evidence.\n")?;
    let mut request = ResearchRequest::new("local citation", workspace.path().to_path_buf());
    request
        .sources
        .push(SourceSpec::TechnicalDocument { path: name.into() });
    let bundle = collect(&request).await?;
    assert_eq!(bundle.sources.len(), 1);
    let expected = workspace.path().canonicalize()?.join(name);
    assert_eq!(Path::new(&bundle.sources[0].locator), expected);
    assert_eq!(
        fs::read_to_string(&bundle.sources[0].locator)?,
        "Exact local evidence.\n"
    );
    assert!(recheck_local_sources(&request, &bundle).await?.is_empty());

    let mut legacy = bundle.clone();
    legacy.sources[0].locator = name.to_owned();
    assert!(recheck_local_sources(&request, &legacy).await?.is_empty());

    let outside = tempfile::tempdir()?;
    fs::write(outside.path().join("other.md"), "Exact local evidence.\n")?;
    legacy.sources[0].locator = outside
        .path()
        .join("other.md")
        .to_string_lossy()
        .into_owned();
    let failures = recheck_local_sources(&request, &legacy).await?;
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].error_code, "unsafe_source_path");
    Ok(())
}

#[tokio::test]
async fn rejects_escapes_private_state_symlinks_and_nonregular_files() -> crate::Result<()> {
    let workspace = tempfile::tempdir()?;
    fs::write(workspace.path().join(".env"), "DO_NOT_CAPTURE_SECRET")?;
    fs::create_dir(workspace.path().join("directory"))?;
    let mut request = ResearchRequest::new("source safety", workspace.path().to_path_buf());
    request.sources = vec![
        SourceSpec::Repository {
            path: "../escape".into(),
        },
        SourceSpec::Repository {
            path: ".env".into(),
        },
        SourceSpec::Repository {
            path: "directory".into(),
        },
    ];
    #[cfg(unix)]
    {
        fs::write(workspace.path().join("target.txt"), "secret target")?;
        std::os::unix::fs::symlink(
            workspace.path().join("target.txt"),
            workspace.path().join("link.txt"),
        )?;
        request.sources.push(SourceSpec::TechnicalDocument {
            path: "link.txt".into(),
        });
    }
    let bundle = collect(&request).await?;
    assert!(bundle.sources.is_empty());
    assert_eq!(bundle.failures.len(), request.sources.len());
    assert!(!serde_json::to_string(&bundle)?.contains("DO_NOT_CAPTURE_SECRET"));
    assert!(
        bundle
            .failures
            .iter()
            .any(|failure| failure.error_code == "sensitive_source_path")
    );
    #[cfg(unix)]
    assert!(
        bundle
            .failures
            .iter()
            .any(|failure| failure.error_code == "symlink_source")
    );
    Ok(())
}

#[tokio::test]
async fn captures_https_with_exact_public_address_pin_and_redirect_receipts() -> crate::Result<()> {
    let workspace = tempfile::tempdir()?;
    let transport = FixtureTransport::with(
        vec![
            WebReply {
                status: 302,
                location: Some("/final".to_owned()),
                media_type: String::new(),
                body: Vec::new(),
            },
            reply("Captured web evidence.\n"),
        ],
        vec![vec![address("93.184.216.34")?], vec![address("1.1.1.1")?]],
    );
    let mut request = ResearchRequest::new("web evidence", workspace.path().to_path_buf());
    request.sources.push(SourceSpec::Web {
        url: "https://example.com/start#ignored".to_owned(),
    });
    let bundle = collect_with(&request, &transport).await?;
    assert!(bundle.failures.is_empty());
    assert_eq!(bundle.sources.len(), 1);
    let source = &bundle.sources[0];
    assert_eq!(source.locator, "https://example.com/final");
    assert_eq!(
        source.redirect_chain,
        ["https://example.com/start", "https://example.com/final"]
    );
    assert_eq!(source.body_sha256, digest(b"Captured web evidence.\n"));
    let observed = transport
        .observed
        .lock()
        .map_err(|_| "fixture lock poisoned")?;
    assert_eq!(observed.len(), 2);
    assert_eq!(observed[0].1, [address("93.184.216.34")?]);
    assert_eq!(observed[1].1, [address("1.1.1.1")?]);
    Ok(())
}

#[tokio::test]
async fn html_web_excerpt_prefers_raw_main_body_over_long_matching_navigation() -> crate::Result<()>
{
    let workspace = tempfile::tempdir()?;
    let body = format!(
        "<html><head><title>Rust2024 targetquestion</title></head><body><nav>{}</nav><main lang=\"ko\"><h1>실제 본문</h1><p>targetquestion의 검증된 answer_evidence는 본문에 있다.</p></main><footer>Rust2024</footer></body></html>",
        "<svg>Rust2024 navigation targetquestion</svg>".repeat(500)
    );
    let transport = FixtureTransport::with(
        vec![html_reply(&body)],
        vec![vec![address("93.184.216.34")?]],
    );
    let mut request = ResearchRequest::new(
        "Rust2024 targetquestion answer_evidence",
        workspace.path().to_path_buf(),
    );
    request.budget.max_excerpt_bytes = 320;
    request.sources.push(SourceSpec::Web {
        url: "https://example.com/mdbook".to_owned(),
    });
    let bundle = collect_with(&request, &transport).await?;
    assert!(bundle.failures.is_empty());
    let source = &bundle.sources[0];
    assert_eq!(source.body_sha256, digest(body.as_bytes()));
    assert!(source.excerpt.text.contains("answer_evidence"));
    assert!(source.excerpt.text.contains("실제 본문"));
    assert!(!source.excerpt.text.contains("<head>"));
    let start = usize::try_from(source.excerpt.start_byte)?;
    let end = usize::try_from(source.excerpt.end_byte)?;
    assert_eq!(&body[start..end], source.excerpt.text);
    assert_eq!(
        source.excerpt.sha256,
        digest(source.excerpt.text.as_bytes())
    );
    assert_eq!(
        source.excerpt.start_line,
        body[..start].bytes().filter(|byte| *byte == b'\n').count() + 1
    );
    Ok(())
}

#[tokio::test]
async fn local_plaintext_does_not_treat_literal_main_tag_as_html_structure() -> crate::Result<()> {
    let workspace = tempfile::tempdir()?;
    let body = format!(
        "needle_before_literal is the required local evidence.\n{}\nconst SAMPLE: &str = \"<main>unrelated late text</main>\";\n",
        "padding line\n".repeat(200)
    );
    fs::write(workspace.path().join("literal.rs"), &body)?;
    let mut request = ResearchRequest::new("needle_before_literal", workspace.path().to_path_buf());
    request.budget.max_excerpt_bytes = 128;
    request.sources.push(SourceSpec::Repository {
        path: "literal.rs".into(),
    });
    let bundle = collect(&request).await?;
    assert!(bundle.failures.is_empty());
    assert!(
        bundle.sources[0]
            .excerpt
            .text
            .contains("needle_before_literal")
    );
    Ok(())
}

#[test]
fn xhtml_article_fallback_is_ascii_case_insensitive_and_keeps_unicode_offsets() -> crate::Result<()>
{
    let body = format!(
        "<HTML><HEAD>{}</HEAD><BODY><ARTICLE class=\"doc\">앞부분 αβγ unicode_article_answer 뒷부분</ARTICLE></BODY></HTML>",
        "unicode_article_answer navigation ".repeat(400)
    );
    let excerpt = select_captured_excerpt(
        "xhtml-1",
        &body,
        "unicode_article_answer",
        128,
        SourceKind::Web,
        "application/xhtml+xml",
    );
    assert!(excerpt.text.contains("unicode_article_answer"));
    assert!(excerpt.text.contains("αβγ"));
    let start = usize::try_from(excerpt.start_byte)?;
    let end = usize::try_from(excerpt.end_byte)?;
    assert_eq!(body.get(start..end), Some(excerpt.text.as_str()));
    assert_eq!(excerpt.sha256, digest(excerpt.text.as_bytes()));
    Ok(())
}

#[tokio::test]
async fn rejects_private_mixed_dns_and_redirects_before_any_private_fetch() -> crate::Result<()> {
    let workspace = tempfile::tempdir()?;
    for addresses in [
        vec![address("127.0.0.1")?],
        vec![address("93.184.216.34")?, address("10.0.0.1")?],
    ] {
        let transport = FixtureTransport::with(vec![reply("must never read")], vec![addresses]);
        let mut request = ResearchRequest::new("safe URL", workspace.path().to_path_buf());
        request.sources.push(SourceSpec::Web {
            url: "https://example.com/".to_owned(),
        });
        let bundle = collect_with(&request, &transport).await?;
        assert!(bundle.sources.is_empty());
        assert_eq!(bundle.failures[0].error_code, "private_address");
        assert!(
            transport
                .observed
                .lock()
                .map_err(|_| "fixture lock poisoned")?
                .is_empty()
        );
    }
    let transport = FixtureTransport::with(
        vec![WebReply {
            status: 302,
            location: Some("https://127.0.0.1/private".to_owned()),
            media_type: String::new(),
            body: Vec::new(),
        }],
        vec![vec![address("93.184.216.34")?]],
    );
    let mut request = ResearchRequest::new("safe redirect", workspace.path().to_path_buf());
    request.sources.push(SourceSpec::Web {
        url: "https://example.com/".to_owned(),
    });
    let bundle = collect_with(&request, &transport).await?;
    assert_eq!(bundle.failures[0].error_code, "private_address");
    assert_eq!(
        transport
            .observed
            .lock()
            .map_err(|_| "fixture lock poisoned")?
            .len(),
        1
    );
    Ok(())
}

#[test]
fn rejects_nonpublic_ipv4_and_ipv6_including_mapped_addresses() -> crate::Result<()> {
    for ip in [
        "0.0.0.0",
        "127.0.0.1",
        "10.0.0.1",
        "169.254.169.254",
        "100.64.0.1",
        "192.0.2.1",
        "192.88.99.1",
        "198.18.0.1",
        "224.0.0.1",
        "240.0.0.1",
        "::1",
        "::ffff:127.0.0.1",
        "fc00::1",
        "fe80::1",
        "2001:db8::1",
        "2002:7f00:1::1",
        "3fff::1",
        "64:ff9b::7f00:1",
    ] {
        assert!(
            !public_ip(ip.parse()?),
            "private or special address accepted: {ip}"
        );
    }
    for ip in [
        "93.184.216.34",
        "8.8.8.8",
        "2606:4700:4700::1111",
        "2001:4860:4860::8888",
    ] {
        assert!(public_ip(ip.parse()?), "public address refused: {ip}");
    }
    Ok(())
}

#[tokio::test]
async fn unsupported_oversized_and_duplicate_evidence_remain_failures() -> crate::Result<()> {
    let workspace = tempfile::tempdir()?;
    let transport = FixtureTransport::with(
        vec![
            WebReply {
                status: 200,
                location: None,
                media_type: "application/pdf".to_owned(),
                body: b"%PDF-no".to_vec(),
            },
            reply("123456789"),
            reply("real"),
            reply("real"),
        ],
        vec![vec![address("1.1.1.1")?]; 4],
    );
    let mut request = ResearchRequest::new("bounded", workspace.path().to_path_buf());
    request.budget.max_source_bytes = 8;
    for suffix in ["pdf", "large", "first", "duplicate"] {
        request.sources.push(SourceSpec::Web {
            url: format!("https://example.com/{suffix}"),
        });
    }
    let bundle = collect_with(&request, &transport).await?;
    assert_eq!(bundle.sources.len(), 1);
    assert_eq!(
        bundle
            .failures
            .iter()
            .map(|failure| failure.error_code.as_str())
            .collect::<Vec<_>>(),
        [
            "unsupported_media_type",
            "source_too_large",
            "duplicate_source_content"
        ]
    );
    Ok(())
}

#[tokio::test]
async fn aggregate_body_and_excerpt_budgets_stop_further_acquisition() -> crate::Result<()> {
    let workspace = tempfile::tempdir()?;
    fs::write(workspace.path().join("one.md"), "abcd")?;
    fs::write(workspace.path().join("two.md"), "efgh")?;
    let mut request = ResearchRequest::new("budget", workspace.path().to_path_buf());
    request.sources = vec![
        SourceSpec::TechnicalDocument {
            path: "one.md".into(),
        },
        SourceSpec::TechnicalDocument {
            path: "two.md".into(),
        },
    ];
    request.budget.max_total_source_bytes = 4;
    let bundle = collect(&request).await?;
    assert_eq!(bundle.sources.len(), 1);
    assert_eq!(bundle.failures[0].error_code, "source_budget_exhausted");
    request.budget.max_total_source_bytes = 20;
    request.budget.max_total_excerpt_bytes = 4;
    let bundle = collect(&request).await?;
    assert_eq!(bundle.sources.len(), 1);
    assert_eq!(bundle.failures[0].error_code, "source_budget_exhausted");
    Ok(())
}

#[tokio::test]
async fn invalid_credential_url_is_redacted_without_a_network_call() -> crate::Result<()> {
    let workspace = tempfile::tempdir()?;
    let transport = FixtureTransport::default();
    let mut request = ResearchRequest::new("private", workspace.path().to_path_buf());
    for url in [
        "https://user:SECRET@example.com/a?token=PRIVATE#fragment",
        "https://example.com/file?X-Amz-Signature=AWS_PRIVATE",
        "https://example.com/file?api-key=API_PRIVATE",
        "javascript:PRIVATE",
    ] {
        request.sources.push(SourceSpec::Web {
            url: url.to_owned(),
        });
    }
    let bundle = collect_with(&request, &transport).await?;
    assert_eq!(bundle.failures.len(), 4);
    let encoded = serde_json::to_string(&bundle)?;
    assert!(!encoded.contains("SECRET"));
    assert!(!encoded.contains("PRIVATE"));
    assert!(
        transport
            .observed
            .lock()
            .map_err(|_| "fixture lock poisoned")?
            .is_empty()
    );
    Ok(())
}

#[test]
fn excerpt_keeps_utf8_byte_and_line_positions() {
    let body = "첫 번째 줄\nbounded evidence here\n마지막\n";
    let excerpt = select_excerpt("s1", body, "evidence", 30);
    assert!(excerpt.text.len() <= 30);
    assert_eq!(
        &body[excerpt.start_byte as usize..excerpt.end_byte as usize],
        excerpt.text
    );
    assert_eq!(
        excerpt.start_line,
        body[..excerpt.start_byte as usize]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
            + 1
    );
    assert_eq!(
        excerpt.end_line,
        excerpt.start_line + excerpt.text.lines().count().max(1) - 1
    );
}

#[test]
fn long_single_line_web_text_selects_an_observed_question_window() {
    let body = format!(
        "{} critical_target evidence is here {}",
        "navigation ".repeat(1500),
        "footer ".repeat(20)
    );
    let excerpt = select_excerpt("web1", &body, "critical_target", 96);
    assert!(excerpt.text.contains("critical_target"));
    assert_eq!(
        &body[excerpt.start_byte as usize..excerpt.end_byte as usize],
        excerpt.text
    );
    assert!(excerpt.text.len() <= 96);
    assert_eq!(excerpt.start_line, 1);
    assert_eq!(excerpt.end_line, 1);
}

#[test]
fn replacement_after_open_is_not_admitted_as_stable_evidence() -> crate::Result<()> {
    let workspace = tempfile::tempdir()?;
    let path = workspace.path().join("source.md");
    let replacement = workspace.path().join("replacement.md");
    fs::write(&path, "Initial observed source.\n")?;
    fs::write(&replacement, "Different source at same path.\n")?;
    let outcome = read_local_observed(
        workspace.path(),
        Path::new("source.md"),
        SourceKind::TechnicalDocument,
        1024,
        || {
            assert!(fs::rename(&replacement, &path).is_ok());
        },
    );
    let Err(issue) = outcome else {
        return Err("replaced source admitted".into());
    };
    assert_eq!(issue.code, "source_changed");
    Ok(())
}

struct PendingDns {
    started: std::sync::Arc<tokio::sync::Notify>,
    dropped: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

struct DnsLifetime(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Drop for DnsLifetime {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

impl WebTransport for PendingDns {
    fn resolve<'a>(&'a self, _host: &'a str, _port: u16) -> WebFuture<'a, Vec<SocketAddr>> {
        Box::pin(async move {
            let _lifetime = DnsLifetime(self.dropped.clone());
            self.started.notify_one();
            std::future::pending().await
        })
    }

    fn fetch<'a>(
        &'a self,
        _target: &'a PublicTarget,
        _max_bytes: usize,
        _remaining: Duration,
    ) -> WebFuture<'a, WebReply> {
        Box::pin(async {
            Err(SourceIssue::new(
                "unexpected_fetch",
                "pending DNS must not reach HTTP",
            ))
        })
    }
}

#[tokio::test]
async fn owned_cancellation_drops_pending_network_and_preserves_observed_local_sources()
-> crate::Result<()> {
    let workspace = tempfile::tempdir()?;
    fs::write(
        workspace.path().join("observed.md"),
        "Observed local evidence before cancellation.\n",
    )?;
    let mut request = ResearchRequest::new("cancel source fetch", workspace.path().to_path_buf());
    request.sources = vec![
        SourceSpec::TechnicalDocument {
            path: "observed.md".into(),
        },
        SourceSpec::Web {
            url: "https://example.com/pending".to_owned(),
        },
    ];
    let started = std::sync::Arc::new(tokio::sync::Notify::new());
    let dropped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let transport = PendingDns {
        started: started.clone(),
        dropped: dropped.clone(),
    };
    let supervisor = crate::supervisor::Supervisor::new(1);
    let (sender, receiver) = tokio::sync::oneshot::channel();
    supervisor
        .spawn("source-cancel-fixture".to_owned(), async move {
            let _sent = sender.send(collect_with(&request, &transport).await);
        })
        .await?;
    started.notified().await;
    assert!(supervisor.cancel("source-cancel-fixture").await);
    let bundle = receiver.await??;
    supervisor.shutdown(Duration::from_secs(1)).await?;
    assert_eq!(bundle.sources.len(), 1);
    assert_eq!(bundle.failures.len(), 1);
    assert_eq!(bundle.failures[0].error_code, "cancelled");
    assert!(dropped.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(supervisor.active().await, 0);
    Ok(())
}
