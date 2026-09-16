# Security

NiceGit is preview software that can modify repositories and execute local Git
and shell processes. Only open repositories you trust: Git hooks, filters, Git
configuration, and shell startup files may execute code with your user privileges.
The embedded terminal is a real shell, not an isolated sandbox.

## Reporting a Vulnerability

Do not post credentials, private repository content, or exploit details in a public
issue. Use GitHub's private **Report a vulnerability** option in this repository's
Security tab when available. Maintainers must enable private vulnerability
reporting before inviting public reports. If it is unavailable, open a minimal
issue requesting a private reporting channel without disclosing the vulnerability.
There is no guaranteed response time or supported maintenance window yet.

## Credentials and Distribution

NiceGit reuses system Git authentication and the GitHub CLI login; it does not
implement its own token store. Repository paths and commit drafts are saved in
local app preferences. Diagnostic screenshots and errors can contain private
paths or remote addresses; review them before sharing.

Local app bundles are ad-hoc signed, not Developer ID signed or notarized. Do not
disable Gatekeeper or remove quarantine attributes to run an untrusted download.
For now, building reviewed source locally is the intended installation path.
