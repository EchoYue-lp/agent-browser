# Documentation

Welcome to the Agent Browser documentation!

## Overview

Agent Browser is a browser automation toolkit designed specifically for AI agents. It implements the MCP 2025-11-25 specification with full support for Tools, Resources, Prompts, and Logging capabilities.

## Documentation Index

| Document | Description |
|----------|-------------|
| [Getting Started](./getting-started.md) | Installation and quick start guide |
| [API Reference](./api-reference.md) | Complete API documentation |
| [Configuration](./configuration.md) | Configuration options and environment variables |
| [Examples](./examples.md) | Usage examples and code snippets |
| [Architecture](./architecture.md) | System architecture and design decisions |

## Quick Links

### For AI Agent Users

- [MCP Server Setup](./getting-started.md#mcp-server-setup) - Configure with Claude Code, Cursor, etc.
- [Available Tools](./api-reference.md#mcp-tools) - 30+ MCP tools with annotations
- [MCP Resources](./api-reference.md#mcp-resources) - Screenshot and snapshot as resources
- [MCP Prompts](./api-reference.md#mcp-prompts) - Pre-defined prompts for common tasks
- [CSS Selector Operations](./examples.md#css-selector-operations) - Direct element operations

### For Developers

- [HTTP API](./api-reference.md#http-api) - RESTful API endpoints
- [Rust Library](./getting-started.md#rust-library) - Use as a Rust crate
- [Architecture](./architecture.md) - Understand the system design

## MCP 2025-11-25 Support

Agent Browser fully implements [MCP 2025-11-25](https://modelcontextprotocol.io/specification/2025-11-25):

- **Protocol Version**: `2025-11-25`
- **Supported Versions**: `2025-11-25`, `2025-06-18`, `2025-03-26`, `2024-11-05`

### Server Capabilities

| Capability | Features |
|------------|----------|
| **Tools** | 30+ browser automation tools with behavior annotations |
| **Resources** | Screenshot and snapshot accessible via MCP resources |
| **Prompts** | Pre-defined prompts for page analysis, form filling, data extraction |
| **Logging** | Configurable log levels |

### Tool Annotations

Each tool includes annotations describing its behavior:

- `readOnlyHint` - Tool only reads data, no side effects
- `destructiveHint` - Tool may cause irreversible changes
- `idempotentHint` - Same input always produces same result
- `openWorldHint` - Tool interacts with external systems

## Language

Documentation is available in:
- **English** (default) - You're reading it
- [中文文档](./README_CN.md) - Chinese documentation

## Need Help?

- [GitHub Issues](https://github.com/EchoYue/agent-browser/issues) - Report bugs or request features
- [Contributing](../CONTRIBUTING.md) - How to contribute