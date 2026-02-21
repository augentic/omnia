# Docker Compose Resources

## Publishing to GitHub Container Registry


### Login

```bash
 docker login ghcr.io -u $GITHUB_USERNAME --password $GITHUB_TOKEN
```

### Publish

```bash
docker compose -f docker/otelcol.yaml publish ghcr.io/omnia:/compose-otelcol:latest
docker compose -f docker/kafka.yaml publish --with-env ghcr.io/omnia/compose-kafka:latest
docker compose -f docker/mongodb.yaml publish --with-env ghcr.io/omnia/compose-mongodb:latest
docker compose -f docker/nats.yaml publish --with-env ghcr.io/omnia/compose-nats:latest
docker compose -f docker/postgres.yaml publish --with-env ghcr.io/omnia/compose-postgres:latest
docker compose -f docker/redis.yaml publish --with-env ghcr.io/omnia/compose-redis:latest
```