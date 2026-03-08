# Rust Reference: Claude + Git Hooks Flow

This document captures the **Rust reference** hook lifecycle for Claude sessions and git commits.

- Claude metadata/storage names in Rust reference: `.bitloops/...`
- Temporary branch in Rust reference: `bitloops/<base-sha[:7]>[-worktree]`
- Committed metadata branch in Rust reference: `bitloops/checkpoints/v1`

## 1) End-to-End Flow (Claude + Git)

```mermaid
flowchart TD
    U["User"] --> C["Claude Code"]

    C --> H1["SessionStart hook"]
    H1 --> CS["Write .bitloops/current_session"]

    C --> H2["UserPromptSubmit hook"]
    H2 --> PP["Write .bitloops/tmp/pre-prompt-<session>.json"]
    H2 --> SI["Initialize session state + shadow branch"]

    C --> H3["Stop hook"]
    H3 --> MD["Write .bitloops/metadata/<session>/ full.jsonl, prompt.txt, summary.txt, context.md"]
    H3 --> SS["SaveChanges to shadow branch bitloops/<base-sha[:7]>[-worktree]"]

    U --> GIT["git commit"]
    GIT --> G1["prepare-commit-msg"]
    G1 --> G2["commit-msg"]
    G2 --> G3["post-commit"]

    G3 -->|"if trailer exists"| CP["Condense shadow data to bitloops/checkpoints/v1"]
    G3 -->|"if no trailer"| UB["Update active session base commit"]

    U --> GP["git push"]
    GP --> G4["pre-push"]
    G4 --> PS["Push bitloops/checkpoints/v1 (non-blocking)"]
```

## 2) Claude Hook Sequence

```mermaid
sequenceDiagram
    participant U as User
    participant C as Claude
    participant HC as "bitloops hooks claude-code"
    participant GS as Git/Status
    participant ST as Session State
    participant SB as Shadow Branch
    participant MD as .bitloops/metadata

    C->>HC: SessionStart
    HC->>ST: Persist date-prefixed session id
    HC->>ST: Write .bitloops/current_session

    U->>C: Submit prompt
    C->>HC: UserPromptSubmit
    HC->>HC: Check concurrent session overlap
    HC->>GS: Collect untracked files baseline
    HC->>ST: Save pre-prompt file (.bitloops/tmp/pre-prompt-...)
    HC->>ST: InitializeSession (manual-commit)
    HC->>SB: Create/validate bitloops/<base-sha[:7]>[-worktree]

    C->>C: Generate code / tool calls
    C->>HC: Stop
    HC->>MD: Write full.jsonl, prompt.txt, summary.txt, context.md
    HC->>GS: Compute modified/new/deleted files
    HC->>SB: SaveChanges (checkpoint snapshot commit)
    HC->>ST: Update session counters/state
    HC->>ST: Delete pre-prompt temp file

    rect rgb(240,240,240)
    Note over C,HC: Subagent path
    C->>HC: PreToolUse[Task]
    HC->>ST: Write .bitloops/tmp/pre-task-<tool-use-id>.json

    C->>HC: PostToolUse[TodoWrite] (optional, repeated)
    HC->>GS: Detect file changes
    HC->>SB: SaveTaskCheckpoint incremental if changed

    C->>HC: PostToolUse[Task]
    HC->>GS: Resolve agent transcript + changes
    HC->>SB: SaveTaskCheckpoint final
    HC->>ST: Delete pre-task temp file
    end
```

## 3) Git Hook Decision Flow (Manual-Commit)

```mermaid
flowchart TD
    A["git commit starts"] --> B["prepare-commit-msg"]

    B --> C{"git sequence operation? rebase/cherry-pick/revert"}
    C -->|"yes"| C0["No-op"]
    C -->|"no"| D{"source is merge/squash?"}
    D -->|"yes"| D0["No-op"]
    D -->|"no"| E{"source == commit (amend)?"}

    E -->|"yes"| E1["Preserve existing trailer or restore LastCheckpointID"]
    E -->|"no"| F["Find sessions for current worktree"]

    F --> G{"Any relevant session content?"}
    G -->|"no"| G0["No trailer added"]
    G -->|"yes"| H["Generate checkpoint id Add bitloops-Checkpoint trailer"]

    H --> I["commit-msg"]
    G0 --> I
    E1 --> I
    C0 --> I
    D0 --> I

    I --> J{"Message has only trailer (no user content)?"}
    J -->|"yes"| J1["Strip trailer"]
    J -->|"no"| K["Keep message"]
    J1 --> L["post-commit"]
    K --> L

    L --> M{"HEAD commit has bitloops-Checkpoint trailer?"}
    M -->|"no"| M1["Update base_commit for active sessions"]
    M -->|"yes"| N["Condense session data"]

    N --> O["Write committed metadata tree to bitloops/checkpoints/v1"]
    O --> P["Reset/carry-forward session state"]

    Q["git push"] --> R["pre-push"]
    R --> S{"bitloops/checkpoints/v1 exists locally?"}
    S -->|"no"| S0["No-op"]
    S -->|"yes"| T["Push sessions branch (non-blocking) --no-verify, retry via sync/merge path"]
```

## 4) Session Phase State Machine

```mermaid
stateDiagram-v2
    [*] --> IDLE: SessionStart

    IDLE --> ACTIVE: TurnStart (UserPromptSubmit)
    ACTIVE --> IDLE: TurnEnd (Stop)

    ACTIVE --> ACTIVE: GitCommit / Condense
    IDLE --> IDLE: GitCommit / Condense

    IDLE --> ENDED: SessionStop
    ACTIVE --> ENDED: SessionStop

    ENDED --> ACTIVE: TurnStart (session resume)
    ENDED --> ENDED: GitCommit / CondenseIfFilesTouched
```
