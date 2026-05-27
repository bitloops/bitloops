# Bitloops Documentation

This directory contains the Bitloops Docusaurus app.

## Structure

- `docs/` contains the main user-facing product documentation
- `contributors/` contains contributor and architecture documentation
- `sidebars.ts` defines the main docs navigation
- `sidebarsContributors.ts` defines the contributors navigation

## Tooling

- Package manager: `pnpm`
- Node.js: `>=20`

Install dependencies from this directory with:

```bash
pnpm install
```

## Local Development

Start the docs app with hot reload:

```bash
pnpm start
```

## Build

Create the production build:

```bash
pnpm build
```

Serve the built site locally:

```bash
pnpm serve
```

## Deployment

The production docs are deployed to Google Cloud Run as the `bitloops-docs-v2`
service in the `www-main-192415` project.

Run the normal deployment from this directory with:

```bash
pnpm run deploy:gcp
```

This command runs two steps:

1. `pnpm run deploy:gcp:build` builds the Docker image with Google Cloud Build
   and pushes it to `gcr.io/www-main-192415/bitloops-docs-v2`.
2. `pnpm run deploy:gcp:run` deploys that image to Cloud Run in
   `europe-west1` with unauthenticated access enabled.

To run the steps separately:

```bash
pnpm run deploy:gcp:build
pnpm run deploy:gcp:run
```

To print the current Cloud Run service URL after deployment:

```bash
pnpm run deploy:gcp:url
```

The Docusaurus config uses `baseUrl: '/docs/'`, so production should route
`https://bitloops.com/docs/` to this Cloud Run service while preserving the
`/docs/` path.

## Maintenance

Type-check the Docusaurus app:

```bash
pnpm typecheck
```

Useful helper scripts:

```bash
pnpm clear
pnpm write-heading-ids
pnpm write-translations
```
