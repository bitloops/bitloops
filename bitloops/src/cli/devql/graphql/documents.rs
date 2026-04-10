pub(super) const INIT_SCHEMA_MUTATION: &str = r#"
    mutation InitSchema {
      initSchema {
        success
        repoIdentity
        repoId
        relationalBackend
        eventsBackend
      }
    }
"#;

pub(super) const ENQUEUE_TASK_MUTATION: &str = r#"
    mutation EnqueueTask($input: EnqueueTaskInput!) {
      enqueueTask(input: $input) {
        merged
        task {
          taskId
          repoId
          repoName
          repoIdentity
          kind
          source
          status
          submittedAtUnix
          startedAtUnix
          updatedAtUnix
          completedAtUnix
          queuePosition
          tasksAhead
          error
          syncSpec {
            mode
            paths
          }
          ingestSpec {
            backfill
          }
          syncProgress {
            phase
            currentPath
            pathsTotal
            pathsCompleted
            pathsRemaining
            pathsUnchanged
            pathsAdded
            pathsChanged
            pathsRemoved
            cacheHits
            cacheMisses
            parseErrors
          }
          ingestProgress {
            phase
            commitsTotal
            commitsProcessed
            checkpointCompanionsProcessed
            currentCheckpointId
            currentCommitSha
            eventsInserted
            artefactsUpserted
          }
          syncResult {
            success
            mode
            parserVersion
            extractorVersion
            activeBranch
            headCommitSha
            headTreeSha
            pathsUnchanged
            pathsAdded
            pathsChanged
            pathsRemoved
            cacheHits
            cacheMisses
            parseErrors
            validation {
              valid
              expectedArtefacts
              actualArtefacts
              expectedEdges
              actualEdges
              missingArtefacts
              staleArtefacts
              mismatchedArtefacts
              missingEdges
              staleEdges
              mismatchedEdges
              filesWithDrift {
                path
                missingArtefacts
                staleArtefacts
                mismatchedArtefacts
                missingEdges
                staleEdges
                mismatchedEdges
              }
            }
          }
          ingestResult {
            success
            commitsProcessed
            checkpointCompanionsProcessed
            eventsInserted
            artefactsUpserted
            semanticFeatureRowsUpserted
            semanticFeatureRowsSkipped
            symbolEmbeddingRowsUpserted
            symbolEmbeddingRowsSkipped
            symbolCloneEdgesUpserted
            symbolCloneSourcesScored
          }
        }
      }
    }
"#;

pub(super) const TASK_QUERY: &str = r#"
    query Task($id: String!) {
      task(id: $id) {
        taskId
        repoId
        repoName
        repoIdentity
        kind
        source
        status
        submittedAtUnix
        startedAtUnix
        updatedAtUnix
        completedAtUnix
        queuePosition
        tasksAhead
        error
        syncSpec {
          mode
          paths
        }
        ingestSpec {
          backfill
        }
        syncProgress {
          phase
          currentPath
          pathsTotal
          pathsCompleted
          pathsRemaining
          pathsUnchanged
          pathsAdded
          pathsChanged
          pathsRemoved
          cacheHits
          cacheMisses
          parseErrors
        }
        ingestProgress {
          phase
          commitsTotal
          commitsProcessed
          checkpointCompanionsProcessed
          currentCheckpointId
          currentCommitSha
          eventsInserted
          artefactsUpserted
        }
        syncResult {
          success
          mode
          parserVersion
          extractorVersion
          activeBranch
          headCommitSha
          headTreeSha
          pathsUnchanged
          pathsAdded
          pathsChanged
          pathsRemoved
          cacheHits
          cacheMisses
          parseErrors
          validation {
            valid
            expectedArtefacts
            actualArtefacts
            expectedEdges
            actualEdges
            missingArtefacts
            staleArtefacts
            mismatchedArtefacts
            missingEdges
            staleEdges
            mismatchedEdges
            filesWithDrift {
              path
              missingArtefacts
              staleArtefacts
              mismatchedArtefacts
              missingEdges
              staleEdges
              mismatchedEdges
            }
          }
        }
        ingestResult {
          success
          commitsProcessed
          checkpointCompanionsProcessed
          eventsInserted
          artefactsUpserted
          semanticFeatureRowsUpserted
          semanticFeatureRowsSkipped
          symbolEmbeddingRowsUpserted
          symbolEmbeddingRowsSkipped
          symbolCloneEdgesUpserted
          symbolCloneSourcesScored
        }
      }
    }
"#;

pub(super) const TASKS_QUERY: &str = r#"
    query Tasks($kind: TaskKind, $status: TaskStatus, $limit: Int) {
      tasks(kind: $kind, status: $status, limit: $limit) {
        taskId
        repoId
        repoName
        repoIdentity
        kind
        source
        status
        submittedAtUnix
        startedAtUnix
        updatedAtUnix
        completedAtUnix
        queuePosition
        tasksAhead
        error
        syncSpec {
          mode
          paths
        }
        ingestSpec {
          backfill
        }
        syncProgress {
          phase
          currentPath
          pathsTotal
          pathsCompleted
          pathsRemaining
          pathsUnchanged
          pathsAdded
          pathsChanged
          pathsRemoved
          cacheHits
          cacheMisses
          parseErrors
        }
        ingestProgress {
          phase
          commitsTotal
          commitsProcessed
          checkpointCompanionsProcessed
          currentCheckpointId
          currentCommitSha
          eventsInserted
          artefactsUpserted
        }
        syncResult {
          success
          mode
        }
        ingestResult {
          success
          commitsProcessed
          eventsInserted
          artefactsUpserted
        }
      }
    }
"#;

pub(super) const TASK_QUEUE_QUERY: &str = r#"
    query TaskQueue {
      taskQueue {
        persisted
        queuedTasks
        runningTasks
        failedTasks
        completedRecentTasks
        byKind {
          kind
          queuedTasks
          runningTasks
          failedTasks
          completedRecentTasks
        }
        paused
        pausedReason
        lastAction
        lastUpdatedUnix
        currentRepoTasks {
          taskId
          repoId
          repoName
          repoIdentity
          kind
          source
          status
          submittedAtUnix
          startedAtUnix
          updatedAtUnix
          completedAtUnix
          queuePosition
          tasksAhead
          error
          syncSpec {
            mode
            paths
          }
          ingestSpec {
            backfill
          }
          syncProgress {
            phase
            currentPath
            pathsTotal
            pathsCompleted
            pathsRemaining
            pathsUnchanged
            pathsAdded
            pathsChanged
            pathsRemoved
            cacheHits
            cacheMisses
            parseErrors
          }
          ingestProgress {
            phase
            commitsTotal
            commitsProcessed
            checkpointCompanionsProcessed
            currentCheckpointId
            currentCommitSha
            eventsInserted
            artefactsUpserted
          }
          syncResult {
            success
            mode
          }
          ingestResult {
            success
            commitsProcessed
            eventsInserted
            artefactsUpserted
          }
        }
      }
    }
"#;

pub(super) const PAUSE_TASK_QUEUE_MUTATION: &str = r#"
    mutation PauseTaskQueue($reason: String) {
      pauseTaskQueue(reason: $reason) {
        message
        repoId
        paused
        pausedReason
        updatedAtUnix
      }
    }
"#;

pub(super) const RESUME_TASK_QUEUE_MUTATION: &str = r#"
    mutation ResumeTaskQueue($repoId: String) {
      resumeTaskQueue(repoId: $repoId) {
        message
        repoId
        paused
        pausedReason
        updatedAtUnix
      }
    }
"#;

pub(super) const CANCEL_TASK_MUTATION: &str = r#"
    mutation CancelTask($id: String!) {
      cancelTask(id: $id) {
        taskId
        repoId
        repoName
        repoIdentity
        kind
        source
        status
        submittedAtUnix
        startedAtUnix
        updatedAtUnix
        completedAtUnix
        queuePosition
        tasksAhead
        error
        syncSpec {
          mode
          paths
        }
        ingestSpec {
          backfill
        }
        syncProgress {
          phase
          currentPath
          pathsTotal
          pathsCompleted
          pathsRemaining
          pathsUnchanged
          pathsAdded
          pathsChanged
          pathsRemoved
          cacheHits
          cacheMisses
          parseErrors
        }
        ingestProgress {
          phase
          commitsTotal
          commitsProcessed
          checkpointCompanionsProcessed
          currentCheckpointId
          currentCommitSha
          eventsInserted
          artefactsUpserted
        }
        syncResult {
          success
          mode
        }
        ingestResult {
          success
          commitsProcessed
          eventsInserted
          artefactsUpserted
        }
      }
    }
"#;

pub(super) const TASK_PROGRESS_SUBSCRIPTION: &str = r#"
    subscription TaskProgress($taskId: String!) {
      taskProgress(taskId: $taskId) {
        task {
          taskId
          repoId
          repoName
          repoIdentity
          kind
          source
          status
          submittedAtUnix
          startedAtUnix
          updatedAtUnix
          completedAtUnix
          queuePosition
          tasksAhead
          error
          syncSpec {
            mode
            paths
          }
          ingestSpec {
            backfill
          }
          syncProgress {
            phase
            currentPath
            pathsTotal
            pathsCompleted
            pathsRemaining
            pathsUnchanged
            pathsAdded
            pathsChanged
            pathsRemoved
            cacheHits
            cacheMisses
            parseErrors
          }
          ingestProgress {
            phase
            commitsTotal
            commitsProcessed
            checkpointCompanionsProcessed
            currentCheckpointId
            currentCommitSha
            eventsInserted
            artefactsUpserted
          }
          syncResult {
            success
            mode
            parserVersion
            extractorVersion
            activeBranch
            headCommitSha
            headTreeSha
            pathsUnchanged
            pathsAdded
            pathsChanged
            pathsRemoved
            cacheHits
            cacheMisses
            parseErrors
            validation {
              valid
              expectedArtefacts
              actualArtefacts
              expectedEdges
              actualEdges
              missingArtefacts
              staleArtefacts
              mismatchedArtefacts
              missingEdges
              staleEdges
              mismatchedEdges
              filesWithDrift {
                path
                missingArtefacts
                staleArtefacts
                mismatchedArtefacts
                missingEdges
                staleEdges
                mismatchedEdges
              }
            }
          }
          ingestResult {
            success
            commitsProcessed
            checkpointCompanionsProcessed
            eventsInserted
            artefactsUpserted
            semanticFeatureRowsUpserted
            semanticFeatureRowsSkipped
            symbolEmbeddingRowsUpserted
            symbolEmbeddingRowsSkipped
            symbolCloneEdgesUpserted
            symbolCloneSourcesScored
          }
        }
      }
    }
"#;
