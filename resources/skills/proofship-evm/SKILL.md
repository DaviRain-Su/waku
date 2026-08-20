---
name: proofship-evm
description: >-
  Waku multi-EVM deploy guidance with X Layer first. Use when resolving
  chain metadata, RPC/explorer URLs, or casting
  deploy/interact against Waku networks. Prefer xlayer-testnet (1952)
  unless the user names another chain.
---

# Waku EVM (X Layer first)

**Product focus right now: OKX X Layer only** (testnet 1952 default, mainnet 196
for funded ops). Sepolia/Base Sepolia remain as Settings builtins for power users.

## Scope

Authoritative builtins live in Settings → Networks:

| id | name | chainId | currency | Role |
| --- | --- | --- | --- | --- |
| `xlayer-testnet` | X Layer Testnet | **1952** | OKB | **Default** — drafts, demos |
| `xlayer-mainnet` | X Layer | 196 | OKB | Funded product ops (no DevEnvKey) |
| `ethereum-sepolia` | Ethereum Sepolia | 11155111 | ETH | Multi-EVM optional |
| `base-sepolia` | Base Sepolia | 84532 | ETH | Multi-EVM optional |

Custom networks may be added in Settings → Networks. Known mainnet chain ids
are blocked for **DevEnvKey** signing; use a Local signer for those.

## Resolve before acting

1. Infer network from: explicit name, chain id, or Settings → Networks pick.
2. If ambiguous, ask. Do **not** default to Ethereum mainnet.
3. Prefer `xlayer-testnet` for drafts, gates, and demos.
4. Explorer links: use the network's explorer URL + `/address/{addr}`.

## Signers

- **Create wallet** / **Import key** in Settings → Wallets (Local, mode 0600)
- **DevEnvKey**: env var *name* only, testnet-only
- Never ask the user to paste a private key into chat

## OKX OnchainOS MCP (when attached)

When the user configured an OnchainOS API key (Settings → Networks), sessions
may carry the hosted `okx-onchainos` MCP server. Prefer its tools for DEX work
instead of hand-rolling calldata. These tools **construct** transactions only —
signing and sending stays in Waku (Settings → Wallets / Deploy). Never ask the
user to paste a private key to "complete" a swap.

## Deploy discipline

- Gate must PASS before deploy (fail closed)
- Keys: Local signer (Create / Import) or env var name for DevEnvKey
- Local keys live under the daemon `web3/wallet-secrets/` directory (mode 0600)
- Never paste private keys into chat
- After `pf_build`, tell the user to use Waku's session Deploy button

## Anti-patterns

- Do not invent RPC URLs when a builtin already exists
- Do not claim full formal verification / bytecode-proven
