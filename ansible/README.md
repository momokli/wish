# Ansible Deployment for Wish

One-command deployment of the entire Wish backend.

## Targets

| Target               | Host            | Domain              | Description                     |
| -------------------- | --------------- | ------------------- | ------------------------------- |
| **projectmellon.de** | Hetzner VPS     | wish.zukkafabrik.de | Public-facing production server |
| **music**            | 192.168.178.200 | wish.simonklimke.de | LAN music host (home network)   |

---

## Usage — projectmellon.de (Hetzner)

```bash
# Install Ansible first
pip install ansible

# Deploy with secrets from environment
WISH_SPOTIFY_CLIENT_ID=your_spotify_client_id \
WISH_SPOTIFY_CLIENT_SECRET=your_spotify_client_secret \
WISH_DEEMIX_ARL=your_deezer_arl \
  ansible-playbook -i inventory.yml playbook.yml

# Redeploy just the binary (after code changes)
ansible-playbook -i inventory.yml playbook.yml --tags deploy

# Only update config
ansible-playbook -i inventory.yml playbook.yml --tags config
```

## What it sets up

| Component  | Port | Description                                               |
| ---------- | ---- | --------------------------------------------------------- |
| **wish**   | 8700 | Rust song request server (systemd)                        |
| **deemix** | 6595 | Deemix Docker container (320kbps MP3, Spotify plugin)     |
| **dufs**   | 8321 | Static file server for downloads (systemd)                |
| **Caddy**  | 443  | Reverse proxy (existing Docker container, routes updated) |

## Tags

- `deps` — system packages, yt-dlp, spotdl
- `deemix` — Docker container, config, ARL auth
- `wish` — Rust binary build, config, systemd
- `dufs` — file server
- `caddy` — reverse proxy routes
- `deploy` — redeploy source + rebuild binary
- `config` — update config files only
- `verify` — health check

---

## Usage — music host (LAN)

Deploy wish to the LAN music host at 192.168.178.200, accessible via
`wish.simonklimke.de` through the Caddy reverse proxy on the LAN host.

### Prerequisites

- `ssh music` must work (host 192.168.178.200, user `momo`)
- `ssh lan` must work (the Caddy Docker host on the LAN)
- Python 3 and pip available on the music host
- Caddy Docker container running on the LAN host (`caddy-caddy-1`)

### Full deploy

```bash
# Install Ansible first
pip install ansible

# Deploy wish + configure Caddy (requires secrets for Spotify)
WISH_SPOTIFY_CLIENT_ID=your_spotify_client_id \
WISH_SPOTIFY_CLIENT_SECRET=your_spotify_client_secret \
WISH_DEEMIX_ARL=your_deezer_arl \
  ansible-playbook -i inventory.yml playbook.yml --limit music,lan
```

### Deploy wish binary only (after code changes)

```bash
ansible-playbook -i inventory.yml playbook.yml --limit music --tags deploy
```

### Configure Caddy only

```bash
ansible-playbook -i inventory.yml playbook.yml --limit lan --tags caddy
```

### Dry run (check mode)

```bash
ansible-playbook -i inventory.yml playbook.yml --limit music,lan --check --diff
```

### What it sets up (LAN)

| Component | Host  | Port | Description                                            |
| --------- | ----- | ---- | ------------------------------------------------------ |
| **wish**  | music | 8700 | Rust song request server (systemd, user `momo`)        |
| **Caddy** | lan   | 443  | Reverse proxy: `wish.simonklimke.de \u2192 music:8700` |

### Infrastructure

```
internet -> fritz.box:443 -> lan host (Caddy Docker) -> 192.168.178.200:8700 (wish)
```

### Manual steps (one-time)

- Ensure `wish.simonklimke.de` DNS points to the Fritz!Box public IP
- Fritz!Box port forwarding: 443 -> LAN host
- `home_domains.txt` is managed by the playbook automatically
