# Installed-client usage

Claude and Codex are separate opt-ins, off until enabled. For Claude and Codex, only installed executable candidates are checked for Settings; detection does not launch clients, read app data or contact services. The old enabled preference migrates both providers once; the new preference then takes precedence.

Claude uses the installed Claude Code read-only control protocol (initialize and get_usage, skip_behaviors). The client may contact Anthropic. A Tuck-owned status-line file remains a local fallback, read only while Claude is on. Codex uses its installed app-server (initialize, account/read with refreshToken false, account/rateLimits/read); the client may contact OpenAI. Neither sends prompts, login requests, tool approvals or token copies. Readers refresh every five minutes while enabled, with manual refresh, bounded messages and deadlines. Disabling cancels in-flight requests and removes the provider's rings.

## Antigravity opt-in

Antigravity is a separate opt-in, off by default. Its switch appears only if `%USERPROFILE%/.gemini/antigravity-acp/acp_token.json` exists (existence only). Turning it on requires confirmation that Tuck reads that sign-in file and contacts Google for quota. This is the only exception to the no-other-app-credential-files rule. Rust reads at most 16 KiB, keeps credentials only during the request, and never writes the file. OAuth refresh goes to oauth2.googleapis.com; quota goes to daily-cloudcode-pa.googleapis.com and cloudcode-pa.googleapis.com, using HTTPS with redirects disabled and timeouts. No userinfo, login or prompts. Turning it off cancels requests, drops secrets and removes the ring. It relies on an undocumented Google service that may stop working.

The Windows path constant `UNVERIFIED_WINDOWS_TOKEN_PATH` is unverified. The pinned [T3 Code reader](https://github.com/pingdotgg/t3code/blob/e455200077ce60806a01c8f28435b58860dcfc74/apps/server/src/provider/antigravityUsageLimits.ts) establishes only `profileDirectory/antigravity-acp/acp_token.json`. [Google ACP docs](https://antigravity.google/docs/ide/extensions/acp) describe cached authentication but do not establish this Windows path. Tuck does not scan T3 profiles or offer alternative credential paths.

Summary bucket and fallback model responses are parsed conservatively: the most-used valid scoped allowance is one ring, not a general weekly or paired window. Only fixed labels, percentages, reset times and fixed errors cross IPC. Failures retain the last reading as Error at the normal five-minute cadence; no login is attempted. The existing epoch cancels work and prevents late results from restoring disabled rings.

Development and tests use fake fixtures only. This reader has never run against a real account. No local Antigravity or T3 Code data was opened, printed, copied or listed.

Google's private quota service appears to refuse clients it doesn't recognise, so quota requests send the same `antigravity/hub` User-Agent as T3 Code. Tuck identifies itself as Antigravity's client on those requests, which is part of why the service could stop working or be restricted.
