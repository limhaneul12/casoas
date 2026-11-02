"""Application entry point for the Heterarchy Codex Collaboration MCP server."""

from __future__ import annotations

from app.server import main, mcp

__all__ = [
    "main",
    "mcp",
]

if __name__ == "__main__":
    main()
