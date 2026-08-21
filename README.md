# ProofShip

[![Latest release](https://img.shields.io/github/v/release/DaviRain-Su/proof_ship)](https://github.com/DaviRain-Su/proof_ship/releases/latest)
[![License: GPL-3.0](https://img.shields.io/github/license/DaviRain-Su/proof_ship)](LICENSE)
[![macOS | Linux | Windows](https://img.shields.io/badge/platform-macOS%20%7C%20Linux%20%7C%20Windows-111)](https://github.com/DaviRain-Su/proof_ship/releases/latest)

Native desktop app for local coding agents — plus on-device wallets and X Layer
deploy. Rust, GPUI, no cloud account.

ProofShip drives the agent CLIs you already have (Amp, Claude Code, Codex,
Cursor, Grok, Kimi, OpenCode, Pi). Projects, sessions, and transcripts stay on
your machine. From the same window you can preview, sign, and deploy EVM
contracts to [OKX X Layer](https://web3.okx.com/xlayer) — no browser wallet
extension required.

- Website: [pfs.grok.me](https://pfs.grok.me)
- Downloads: [GitHub Releases](https://github.com/DaviRain-Su/proof_ship/releases/latest) (desktop installers — not npm/Docker packages)

It is built in Rust with [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui).

## Install

Download builds from the
[GitHub releases](https://github.com/DaviRain-Su/proof_ship/releases).

On macOS, download `ProofShip-<version>.dmg`. These builds are ad-hoc signed
(no Apple Developer certificate), so Gatekeeper will block a double-click.
Clear the quarantine flag, then open the disk image:

```sh
xattr -cr ~/Downloads/ProofShip-*.dmg
open ~/Downloads/ProofShip-*.dmg
```

If macOS still refuses the app after you drag it to Applications:

```sh
xattr -cr /Applications/ProofShip.app
```

You can also Control-click the app and choose **Open**.

On Linux:

```sh
curl -fsSL https://github.com/DaviRain-Su/proof_ship/releases/latest/download/install.sh | sh
```

The script installs into `~/.local` without root. See
[docs/linux.md](docs/linux.md) for requirements, manual installation, and
uninstalling.

On Windows, run `ProofShip-<version>-<arch>-Setup.exe` from the
[latest release](https://github.com/DaviRain-Su/proof_ship/releases/latest). It installs
per-user. A portable `.zip` is published alongside it. See
[docs/windows.md](docs/windows.md) for requirements and what is not available
there yet.

## Supported agents

ProofShip works with:

- [Amp](https://ampcode.com/)
- Claude Code
- Codex CLI
- Cursor CLI
- Grok Build
- Kimi Code
- OpenCode
- Pi

Install and authenticate at least one supported agent CLI before starting ProofShip.
ProofShip detects available CLIs automatically and uses each provider's native
structured protocol and session continuity.

## Highlights

- Keep projects and independent agent sessions in one native app.
- Switch models, reasoning effort, and access modes from a shared interface.
- Queue or steer follow-up messages while an agent is working.
- Rewind Git-backed tasks with conversation-aware checkpoints.
- Store app state locally, with no ProofShip account or remote service required.

## Architecture

The native desktop is an RPC client of the standalone `proofship-daemon` process.
Provider sessions run in [`proofship-core`](crates/proofship-core), behind the
authenticated, versioned WebSocket contract in
[`proofship-protocol`](crates/proofship-protocol). ProofShip Desktop depends on
[`proofship-client`](crates/proofship-client), not on the daemon implementation. The
daemon owns task SQLite data, uploaded attachments, provider-native session
forks, and all workspace filesystem and Git operations; paths returned by it
always refer to the daemon host. The desktop retains only presentation state
and a disposable preview cache.

The browser client lives at [`apps/web`](apps/web) and uses the generated
browser transport in [`packages/proofship-client`](packages/proofship-client). Its
checked-in types are generated directly from the Rust protocol, while its
WebSocket client implements the same handshake, request IDs, subscriptions,
sequence deduplication, and replay cursors as the Rust client. Run
`bun run protocol:generate` after changing a wire type and
`bun run protocol:check` to verify that generated files are current.

Projectless task workspaces live on the daemon host under
`~/.proofship/projects/<date>/<slug>` (or `~/.waku/projects` on machines that
already have a Waku config directory). The daemon moves workspaces created by the
older `~/.waku/<date>/<slug>` layout on first load.

Configuration ownership is separate too: the Release desktop writes
`~/.proofship/app.json` (falling back to `~/.waku/app.json` when that already
exists), while Debug stays isolated at `temp/app.json`. Daemon
provider and Computer Use settings live in the same configuration directory. The
desktop's Settings → Daemon page can explicitly
expose the child daemon on a fixed port, configure exact browser origins, and
copy its stable authentication token. It remains loopback-only by default.

When connected to a daemon managed outside the desktop process, ProofShip never
interprets daemon paths on the client machine. The local folder picker and PTY
are therefore unavailable until the protocol gains daemon-host picker and
terminal-stream endpoints; files, diffs, Git, skills, usage, task state, and
attachments already use daemon RPC.

Release apps bundle and sign `proofship-daemon`. Development keeps the daemon at
`target/debug/proofship-debug-daemon`, allowing provider-only edits to rebuild and
replace the daemon without relaunching ProofShip Debug.

## Development

Development is supported on macOS, Linux, and Windows and requires
[Rust 1.96 or newer](https://www.rust-lang.org/tools/install) and
[Bun](https://bun.sh/). Linux supports both Wayland and X11, and Windows needs
the MSVC toolchain; install the native build prerequisites listed in
[CONTRIBUTING.md](CONTRIBUTING.md) first.

```sh
bun install
bun run dev
```

The embedded browser and experimental computer-use integration currently
remain macOS-only. Agent sessions, projects, transcripts, skills, usage,
diffs, file editing, and the terminal run natively on Linux and Windows.

See [CONTRIBUTING.md](CONTRIBUTING.md) for the development workflow and checks.
Release maintainers should also read [RELEASING.md](RELEASING.md).

## Sponsorship

You can support the project development via [GitHub Sponsors](https://github.com/sponsors/egoist).

## License

ProofShip is licensed under the [GNU General Public License v3.0 only](LICENSE).
