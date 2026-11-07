# Installed-client usage

Claude and Codex are separate opt-ins, off until enabled. Only installed executable candidates are checked for Settings; detection does not launch clients, read app data or contact services. The old enabled preference migrates both providers once; the new preference then takes precedence.

Claude uses the installed Claude Code read-only control protocol (initialize and get_usage, skip_behaviors). The client may contact Anthropic. A Tuck-owned status-line file remains a local fallback, read only while Claude is on. Codex uses its installed app-server (initialize, account/read with refreshToken false, account/rateLimits/read); the client may contact OpenAI. Neither sends prompts, login requests, tool approvals or token copies. Readers refresh every five minutes while enabled, with manual refresh, bounded messages and deadlines. Disabling cancels in-flight requests and removes the provider's rings.

## Antigravity deferred

Checked public source on 2026-11-07, without reading local T3 Code or Antigravity data.

[T3 Code quota reader](https://github.com/pingdotgg/t3code/blob/e455200077ce60806a01c8f28435b58860dcfc74/apps/server/src/provider/antigravityUsageLimits.ts) reads antigravity-acp/acp_token.json containing client_id, client_secret and refresh_token. It exchanges the refresh token at oauth2.googleapis.com/token, optionally reads Google userinfo, then calls retrieveUserQuotaSummary or fetchAvailableModels on daily-cloudcode-pa.googleapis.com and cloudcode-pa.googleapis.com with bearer authorization. This violates Tuck's no-credential-file/no-token-copy rule. No Antigravity reader, switch or ring is shipped.

Its separate conversation database reader counts token usage, not subscription allowances, so it cannot substitute for quota numbers. [Google's plans documentation](https://antigravity.google/docs/plans) describes baseline limits and viewing usage in Settings; it does not establish a credential-free external quota protocol.

Options for the owner: wait for a documented read-only installed-client protocol; support an explicitly exported quota-only file if Google provides one; or separately reconsider a first-party Tuck authorization flow. Credential-file access is excluded under the current rules.
