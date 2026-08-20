---
name: waku-cloudflare
description: >-
  Waku's default Web2 half: Cloudflare Workers, Pages, R2, KV, D1.
  Use when a dapp needs storage, indexing, a frontend host, or any
  service a contract cannot do alone. Prefer the attached Cloudflare MCP tools.
---

# Waku Cloudflare (default Web2)

A gated contract is the on-chain kernel. Real products also need a frontend
and a Web2 half. **Cloudflare is that default** in Waku — toggle it in
Settings → MCP.

## When this is attached

Settings → MCP has the Cloudflare rows **on**. Then every new run carries
those HTTP URLs. The coding agent opens Waku's built-in browser the first
time a server asks for authorization. **Do not ask the user for an API
token for MCP.**

| MCP | URL | Auth |
| --- | --- | --- |
| `cloudflare-docs` | `https://docs.mcp.cloudflare.com/mcp` | none |
| `cloudflare-api` | `https://mcp.cloudflare.com/mcp` | OAuth in the agent |
| `cloudflare-bindings` | `https://bindings.mcp.cloudflare.com/mcp` | OAuth in the agent |

One-click Ship → Cloudflare still uses `CLOUDFLARE_API_TOKEN` or the
optional token on Settings → MCP (wrangler only — not MCP).

## What to use it for

- **Frontend host** — Cloudflare Pages
- **Edge backend** — Workers (indexers, signature checks, webhooks)
- **Storage** — R2, KV, D1
- Do not reimplement these as a random VPS

## Frontend loop

1. Put UI next to the contract (`index.html`, Vite, or `public/`).
2. Header **Preview** opens the right-panel browser on loopback.
3. **Ship** can publish Pages / Workers when a CLI token is present.
