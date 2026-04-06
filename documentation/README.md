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
