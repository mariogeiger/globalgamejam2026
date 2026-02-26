# Deployment Guide

Live at **https://ggj26.geiger.ink**

## Update

```bash
cd ~/git/globalgamejam2026
git pull
sudo docker compose -f deploy/docker-compose.yml up -d --build
```

## First-Time Server Setup

1. Install Docker: https://docs.docker.com/engine/install/
2. Add `ggj26.geiger.ink` to Caddy (`/etc/caddy/Caddyfile`):

```
ggj26.geiger.ink {
    reverse_proxy localhost:9000
}
```

3. Reload Caddy: `sudo systemctl reload caddy`
4. Start the container: `sudo docker compose -f deploy/docker-compose.yml up -d --build`

## GitHub Actions (automatic)

Pushes to `main` trigger a build + deploy via GitHub Actions. Requires these repo secrets:

| Secret | Description | Value |
|--------|-------------|-------|
| `SSH_PRIVATE_KEY` | Deploy key (full file contents) | `cat ~/.ssh/github_deploy` |
| `SSH_HOST` | Server hostname | `geiger.ink` |
| `SSH_USER` | SSH user | `mario` |
| `DEPLOY_PATH` | Repo path on server | `/home/mario/git/globalgamejam2026` |

To set up the deploy key:

```bash
ssh-keygen -t ed25519 -C "github-deploy" -f ~/.ssh/github_deploy -N ""
ssh-copy-id -i ~/.ssh/github_deploy.pub mario@geiger.ink
```

Manual trigger: Actions → Deploy → Run workflow

## Commands

All from the repo root:

```bash
sudo docker compose -f deploy/docker-compose.yml logs -f       # View logs
sudo docker compose -f deploy/docker-compose.yml restart        # Restart
sudo docker compose -f deploy/docker-compose.yml ps             # Status
sudo docker compose -f deploy/docker-compose.yml down           # Stop
```

## Ports

| Port | Protocol | Used by |
|------|----------|---------|
| 443 | TCP | Caddy (HTTPS → reverse proxy to :9000) |
| 9000 | TCP | Game server (HTTP + WebSocket) |
| 3478 | UDP | TURN/STUN (NAT traversal) |

## Troubleshooting

**Container won't start**: `sudo docker compose logs` to check errors

**HTTPS issues**: `sudo systemctl status caddy`, ensure DNS resolves: `dig +short ggj26.geiger.ink`

**TURN not working**: Ensure UDP port 3478 is forwarded on the router
