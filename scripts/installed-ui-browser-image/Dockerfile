# Disposable browser image for the installed-UI smoke. The base is the
# linux/amd64 Playwright 1.62.0 Noble image used by e2e/playwright.
FROM mcr.microsoft.com/playwright@sha256:02bbb2155cd7109e3e9c741941097ed1608cf8b6fa44ee2595896da2bdc1f471

USER root
RUN apt-get update \
    && apt-get install --no-install-recommends --yes ca-certificates libnss3-tools \
    && rm -rf /var/lib/apt/lists/*

# The runner imports the disposable Caddy CA into this user's NSS profile.
# Use the image's pwuser account so Chromium does not run as root.
USER pwuser
