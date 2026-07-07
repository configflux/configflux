# Containerized ConfigFlux Pattern 1 consumer.
#
# The image carries the service code and a delivery bundle (the resolved
# snapshot + its ccm/ solver model — shipped together as one unit, the
# fail-closed rule from docs/service-integration-guide.md and ADR-0030 D2).
# Distribution here is the honest minimum: a `cp` into the build context (done
# by run.sh), then a COPY into the image. No backend, no agent, no MQTT.
#
# The build context is OUT_DIR/compose-build/, assembled by run.sh as:
#   compose-build/
#     app.py                                  # the service (this example)
#     bundles/<env>--<scope>/                 # one delivery bundle per service
#       resolve_result.<root>.<sel>.json
#       ccm/...
#
# Which bundle a container reads is chosen at runtime via CONFIGFLUX_SNAPSHOT
# (set per service in docker-compose.yml), so ONE image serves every scope.
FROM python:3.12-slim

WORKDIR /app

# The service is dependency-free stdlib Python (Pattern 1 reads JSON only), so
# there is nothing to pip install. Copy the app and the delivery bundles.
COPY app.py /app/app.py
COPY bundles /app/bundles

# Read-only consumer; no need to run as root.
RUN useradd --no-create-home --uid 10001 svc
USER svc

# CONFIGFLUX_SNAPSHOT (and optionally CONFIGFLUX_EXPECT_RESOLVE_HASH,
# CONFIGFLUX_ENV_LABEL) are supplied by docker-compose.yml per service.
ENTRYPOINT ["python3", "/app/app.py"]
