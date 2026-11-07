# Security

## Supported versions

Only the latest published release receives security fixes. Upgrade before reporting a problem unless the problem prevents upgrading.

## Reporting

Use [GitHub private vulnerability reporting](https://github.com/jojomensah89/tuck/security/advisories/new). If the form is unavailable, ask the maintainer in an issue to enable private reporting without describing the vulnerability. Do not post exploit details, credentials, private copied text, or signing material publicly.

In scope: Tuck code, installer and updater integrity, unsafe local file access, permission boundaries, and dependency vulnerabilities reachable through Tuck. Provider account problems and vulnerabilities in Windows or third-party services belong with their owners.

## Updates

The Tauri updater verifies signed packages against the public key embedded in the app. This helps reject modified updates without a matching signature. It cannot protect against a compromised signing key or malicious authorized release. HTTPS protects transport. Updater signing is separate from Windows Authenticode signing. Model downloads also check pinned sizes and SHA-256 digests. Never include signing keys or passwords in reports.
