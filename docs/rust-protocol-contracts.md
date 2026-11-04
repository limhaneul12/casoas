# Rust MCP protocol compatibility

The Rust adapter supports legacy initialize negotiation for 2024-11-05, 2025-03-26, 2025-06-18 and 2025-11-25. It echoes a supported requested handshake version and counter-offers 2025-11-25 otherwise; malformed versions are rejected.

It also implements the 2026-07-28 stateless discovery path: `server/discover`, version and client-capabilities request metadata, supported-version response, serverInfo metadata, and unsupported-version error -32022. Registered tool descriptors remain the pinned 16-tool contract; successful calls include the advertised `structuredContent.result` text wrapper.

Primary protocol references: [legacy lifecycle](https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/docs/specification/2025-03-26/basic/lifecycle.mdx), [modern discovery](https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/docs/specification/2026-07-28/server/discover.mdx), [modern versioning](https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/docs/specification/2026-07-28/basic/versioning.mdx). Installed Python MCP 2.3.0 version constants were inspected as the migration compatibility baseline.

Verification: typed parser tests cover malformed/unsupported/supported versions, modern discovery metadata, legacy finite stdio framing, and the real Rust binary catalog/handoff/job smoke. Complete cross-client/provider acceptance remains tracked in the migration status; protocol unit tests are not provider E2E.
