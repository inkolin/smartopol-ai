# Security Policy

## Supported Versions

| Version | Supported |
|---|---|
| 0.1.x | Yes |

## Reporting a Vulnerability

**Do not open a public issue for security vulnerabilities.**

Instead, please report security issues by emailing:

**nenad@nikolin.eu**

Include:
- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Suggested fix (if any)

You will receive a response within 48 hours acknowledging the report. We will work with you to understand and address the issue before any public disclosure.

## Security Measures

SmartopolAI is designed with security as a core principle:

- Pure Rust TLS (rustls) — no OpenSSL dependency
- SQLite with bundled compilation — no system library dependency
- Payload size limits on all network inputs
- Authentication required by default (token mode)
- Handshake timeouts to prevent resource exhaustion
- Planned: audit logging, secrets vault, exec sandboxing

## Responsible Disclosure

We ask that you give us reasonable time to address vulnerabilities before public disclosure. We commit to:

- Acknowledging reports within 48 hours
- Providing a timeline for a fix
- Crediting reporters (unless they prefer anonymity)
