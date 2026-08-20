---
name: waku-vercel
description: >-
  Waku Vercel hosting via the official MCP. Use when the user wants to
  deploy a frontend to Vercel, manage projects, or inspect deployments.
---

# Waku Vercel

Vercel is a first-class hosting preset, not a second worldview. Cloudflare
stays the default Web2 half; turn Vercel on in Settings → MCP when the
user asks for it.

## When this is attached

Settings → MCP has the Vercel row **on**. The coding agent opens Waku's
built-in browser if the server asks for authorization. **Do not ask the
user to paste a Vercel token into chat.**

One-click Ship → Vercel uses `VERCEL_TOKEN` or the optional token on
Settings → MCP (CLI only — not MCP).

## What to use it for

- Production frontend host (`vercel.app`)
- Project and domain management through the Vercel MCP tools
- Prefer MCP tools over guessing CLI flags
