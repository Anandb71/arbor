# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 0.1.x   | :white_check_mark: |

## Reporting a Vulnerability

We take security seriously. If you discover a security vulnerability in Arbor, please report it responsibly.

### How to Report

1. **Do NOT** open a public GitHub issue for security vulnerabilities
2. Email security concerns to: **<anandbiju71@gmail.com>**
3. Include:
   - Description of the vulnerability
   - Steps to reproduce
   - Potential impact
   - Suggested fix (if any)

### What to Expect

- **Response time**: Within 48 hours
- **Fix timeline**: Critical issues within 7 days, others within 30 days
- **Disclosure**: Coordinated disclosure after fix is released

### Scope

The following are in scope:

- Arbor CLI (`arbor-cli`)
- Arbor MCP Bridge (`arbor-mcp`)
- Arbor Server (`arbor-server`)
- Visualizer (Flutter desktop app)

Out of scope:

- Third-party dependencies (report to upstream)
- Issues requiring physical access

## Security Best Practices

When using Arbor:

- Keep your Arbor installation updated
- Don't expose the WebSocket server (port 7433) to the public internet
- Review the graph data before sharing `.arbor/` directories

Thank you for helping keep Arbor secure!
