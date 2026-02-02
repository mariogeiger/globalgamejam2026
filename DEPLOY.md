# Deployment Guide

GitHub Actions builds on ubuntu-latest and SCPs binaries to your server. No Rust needed on server.

## Server Setup

```bash
# 1. Create deploy directory
mkdir -p ~/ggj26/client/dist

# 2. Create systemd service (replace YOUR_USERNAME)
sudo tee /etc/systemd/system/ggj-server.service << 'EOF'
[Unit]
Description=GGJ26 Server
After=network.target
[Service]
Type=simple
User=YOUR_USERNAME
WorkingDirectory=/home/YOUR_USERNAME/ggj26
ExecStart=/home/YOUR_USERNAME/ggj26/server
Restart=always
Environment=RUST_LOG=info
[Install]
WantedBy=multi-user.target
EOF

# 3. Enable service
sudo systemctl daemon-reload && sudo systemctl enable ggj-server

# 4. Allow passwordless restart (replace YOUR_USERNAME)
echo "YOUR_USERNAME ALL=(ALL) NOPASSWD: /usr/bin/systemctl restart ggj-server" | sudo tee /etc/sudoers.d/ggj-deploy
```

## GitHub Setup

### 1. Generate SSH key and add to server

```bash
ssh-keygen -t ed25519 -C "github-deploy" -f ~/.ssh/github_deploy -N ""
ssh-copy-id -i ~/.ssh/github_deploy.pub YOUR_USERNAME@your-server.com
```

### 2. Add secrets to GitHub

Go to your repo **Settings** → **Secrets and variables** → **Actions** → **New repository secret**.

Add these 4 secrets:

| Secret | Description | Example |
|--------|-------------|---------|
| `SSH_PRIVATE_KEY` | Full private key file contents (including `-----BEGIN` and `-----END` lines) | `cat ~/.ssh/github_deploy` |
| `SSH_HOST` | Server hostname or IP | `ggj26.cheapmo.ch` |
| `SSH_USER` | SSH username on the server | `deploy` |
| `DEPLOY_PATH` | Absolute path where files are deployed | `/home/deploy/ggj26` |


## Deploying

- **Automatic**: Push to `main`
- **Manual**: Actions → Deploy → Run workflow

## Commands

```bash
sudo journalctl -u ggj-server -f      # View logs
sudo systemctl restart ggj-server     # Restart
sudo systemctl status ggj-server      # Status
```

## Troubleshooting

**Permission denied on deploy**: Check SSH key in `authorized_keys`, test with `ssh -i ~/.ssh/github_deploy user@server`

**Server won't start**: Check logs `sudo journalctl -u ggj-server -n 50`, try `cd ~/ggj26 && ./server`

**Sudoers issue**: Verify with `sudo -n systemctl restart ggj-server`
