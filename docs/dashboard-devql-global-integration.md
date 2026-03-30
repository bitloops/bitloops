# Dashboard DevQL Global Integration

## Summary

The Bitloops daemon now exposes two DevQL GraphQL surfaces:

- `/devql`
  - Slim, CLI-scoped schema for agent clients.
  - No explicit `repo(...)` or `project(...)` wrappers in the SDL.
  - Requires Bitloops CLI scope headers on each request.
- `/devql/global`
  - Full explicit schema for dashboard and direct GraphQL clients.
  - Explicit `repo(name:)`, `branch(name:)`, `project(path:)`, and `asOf(input:)`.

The dashboard should integrate with `/devql/global`.

## Endpoints

- HTTP GraphQL: `/devql/global`
- Playground: `/devql/global/playground`
- SDL: `/devql/global/sdl`
- WebSocket subscriptions: `/devql/global/ws`

## Required Query Shape

Dashboard queries must select an explicit repository first:

```graphql
query DashboardCommits {
  repo(name: "demo") {
    defaultBranch
    commits(first: 20) {
      totalCount
      edges {
        node {
          sha
          commitMessage
          branch
        }
      }
    }
  }
}
```

For project-scoped reads, add `project(path:)` explicitly:

```graphql
query DashboardProjectArtefacts {
  repo(name: "demo") {
    project(path: "packages/api") {
      artefacts(first: 50) {
        totalCount
      }
    }
  }
}
```

## Branch Selection

Use `branch(name:)` for live current-state reads on a non-default branch:

```graphql
query DashboardFeatureBranch {
  repo(name: "demo") {
    branch(name: "feature/refactor") {
      files(path: "src") {
        path
      }
    }
  }
}
```

Notes:

- `branch(name:)` is for live branch selection.
- If no branch is selected, repo-level live reads fall back to the repository default branch.
- `asOf(input:)` remains temporal and historical only. Do not use it for live branch switching.

## Repo Checkout Registry

The daemon keeps a persistent repo-path registry keyed by `repo_id`.

How it is populated:

- `bitloops devql ...` discovers the current checkout and sends repo scope headers to `/devql`.
- The daemon stores or refreshes the last known checkout path for that repository.

Why it matters:

- `/devql/global` can resolve git-backed fields such as commits, branches, and checkpoint fallback reads only when the daemon knows the checkout path for the selected repo.

Expected error:

- If a repo exists in storage but has no known checkout path, git-backed queries will fail with:
  - `repo checkout unknown for '<repo>'`

Operational guidance:

- If this happens, run a slim CLI command from the relevant checkout first, for example:
  - `bitloops devql query 'commits()'`
  - `bitloops devql ingest`

## Mutations

Repo-scoped write mutations are not available on `/devql/global`.

These include:

- `ingest`
- `addKnowledge`
- `associateKnowledge`
- `refreshKnowledge`

Expected behaviour:

- `/devql/global` returns a validation error explaining that repo-scoped DevQL mutations require CLI repository scope.

Allowed globally:

- `initSchema`
- `applyMigrations`

## Dashboard Client Guidance

- Always query `/devql/global`.
- Always require an explicit repository selection in dashboard state.
- Add explicit project selection only when the UI is operating on a subtree.
- Add explicit branch selection when the UI is showing live data for a non-default branch.
- Treat `asOf(input:)` as a historical mode toggle, not a branch selector.
- Handle the “repo checkout unknown” error with a user-facing explanation and a remediation hint.

## Example Errors To Handle

- Unknown repository:
  - `unknown repository '<name>'`
- Ambiguous repository name:
  - `repository name '<name>' is ambiguous; use the repo id or identity instead`
- Missing checkout path:
  - `repo checkout unknown for '<repo>'; re-run a slim CLI query or ingest from that checkout to register its path`
- Repo-scoped mutation on global schema:
  - `repo-scoped DevQL mutations require CLI repository scope; use bitloops devql ... against /devql`

## SDL Contract

The dashboard should use `/devql/global/sdl` as the source of truth for tooling and generated clients.

Do not use the slim SDL from `/devql/sdl` in the dashboard, because that surface intentionally omits explicit repo and project selection.
