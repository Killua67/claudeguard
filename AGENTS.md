# ClaudeGuard engineering boundaries

- Preserve existing user configurations and installed apps. Development does not replace an installation or Dock entry unless explicitly authorized.
- Tests use pure rules, fixtures, harmless stubs, `--demo` or `--demo-first-run`. User-authorized live acceptance uses `--observe-only`: real read-only collection / fixed Cloudflare with an immutable backend client-execution ban.
- Never execute real Claude CLI (including `--version`), Desktop, login or installers in automated development. Never send HTTP, TLS or DNS probes to Anthropic/Claude hosts.
- Never inspect accounts, credentials, conversations, Keychain, official-client settings or shell startup files. Never modify system proxy, DNS, VPN, timezone or firewall.
- Proxy discovery is local read-only evidence, not trust or authorization. First use requires explicit confirmation before diagnostics; existing valid settings must be preserved. Missing, invalid, unconfirmed and unsupported cannot become pass.
- Live diagnostics have one fixed HTTPS destination: Cloudflare trace through the confirmed loopback HTTP proxy. No direct fallback, redirects, arbitrary destinations, authentication or TLS bypass. Offline domains are documentation, not a probe queue.
- Proxy-preflight mode keeps IPv6 enabled: only IPv6 Warning is a nonblocking coverage advisory. Never relabel unknown coverage as Pass or relax other warnings/failures. Refresh IPv6 observations during full scans only; do not claim runtime traffic coverage.
- Loopback mock CONNECT tests may use real curl and ephemeral 127.0.0.1/::1 servers; all resolver overrides/forwarding targets must remain loopback, with no public traffic or real clients.
- Keep manual launch intent, freshness and backend authorization strict. UI, imported reports or background recovery cannot authorize launch. Demo and observe-only reject execution regardless of frontend state.
- Platform capabilities are explicit. Windows and WSL require independent implementation and acceptance before support claims.
- Behavior changes need observable regression tests, including delayed callbacks, configuration failure and concurrency. Run Rust tests/fmt/Clippy and frontend tests/build/format; public export tests and exact file manifest must also pass.
- Launch, network, privacy and authorization changes require independent review. Never upload the development workspace wholesale; export only the exact reviewed public-file list. Do not invent vulnerability contacts or claim unexecuted CI/release checks passed.
