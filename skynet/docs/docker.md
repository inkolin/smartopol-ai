# Docker Deployment

SmartopolAI provides Docker support for containerized deployments. The Docker setup uses a multi-stage build with dependency caching for fast rebuilds.

---

## Quick Start

```bash
./docker-setup.sh
```

This script:
1. Checks for Docker and Docker Compose
2. Generates a `.env` file with a random auth token
3. Builds the Docker image
4. Starts the container
5. Verifies the health endpoint

After running, edit `.env` to add your LLM provider API key.

---

## Manual Setup

```bash
# 1. Create .env from template
cp .env.example .env   # or let docker-setup.sh generate it
# Edit .env — set ANTHROPIC_API_KEY or OPENAI_API_KEY

# 2. Build and start
docker compose up -d

# 3. Verify
curl http://localhost:18789/health
```

---

## Configuration

All configuration is done via environment variables in `.env`. The gateway reads `SKYNET_*` env vars via [figment](https://docs.rs/figment/).

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `SKYNET_AUTH_TOKEN` | API auth token (required) | — |
| `SKYNET_PORT` | Host port mapping | `18789` |
| `ANTHROPIC_API_KEY` | Anthropic API key | — |
| `OPENAI_API_KEY` | OpenAI API key | — |
| `SKYNET_MODEL` | LLM model name | `claude-sonnet-4-6` |
| `DISCORD_BOT_TOKEN` | Discord bot token | — |

### Bind-mount Config (Advanced)

For complex configurations that don't map to env vars, bind-mount a `skynet.toml`:

```yaml
# In docker-compose.yml, uncomment:
volumes:
  - ./skynet.toml:/home/skynet/.skynet/skynet.toml:ro
```

---

## Local LLM (Ollama)

To use Ollama running on the host machine:

1. In `docker-compose.yml`, uncomment:
   ```yaml
   extra_hosts:
     - "host.docker.internal:host-gateway"
   ```

2. In `.env`, add:
   ```
   SKYNET_PROVIDERS_OLLAMA_BASE_URL=http://host.docker.internal:11434
   ```

3. Restart: `docker compose up -d`

---

## Discord Bot in Docker

Set the bot token in `.env`:

```
DISCORD_BOT_TOKEN=your-bot-token-here
```

The Discord adapter starts automatically when the token is present. No additional ports or configuration needed.

---

## Production Deployment

### Using the Release Target

The release target uses Google's distroless image — no shell, no package manager, minimal attack surface:

```bash
# Build release image
docker build --target release -t smartopol-ai:release .

# Run
docker run -d --name skynet \
  -p 18789:18789 \
  -e SKYNET_GATEWAY_AUTH_TOKEN=your-token \
  -e SKYNET_PROVIDERS_ANTHROPIC_API_KEY=sk-ant-... \
  -v skynet-data:/home/nonroot/.skynet \
  smartopol-ai:release
```

Or change the target in `docker-compose.yml`:

```yaml
build:
  context: .
  target: release
```

### Reverse Proxy (nginx)

```nginx
server {
    listen 443 ssl;
    server_name ai.example.com;

    ssl_certificate     /etc/letsencrypt/live/ai.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/ai.example.com/privkey.pem;

    location / {
        proxy_pass http://127.0.0.1:18789;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
    }
}
```

---

## Updating

```bash
git pull
docker compose build
docker compose up -d
```

The named volume `skynet-data` preserves your SQLite database, knowledge base, and sessions across rebuilds.

---

## Data Persistence

The `skynet-data` named volume stores:
- `skynet.db` — SQLite database (memory, sessions, knowledge, scheduler jobs)
- `SOUL.md` — agent personality
- `tools/` — installed plugins
- `skills/` — skill documents
- `knowledge/` — seed knowledge files

Data survives `docker compose down` and image rebuilds. To fully reset:

```bash
docker compose down -v   # -v removes volumes
```

---

## Troubleshooting

### Container exits immediately
Check logs: `docker compose logs gateway`

Common causes:
- Missing API key (warning, not fatal — gateway starts but chat.send returns errors)
- Port already in use — change `SKYNET_PORT` in `.env`

### Cannot connect to gateway
The container binds to `0.0.0.0` inside Docker (hardcoded in Dockerfile). If you override this via env var, make sure it's not `127.0.0.1`.

### Permission denied on volume
The container runs as uid 10001 (`skynet`). If you bind-mount a host directory, ensure it's readable:
```bash
chmod 755 /path/to/your/data
```

### Build takes too long
First build compiles all dependencies (~5-10 min). Subsequent builds use cached layers and only recompile changed source code (~30s).
