# Host Controller Integration

HalluScribe's private remote lifecycle path uses a separate standalone Windows
Host Controller. The controller owns process start/stop; HalluScribe remains
the authoritative application and model/archive owner.

## Current implementation

- The controller listens on the Windows PC LAN address at port `8787`.
- The airOS wakebot sends authenticated requests from the dish IP.
- `/halluscribe start|stop` controls the fixed HalluScribe executable.
- `/vscode start|stop` controls the fixed VS Code executable and workspace.
- Requests use timestamp, nonce, HMAC-SHA256, source-IP pinning, and replay protection.
- Executable paths and arguments are local controller configuration; Telegram cannot supply them.
- The controller secret is stored locally with Windows DPAPI and is never committed.

## Deployment

Build and install the standalone package from the `HalluscribeHostController`
repository, then run its one-time `configure-autostart.ps1` setup. Configure
the dish's non-secret controller URL with the wakebot repository's
`host/set-controller-url.py` helper. Never commit `wake.conf` or any secret.

The controller's `/v1/sleep` endpoint and the HalluScribe Telegram assistant
transport remain follow-up work described in
`HALLUSCRIBE_TELEGRAM_ASSISTANT.md`.