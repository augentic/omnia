# Docker Compose Resources

## Publishing to GitHub Container Registry


### Login

```bash
 docker login ghcr.io -u $GITHUB_USERNAME --password $GITHUB_TOKEN
```

### Publish

```bash
docker compose -f docker/otelcol.yaml publish ghcr.io/qwasr:/compose-otelcol:latest
docker compose -f docker/kafka.yaml publish --with-env ghcr.io/qwasr/compose-kafka:latest
docker compose -f docker/mongodb.yaml publish --with-env ghcr.io/qwasr/compose-mongodb:latest
docker compose -f docker/nats.yaml publish --with-env ghcr.io/qwasr/compose-nats:latest
docker compose -f docker/postgres.yaml publish --with-env ghcr.io/qwasr/compose-postgres:latest
docker compose -f docker/redis.yaml publish --with-env ghcr.io/qwasr/compose-redis:latest
```