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
          embeddingsBootstrapSpec {
            configPath
            profileName
          }
          summaryBootstrapSpec {
            action
            message
            modelName
            gatewayUrlOverride
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
          embeddingsBootstrapProgress {
            phase
            assetName
            bytesDownloaded
            bytesTotal
            version
            message
          }
          summaryBootstrapProgress {
            phase
            assetName
            bytesDownloaded
            bytesTotal
            version
            message
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
          embeddingsBootstrapResult {
            version
            binaryPath
            cacheDir
            runtimeName
            modelName
            freshlyInstalled
            message
          }
          summaryBootstrapResult {
            outcomeKind
            modelName
            message
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
        embeddingsBootstrapSpec {
          configPath
          profileName
        }
        summaryBootstrapSpec {
          action
          message
          modelName
          gatewayUrlOverride
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
        embeddingsBootstrapProgress {
          phase
          assetName
          bytesDownloaded
          bytesTotal
          version
          message
        }
        summaryBootstrapProgress {
          phase
          assetName
          bytesDownloaded
          bytesTotal
          version
          message
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
        embeddingsBootstrapResult {
          version
          binaryPath
          cacheDir
          runtimeName
          modelName
          freshlyInstalled
          message
        }
        summaryBootstrapResult {
          outcomeKind
          modelName
          message
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
        embeddingsBootstrapSpec {
          configPath
          profileName
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
        embeddingsBootstrapProgress {
          phase
          assetName
          bytesDownloaded
          bytesTotal
          version
          message
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
          embeddingsBootstrapSpec {
            configPath
            profileName
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
          embeddingsBootstrapProgress {
            phase
            assetName
            bytesDownloaded
            bytesTotal
            version
            message
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

pub(super) const START_INIT_MUTATION: &str = r#"
    mutation StartInit($repoId: String!, $input: StartInitInput!) {
      startInit(repoId: $repoId, input: $input) {
        initSessionId
      }
    }
"#;

pub(crate) const RUNTIME_SNAPSHOT_QUERY: &str = r#"
    query RuntimeSnapshot($repoId: String!) {
      runtimeSnapshot(repoId: $repoId) {
        repoId
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
            embeddingsBootstrapSpec {
              configPath
              profileName
            }
            summaryBootstrapSpec {
              action
              message
              modelName
              gatewayUrlOverride
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
            embeddingsBootstrapProgress {
              phase
              assetName
              bytesDownloaded
              bytesTotal
              version
              message
            }
            summaryBootstrapProgress {
              phase
              assetName
              bytesDownloaded
              bytesTotal
              version
              message
            }
          }
        }
        currentStateConsumer {
          persisted
          pendingRuns
          runningRuns
          failedRuns
          completedRecentRuns
          lastAction
          lastUpdatedUnix
          currentRepoRun {
            runId
            repoId
            capabilityId
            initSessionId
            consumerId
            handlerId
            fromGenerationSeq
            toGenerationSeq
            reconcileMode
            status
            attempts
            submittedAtUnix
            startedAtUnix
            updatedAtUnix
            completedAtUnix
            error
          }
        }
        workplane {
          pendingJobs
          runningJobs
          failedJobs
          completedRecentJobs
          pools {
            poolName
            displayName
            workerBudget
            activeWorkers
            pendingJobs
            runningJobs
            failedJobs
            completedRecentJobs
          }
          mailboxes {
            mailboxName
            displayName
            pendingJobs
            runningJobs
            failedJobs
            completedRecentJobs
            pendingCursorRuns
            runningCursorRuns
            failedCursorRuns
            completedRecentCursorRuns
            intentActive
            blockedReason
          }
        }
        blockedMailboxes {
          mailboxName
          displayName
          reason
        }
        embeddingsReadinessGate {
          blocked
          readiness
          reason
          activeTaskId
          profileName
          configPath
          lastError
          lastUpdatedUnix
        }
        summariesBootstrap {
          runId
          repoId
          initSessionId
          status
          request {
            action
            message
            modelName
            gatewayUrlOverride
          }
          progress {
            phase
            assetName
            bytesDownloaded
            bytesTotal
            version
            message
          }
          result {
            outcomeKind
            modelName
            message
          }
          error
          submittedAtUnix
          startedAtUnix
          updatedAtUnix
          completedAtUnix
        }
        currentInitSession {
          initSessionId
          status
          waitingReason
          warningSummary
          followUpSyncRequired
          runSync
          runIngest
          embeddingsSelected
          summariesSelected
          initialSyncTaskId
          ingestTaskId
          followUpSyncTaskId
          embeddingsBootstrapTaskId
          summaryBootstrapTaskId
          terminalError
          topPipelineLane {
            status
            waitingReason
            detail
            activityLabel
            taskId
            runId
            progress {
              completed
              inMemoryCompleted
              total
              remaining
            }
            queue {
              queued
              running
              failed
            }
            warnings {
              componentLabel
              message
              retryCommand
            }
            pendingCount
            runningCount
            failedCount
            completedCount
          }
          embeddingsLane {
            status
            waitingReason
            detail
            activityLabel
            taskId
            runId
            progress {
              completed
              inMemoryCompleted
              total
              remaining
            }
            queue {
              queued
              running
              failed
            }
            warnings {
              componentLabel
              message
              retryCommand
            }
            pendingCount
            runningCount
            failedCount
            completedCount
          }
          summariesLane {
            status
            waitingReason
            detail
            activityLabel
            taskId
            runId
            progress {
              completed
              inMemoryCompleted
              total
              remaining
            }
            queue {
              queued
              running
              failed
            }
            warnings {
              componentLabel
              message
              retryCommand
            }
            pendingCount
            runningCount
            failedCount
            completedCount
          }
        }
      }
    }
"#;

pub(super) const RUNTIME_EVENTS_SUBSCRIPTION: &str = r#"
    subscription RuntimeEvents($repoId: String!, $initSessionId: ID) {
      runtimeEvents(repoId: $repoId, initSessionId: $initSessionId) {
        domain
        repoId
        initSessionId
        updatedAtUnix
        taskId
        runId
        mailboxName
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
        embeddingsBootstrapSpec {
          configPath
          profileName
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
        embeddingsBootstrapProgress {
          phase
          assetName
          bytesDownloaded
          bytesTotal
          version
          message
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
            embeddingsBootstrapResult {
              version
              binaryPath
              cacheDir
              runtimeName
              modelName
              freshlyInstalled
              message
            }
            summaryBootstrapResult {
              outcomeKind
              modelName
              message
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
          embeddingsBootstrapSpec {
            configPath
            profileName
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
          embeddingsBootstrapProgress {
            phase
            assetName
            bytesDownloaded
            bytesTotal
            version
            message
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
          embeddingsBootstrapResult {
            version
            binaryPath
            cacheDir
            runtimeName
            modelName
            freshlyInstalled
            message
          }
        }
      }
    }
"#;
