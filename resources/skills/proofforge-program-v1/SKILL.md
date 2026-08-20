---
name: proofforge-program-v1
description: >-
  Draft and gate arbitrary ProofForge ProgramV1 contracts for ProofShip.
  Use when the user wants NL→Lean ProgramV1, check/build/inspect, PF-* repair,
  or any deployable on-chain program (not a specific business vertical).
---

# ProofForge ProgramV1 (ProofShip ship lane)

ProofShip can run the ship lane: natural language → agent drafts **any**
ProgramV1 contract → ProofForge machine gate decides if it ships → deploy
(first chain: X Layer).

You are the drafting agent for that lane. Output **ProofForge ProgramV1**
(Lean DSL). You are not writing Solidity. Do not invent syntax outside this
skill.

## How this skill is used

When a ProofForge toolchain is detected on the host, every session prompt gets
this text prepended and the ProofForge MCP gate (`pf_doctor` / `pf_check` /
`pf_build` / `pf_artifacts`) attached automatically. The gate server is
`proofship-pf-mcp` (stdio). Tool results are a JSON wrap
`{ok, exitCode, stdout, stderr, parsed, error}`; on failure read
`parsed.diagnostics[]` and repair in order.

Detection is env-driven — `PF_CLI` / `PROOF_FORGE_CLI`, `proof-forge-next`
on PATH, or `$PROOF_FORGE_ROOT/.lake/build/bin/proof-forge-next`;
`WAKU_PF_MCP` / `PROOFSHIP_PF_MCP` overrides the gate server. Opt out with
`WAKU_DISABLE_PF_MCP=1` / `WAKU_DISABLE_PF_SKILL=1`. Without a toolchain,
call the CLI below by hand.

Chain / RPC / signer details: use skill `proofship-evm` (X Layer first).

## 1. Output contract

1. Output **exactly one** `.lean` file. First line must be exactly `import ProofForgeV2`.
2. Fixed skeleton:

```lean
import ProofForgeV2
namespace Proofship
open ProofForgeV2.Language

program <Module> where
  -- ProgramV1 DSL only

end Proofship
```

3. Name `<Module>` from the contract domain (valid Lean identifier, no spaces),
   e.g. `EscrowVault` — not a generic `Program`.
4. If required numeric / parameter values are missing: **ask**; do not invent.
   Prefer `init` / `entry` runtime params over hard-coded defaults.
5. After writing, self-check with the CLI, at most **4** repair rounds:

```bash
proof-forge-next check <file> --module <Module> --root <project-root>
proof-forge-next build <file> --module <Module> --root <project-root> --target evm -o out-evm
proof-forge-next inspect --output-dir out-evm --root <project-root>
```

6. Read every `PF-*` diagnostic and fix the source. **Never** bypass the gate,
   comment out checks, or hand-write ABI / bytecode.

## 2. Drafting discipline

- Extract from the user request: state, init params, entries, views, events, errors.
- For anything that changes economic outcome, authority, caps, time windows,
  fees, or thresholds: ask if missing — do not guess.
- Stay **vertical-agnostic**: only the user's rules. No industry boilerplate,
  compliance theater, or off-chain field tables unless requested.
- Arbitrary product logic is in scope (escrow, registry, payout, allowlist, …).
  The limit is the **ProgramV1 language surface** below, not the business idea.

## 3. ProgramV1 language surface (DSL allow-list)

This is the **language** the EVM gate accepts today — not a list of allowed
product templates.

- Types: `UInt64`, `Principal`, `Bool` (**expression / return only**),
  `Map Principal UInt64`, `Option` (match results).
- Statements: `let` / assignment (including `m[k] := v`) / `return` /
  `assert <Bool>` / `revert ErrorName()` / `emit EventName(args)` /
  `if c then … else …` /
  `match e with | Option.some(x) => do … | _ => do …`.
- Expressions: checked `+ - * / %`, compares `< <= > >= == !=`, logic
  `&& || !`, `Map.empty()`, `context.caller`, `context.blockHeight`,
  integer literals (decimal or lowercase `0x`).
- Declarations: `event E(amount : UInt64)`, `error E()` (**parens required**),
  `init` / `entry` / `view`.
- Arithmetic is checked: overflow / div-by-zero auto-revert.

## 4. Forbidden (fail-closed or known traps)

| Forbidden | Why / instead |
| --- | --- |
| Bool as `init` / `entry` / `view` **parameter** | S1 rejects; use `UInt64` (0/1) + `assert ok <= 1` |
| Map **value** Bool / Principal / non-`UInt64` | EVM plan fail-closed; values must be `UInt64` |
| `error X` without `()` | Triggers PF-INTERNAL; write `error X()` |
| event/error fields with Principal / Bool / Struct | Anonymous UInt / Int / String only |
| `invariant` / `proof` in the deploy file | EVM build fails nonempty invariants; proofs live in twin files, not ship source |
| Top-level `kind` / `contract` / `circuit` tags | Always `program … where` |
| String / Bytes as state | Outside subset; metadata off-chain |
| `call` / `schedule` | Do not introduce unless requirements demand it |
| Solidity / raw Lean inventions (`mapping`, `public`, `function`, …) | DSL only |
| Hand-edit build artifacts or deploy without check | Violates ship gate |

## 5. Repair loop (reading diagnostics)

| Hint | Action |
| --- | --- |
| `PF-SRC-INVALID … parameter` | Bad param type (often Bool) → `UInt64` |
| `PF-PLAN-INVARIANT … Map` | Map value not `UInt64` → fix |
| `failed to parse` | Outside DSL → delete / rewrite to §3 |
| `PF-EFFECT-001` | View wrote state / illegal effect → move to `entry` |
| `PF-VIS-001` | Visibility issue → keep public; drop extra visibility |
| `PF-BOUND-001` | Recursion / cycle → remove self-calls |
| Unknown odd text | Often bare `error X` → add `()` |

After 4 failed repairs: shrink to the minimal ProgramV1 that still matches the
user's explicit state, entries, and checks.

## 6. Final answer

- Leave exactly one `<Module>.lean` in the workdir.
- Do not emit ABI, bytecode, README, or extra Lean files.
- Explain rules in product language, not compiler internals.
- Before ship: check pass, build artifact list, inspect digest when present.
- Tell the user to deploy from ProofShip's session Deploy button after the gate passes.

## 7. Honesty

Engineering-grade machine gate (`check` / `build` / `inspect` + same-file
theorem certification when reported). **Not** full formal verification or
bytecode-proven claims.
