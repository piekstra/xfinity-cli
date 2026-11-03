# Security policy

## Reporting

Open a private security advisory on the repository (Security → Advisories → New
draft). Please do not file a public issue for anything credential-related.

## What this tool holds

`xfin` talks to Xfinity's account services with a token you capture from your own
logged-in browser. It has no telemetry, and no server component unless you run `xfin
mcp --http` (see [The MCP server](#the-mcp-server)). Everything it stores is local,
except the copy of the `--http` bearer token you give your own Cloudflare Worker:

| Thing | Where it lives | Notes |
| --- | --- | --- |
| Xfinity bearer token | OS keychain, service `piekstra.xfin` (a token stored by an older version under `xfinity-cli` moves there on first read by a command) | Equivalent to your signed-in session until Xfinity expires it. Enters through `--stdin` or `--from-env <VAR>`, never a flag. |
| `--http` bearer token | OS keychain, service `piekstra.xfin`, item `mcp-http-token`, or `XFIN_MCP_TOKEN`; **a copy in your Cloudflare Worker**, as its `HOME_XFINITY_TOKEN` secret | Only if you use `xfin mcp --http`. It admits a caller to the read-only tools. |
| Non-secret settings | `~/.config/xfinity-cli/config.json` | Username, default account number, refresh command. No secrets. |

## The MCP server

`xfin mcp` serves read-only tools to an MCP client (Claude Code, Claude Desktop) over
stdio, on this machine only; it opens no port unless run with `--http` (below). It
reads the token from `$XFINITY_TOKEN`, then the keychain, on each call. A tool never
signs in, prompts, runs the configured refresh command, or creates or changes a
keychain item: a token under the legacy `xfinity-cli` service is read where it is,
and a missing or expired token is an error asking the user to run `xfin auth login
--stdin --overwrite` in a terminal. The Claude Desktop extension (`.mcpb`) takes no
settings and so holds no credential of its own; on macOS its bundled binary needs
the user's approval, once, to read the `piekstra.xfin` keychain item.

`xfin mcp --http --port <PORT>` serves the same tools over HTTP for the user's own
Cloudflare Worker, which reaches it through the user's own Cloudflare Tunnel
(cli-common's `connector/`). It binds 127.0.0.1 only, with no flag for another
address, and refuses to start without a bearer token of 32 or more characters, from
`XFIN_MCP_TOKEN` or the keychain item `mcp-http-token` (`piekstra.xfin`), which `xfin
mcp --new-token` makes from 256 random bits and replaces only with `--overwrite`.
Every request must carry the token (compared in constant time) and a `Host` of
`127.0.0.1` or `localhost`, and must not carry an `Origin`; there are no CORS
headers. Only the read-only tools are served (all three are). The Xfinity token stays
on your computer: the Worker sends tool calls and receives results, and never sees it.

**Tool results leave your computer on this path.** They hold your balance, due date,
latest bill and data use. They cross Cloudflare, through your tunnel and your Worker,
and reach Claude, which shows them in your conversation. Who holds what, and what a
leaked token can do, is in the connector's
[threat model](https://github.com/piekstra/cli-common/blob/main/connector/README.md#threat-model).

`xfin auth logout` removes the Xfinity token and does not touch `mcp-http-token` or
stop `xfin mcp --http`. To revoke the connector's access, run `xfin mcp --new-token
--overwrite` and restart the server (`launchctl kickstart -k gui/$(id -u)/com.piekstra.xfin-mcp-http`);
the old token works until the restart. To stop serving, run `launchctl bootout
gui/$(id -u)/com.piekstra.xfin-mcp-http` and stop the tunnel.
